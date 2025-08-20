use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;
use std::{collections::HashMap, sync::Mutex};

use super::{
    StreamBuffer,
    traits::{DataSource, DataSourceMetadata, DataSourceStatus, Searchable, StreamEvent},
};
use crate::context::AgentHandle;
use crate::error::Result;
use crate::memory::MemoryBlock;
use crate::utils::format_duration;
use async_trait::async_trait;
use atrium_api::app::bsky::feed::defs::PostViewEmbedRefs;
use atrium_api::app::bsky::feed::post::{RecordEmbedRefs, RecordLabelsRefs, ReplyRefData};
use atrium_api::app::bsky::richtext::facet::MainFeaturesItem;
use atrium_api::com::atproto::repo::strong_ref::MainData;
use atrium_api::types::string::{Cid, Did};
use atrium_api::types::{TryFromUnknown, Union};
use atrium_identity::did::{CommonDidResolver, CommonDidResolverConfig, DEFAULT_PLC_DIRECTORY_URL};
use compact_str::CompactString;

use chrono::{DateTime, Utc};
use futures::Stream;
use rocketman::{
    connection::JetstreamConnection,
    handler,
    ingestion::LexiconIngestor,
    options::JetstreamOptions,
    types::event::{Commit, Event},
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio_stream::wrappers::ReceiverStream;

// Constellation API types
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConstellationLinksResponse {
    pub total: usize,
    pub linking_records: Vec<ConstellationRecord>,
    pub cursor: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConstellationRecord {
    pub did: String,
    pub collection: String,
    pub rkey: String,
}

impl ConstellationRecord {
    /// Convert to AT URI format
    pub fn to_at_uri(&self) -> String {
        format!("at://{}/{}/{}", self.did, self.collection, self.rkey)
    }
}

/// Thread context with siblings and their replies
#[derive(Debug, Clone)]
pub struct ThreadContext {
    /// The parent post
    pub parent: Option<atrium_api::app::bsky::feed::defs::PostView>,
    /// Direct siblings (other replies to the same parent)
    pub siblings: Vec<atrium_api::app::bsky::feed::defs::PostView>,
    /// Map of post URI to its direct replies
    pub replies_map:
        std::collections::HashMap<String, Vec<atrium_api::app::bsky::feed::defs::PostView>>,
    /// Engagement metrics for posts (likes, replies, reposts)
    pub engagement_map: std::collections::HashMap<String, PostEngagement>,
    /// Agent's interactions with posts
    pub agent_interactions: std::collections::HashMap<String, AgentInteraction>,
}

/// Post engagement metrics
#[derive(Debug, Clone, Default)]
pub struct PostEngagement {
    pub like_count: u32,
    pub reply_count: u32,
    pub repost_count: u32,
}

/// Agent's interaction with a post
#[derive(Debug, Clone, Default)]
pub struct AgentInteraction {
    pub liked: bool,
    pub replied: bool,
    pub reposted: bool,
}

#[derive(Debug, Clone)]
pub struct PatternHttpClient {
    pub client: reqwest::Client,
}

impl PatternHttpClient {
    /// Check if a DID should be included based on filter rules
    fn should_fetch_did(did: &str, agent_did: Option<&str>, filter: &BlueskyFilter) -> bool {
        // Always fetch agent's own posts
        if let Some(agent) = agent_did {
            if did == agent {
                return true;
            }
        }

        // Never fetch excluded DIDs
        if filter.exclude_dids.contains(&did.to_string()) {
            return false;
        }

        // Always fetch friends
        if filter.friends.contains(&did.to_string()) {
            return true;
        }

        // If we have an allowlist, only fetch those
        if !filter.dids.is_empty() {
            return filter.dids.contains(&did.to_string());
        }

        // Otherwise allow
        true
    }

    /// Filter constellation records based on DIDs we want to fetch
    fn filter_constellation_records(
        records: Vec<ConstellationRecord>,
        agent_did: Option<&str>,
        filter: &BlueskyFilter,
        current_post_uri: &str,
    ) -> Vec<ConstellationRecord> {
        records
            .into_iter()
            .filter(|record| {
                let uri = record.to_at_uri();
                // Skip the current post
                if uri == current_post_uri {
                    return false;
                }
                // Check if we should fetch this DID
                Self::should_fetch_did(&record.did, agent_did, filter)
            })
            .collect()
    }
    /// Fetch posts that reply to the same parent from constellation service
    pub async fn fetch_thread_siblings(
        &self,
        parent_uri: &str,
    ) -> Result<Vec<ConstellationRecord>> {
        let encoded_uri = urlencoding::encode(parent_uri);
        let url = format!(
            "https://constellation.microcosm.blue/links?target={}&collection=app.bsky.feed.post&path=.reply.parent.uri",
            encoded_uri
        );

        let response =
            self.client
                .get(&url)
                .send()
                .await
                .map_err(|e| crate::CoreError::DataSourceError {
                    source_name: "constellation".to_string(),
                    operation: "fetch_thread_siblings".to_string(),
                    cause: e.to_string(),
                })?;

        if !response.status().is_success() {
            return Err(crate::CoreError::DataSourceError {
                source_name: "constellation".to_string(),
                operation: "fetch_thread_siblings".to_string(),
                cause: format!(
                    "HTTP {}: {}",
                    response.status(),
                    response.text().await.unwrap_or_default()
                ),
            });
        }

        let links_response: ConstellationLinksResponse =
            response
                .json()
                .await
                .map_err(|e| crate::CoreError::DataSourceError {
                    source_name: "constellation".to_string(),
                    operation: "parse_response".to_string(),
                    cause: e.to_string(),
                })?;

        Ok(links_response.linking_records)
    }

    /// Build a comprehensive thread context with siblings and their replies
    pub async fn build_thread_context(
        &self,
        post_uri: &str,
        parent_uri: Option<&str>,
        bsky_agent: &Arc<bsky_sdk::BskyAgent>,
        agent_did: Option<&str>,
        filter: &BlueskyFilter,
        max_depth: usize,
    ) -> Result<ThreadContext> {
        let mut context = ThreadContext {
            parent: None,
            siblings: Vec::new(),
            replies_map: std::collections::HashMap::new(),
            engagement_map: std::collections::HashMap::new(),
            agent_interactions: std::collections::HashMap::new(),
        };

        // If we have a parent URI, fetch siblings
        if let Some(parent) = parent_uri {
            // First, fetch the parent post itself
            let parent_params = atrium_api::app::bsky::feed::get_posts::ParametersData {
                uris: vec![parent.to_string()],
            };
            if let Ok(parent_result) = bsky_agent
                .api
                .app
                .bsky
                .feed
                .get_posts(parent_params.into())
                .await
            {
                context.parent = parent_result.posts.clone().into_iter().next();

                // Extract engagement metrics from parent
                if let Some(parent_post) = &context.parent {
                    context.engagement_map.insert(
                        parent_post.uri.clone(),
                        PostEngagement {
                            like_count: parent_post.like_count.unwrap_or(0) as u32,
                            reply_count: parent_post.reply_count.unwrap_or(0) as u32,
                            repost_count: parent_post.repost_count.unwrap_or(0) as u32,
                        },
                    );
                }
            }

            // Fetch all siblings
            let sibling_records = self.fetch_thread_siblings(parent).await?;

            // Filter siblings based on our criteria
            let filtered_records =
                Self::filter_constellation_records(sibling_records, agent_did, filter, post_uri);

            let sibling_uris: Vec<String> = filtered_records
                .into_iter()
                .map(|record| record.to_at_uri())
                .collect();

            if !sibling_uris.is_empty() {
                let params = atrium_api::app::bsky::feed::get_posts::ParametersData {
                    uris: sibling_uris.clone(),
                };
                if let Ok(posts_result) =
                    bsky_agent.api.app.bsky.feed.get_posts(params.into()).await
                {
                    context.siblings = posts_result.posts.clone();

                    // Collect engagement metrics for siblings
                    for sibling in &context.siblings {
                        context.engagement_map.insert(
                            sibling.uri.clone(),
                            PostEngagement {
                                like_count: sibling.like_count.unwrap_or(0) as u32,
                                reply_count: sibling.reply_count.unwrap_or(0) as u32,
                                repost_count: sibling.repost_count.unwrap_or(0) as u32,
                            },
                        );
                    }
                }

                // If depth > 0, fetch replies to each sibling recursively
                if max_depth > 0 {
                    // Prioritize fetching replies to agent's posts
                    let mut priority_siblings = Vec::new();
                    let mut regular_siblings = Vec::new();

                    for sibling in &context.siblings {
                        if let Some(agent) = agent_did {
                            if sibling.author.did.as_str() == agent {
                                priority_siblings.push(sibling.uri.clone());
                                continue;
                            }
                        }

                        // Only fetch replies if the sibling has some
                        if let Some(engagement) = context.engagement_map.get(&sibling.uri) {
                            if engagement.reply_count > 0 {
                                regular_siblings.push(sibling.uri.clone());
                            }
                        }
                    }

                    // Fetch replies recursively for priority siblings first, then regular ones
                    for sibling_uri in priority_siblings
                        .iter()
                        .chain(regular_siblings.iter())
                        .take(5)
                    {
                        self.fetch_replies_recursive(
                            &mut context,
                            sibling_uri,
                            bsky_agent,
                            agent_did,
                            filter,
                            max_depth,
                            1, // current depth
                        )
                        .await;
                    }
                }
            }
        }

        Ok(context)
    }

    /// Recursively fetch replies to a post up to max_depth levels
    async fn fetch_replies_recursive(
        &self,
        context: &mut ThreadContext,
        parent_uri: &str,
        bsky_agent: &Arc<bsky_sdk::BskyAgent>,
        agent_did: Option<&str>,
        filter: &BlueskyFilter,
        max_depth: usize,
        current_depth: usize,
    ) {
        if current_depth > max_depth {
            return;
        }

        let reply_records = self
            .fetch_thread_siblings(parent_uri)
            .await
            .unwrap_or_default();

        if reply_records.is_empty() {
            return;
        }

        // Filter reply records
        let filtered_replies =
            Self::filter_constellation_records(reply_records, agent_did, filter, parent_uri);

        let reply_uris: Vec<String> = filtered_replies
            .into_iter()
            .map(|record| record.to_at_uri())
            .collect();

        if reply_uris.is_empty() {
            return;
        }

        let params = atrium_api::app::bsky::feed::get_posts::ParametersData {
            uris: reply_uris.clone(),
        };

        if let Ok(replies_result) = bsky_agent.api.app.bsky.feed.get_posts(params.into()).await {
            let replies = replies_result.posts.clone();

            // Collect engagement metrics for replies
            for reply in &replies {
                context.engagement_map.insert(
                    reply.uri.clone(),
                    PostEngagement {
                        like_count: reply.like_count.unwrap_or(0) as u32,
                        reply_count: reply.reply_count.unwrap_or(0) as u32,
                        repost_count: reply.repost_count.unwrap_or(0) as u32,
                    },
                );
            }

            // Store replies for this parent
            context
                .replies_map
                .insert(parent_uri.to_string(), replies.clone());

            // If we haven't reached max depth, recursively fetch replies to these replies
            if current_depth < max_depth {
                for reply in &replies {
                    // Only recurse if this reply has replies and we're prioritizing agent posts
                    // or limiting to avoid too many API calls
                    if let Some(engagement) = context.engagement_map.get(&reply.uri) {
                        if engagement.reply_count > 0 {
                            // Prioritize agent posts for deeper recursion
                            let should_recurse = if let Some(agent) = agent_did {
                                reply.author.did.as_str() == agent || current_depth <= 2
                            } else {
                                current_depth <= 2 // Limit depth for non-agent cases
                            };

                            if should_recurse {
                                Box::pin(self.fetch_replies_recursive(
                                    context,
                                    &reply.uri,
                                    bsky_agent,
                                    agent_did,
                                    filter,
                                    max_depth,
                                    current_depth + 1,
                                ))
                                .await;
                            }
                        }
                    }
                }
            }
        }
    }
}

impl atrium_xrpc::HttpClient for PatternHttpClient {
    async fn send_http(
        &self,
        request: atrium_xrpc::http::Request<Vec<u8>>,
    ) -> core::result::Result<
        atrium_xrpc::http::Response<Vec<u8>>,
        Box<dyn std::error::Error + Send + Sync + 'static>,
    > {
        let response = self.client.execute(request.try_into()?).await?;
        let mut builder = atrium_xrpc::http::Response::builder().status(response.status());
        for (k, v) in response.headers() {
            builder = builder.header(k, v);
        }
        builder
            .body(response.bytes().await?.to_vec())
            .map_err(Into::into)
    }
}

impl Default for PatternHttpClient {
    fn default() -> Self {
        Self {
            client: reqwest::Client::builder()
                .user_agent(concat!("pattern/", env!("CARGO_PKG_VERSION")))
                .build()
                .unwrap(), // panics for the same reasons Client::new() would: https://docs.rs/reqwest/latest/reqwest/struct.Client.html#panics
        }
    }
}

pub fn atproto_identity_resolver() -> CommonDidResolver<PatternHttpClient> {
    CommonDidResolver::new(CommonDidResolverConfig {
        plc_directory_url: DEFAULT_PLC_DIRECTORY_URL.to_string(),
        http_client: Arc::new(PatternHttpClient::default()),
    })
}

/// A post from Bluesky/ATProto
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlueskyPost {
    pub uri: String,
    pub did: String, // Author DID
    pub cid: Cid,
    pub handle: String,               // Author handle
    pub display_name: Option<String>, // Author display name (fetched later)
    pub text: String,
    pub created_at: DateTime<Utc>,
    pub reply: Option<ReplyRef>, // Full reply reference with root and parent
    pub embed: Option<Union<atrium_api::app::bsky::feed::post::RecordEmbedRefs>>,
    pub langs: Vec<String>,
    pub labels: Vec<String>,
    pub facets: Vec<Facet>, // Rich text annotations (mentions, links, hashtags)
}

/// Reply reference - alias for atrium type
pub type ReplyRef = ReplyRefData;

/// Post reference - alias for atrium type
pub type PostRef = MainData;

/// Quoted post data extracted from embeds
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuotedPost {
    pub uri: String,
    pub cid: String,
    pub did: String,
    pub handle: String,
    pub display_name: Option<String>,
    pub text: String,
    pub created_at: Option<DateTime<Utc>>,
}

impl BlueskyPost {
    /// Check if this post mentions a specific handle or DID
    pub fn mentions(&self, handle_or_did: &str) -> bool {
        self.facets.iter().any(|facet| {
            facet.features.iter().any(|feature| match feature {
                FacetFeature::Mention { did } => {
                    did == handle_or_did || handle_or_did.contains(did)
                }
                _ => false,
            })
        })
    }

    /// Get all mentioned DIDs
    pub fn mentioned_dids(&self) -> Vec<&str> {
        let mentions = self
            .facets
            .iter()
            .flat_map(|facet| &facet.features)
            .filter_map(|feature| match feature {
                FacetFeature::Mention { did } => Some(did.as_str()),
                _ => None,
            })
            .collect();
        mentions
    }

    /// Check if post has images
    pub fn has_images(&self) -> bool {
        use atrium_api::app::bsky::feed::post::RecordEmbedRefs;
        self.embed.as_ref().map_or(false, |e| {
            matches!(
                e,
                Union::Refs(RecordEmbedRefs::AppBskyEmbedImagesMain(_))
                    | Union::Refs(RecordEmbedRefs::AppBskyEmbedRecordWithMediaMain(_))
            )
        })
    }

    /// Extract alt text from image embeds (for accessibility)
    pub fn image_alt_texts(&self) -> Vec<String> {
        use atrium_api::app::bsky::feed::post::RecordEmbedRefs;
        match &self.embed {
            Some(Union::Refs(RecordEmbedRefs::AppBskyEmbedImagesMain(images))) => images
                .data
                .images
                .iter()
                .map(|img| img.alt.clone())
                .collect(),
            Some(Union::Refs(RecordEmbedRefs::AppBskyEmbedRecordWithMediaMain(
                record_with_media,
            ))) => {
                // Check if the media part contains images
                use atrium_api::app::bsky::embed::record_with_media::MainMediaRefs;
                match &record_with_media.data.media {
                    Union::Refs(MainMediaRefs::AppBskyEmbedImagesMain(images)) => images
                        .data
                        .images
                        .iter()
                        .map(|img| img.alt.clone())
                        .collect(),
                    _ => vec![],
                }
            }
            _ => vec![],
        }
    }

    /// Extract external link from embed (link cards)
    pub fn embedded_link(&self) -> Option<String> {
        use atrium_api::app::bsky::feed::post::RecordEmbedRefs;
        match &self.embed {
            Some(Union::Refs(RecordEmbedRefs::AppBskyEmbedExternalMain(external))) => {
                Some(external.data.external.uri.clone())
            }
            _ => None,
        }
    }

    /// Extract quoted post URI if this is a quote post
    pub fn quoted_post_uri(&self) -> Option<String> {
        use atrium_api::app::bsky::feed::post::RecordEmbedRefs;

        match &self.embed {
            Some(Union::Refs(RecordEmbedRefs::AppBskyEmbedRecordMain(record))) => {
                // The URI should be in record.data.record.uri
                Some(record.data.record.uri.clone())
            }
            Some(Union::Refs(RecordEmbedRefs::AppBskyEmbedRecordWithMediaMain(
                record_with_media,
            ))) => {
                // The URI should be in record_with_media.data.record.data.record.uri
                Some(record_with_media.data.record.data.record.uri.clone())
            }
            _ => None,
        }
    }

    /// This is a stub for backwards compatibility - use quoted_post_uri() and fetch the full post
    pub fn quoted_post(&self) -> Option<QuotedPost> {
        // Return None - the full post data should be fetched using bsky_agent when needed
        None
    }
}

/// Rich text facet for mentions, links, and hashtags
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Facet {
    pub index: ByteSlice,
    pub features: Vec<FacetFeature>,
}

/// Byte range for a facet (UTF-8 byte indices)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ByteSlice {
    pub byte_start: usize,
    pub byte_end: usize,
}

/// Feature type for a facet
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "$type")]
pub enum FacetFeature {
    #[serde(rename = "app.bsky.richtext.facet#mention")]
    Mention { did: String },

    #[serde(rename = "app.bsky.richtext.facet#link")]
    Link { uri: String },

    #[serde(rename = "app.bsky.richtext.facet#tag")]
    Tag { tag: String },
}

/// Cursor for Bluesky firehose - supports resumption
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlueskyFirehoseCursor {
    pub seq: u64,     // Jetstream sequence number
    pub time_us: u64, // Unix microseconds timestamp
}

/// Filter for Bluesky events
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BlueskyFilter {
    /// NSIDs to filter for (e.g., "app.bsky.feed.post")
    pub nsids: Vec<String>,
    /// Specific DIDs to watch (empty = all)
    pub dids: Vec<String>,
    /// Keywords to filter posts by
    pub keywords: Vec<String>,
    /// Languages to filter by (e.g., ["en", "es"])
    pub languages: Vec<String>,
    /// Only include posts that mention these DIDs/handles
    pub mentions: Vec<String>,

    // New fields for enhanced filtering
    /// Friends list - always see posts from these DIDs (bypasses mention requirement)
    #[serde(default)]
    pub friends: Vec<String>,
    /// Allow mentions from anyone, not just allowlisted DIDs
    #[serde(default)]
    pub allow_any_mentions: bool,
    /// Keywords to exclude - filter out posts containing these (takes precedence)
    #[serde(default)]
    pub exclude_keywords: Vec<String>,
    /// DIDs to exclude - never show posts from these (takes precedence over all inclusion filters)
    #[serde(default)]
    pub exclude_dids: Vec<String>,
}

/// Source statistics
#[derive(Debug, Default, Clone)]
struct SourceStats {
    events_received: u64,
    posts_processed: u64,
    errors: u64,
    last_seq: Option<u64>,
}

/// Consumes Bluesky Jetstream firehose
pub struct BlueskyFirehoseSource {
    source_id: String,
    endpoint: String,
    filter: BlueskyFilter,
    current_cursor: Option<BlueskyFirehoseCursor>,
    stats: SourceStats,
    buffer: Option<
        std::sync::Arc<parking_lot::Mutex<StreamBuffer<BlueskyPost, BlueskyFirehoseCursor>>>,
    >,
    notifications_enabled: bool,
    agent_handle: Option<AgentHandle>,
    bsky_agent: Option<Arc<bsky_sdk::BskyAgent>>,
    // Rate limiting
    last_send_time: std::sync::Arc<tokio::sync::Mutex<std::time::Instant>>,
    posts_per_second: f64,
    // Cursor persistence
    cursor_save_interval: std::time::Duration,
    cursor_save_threshold: u64, // Save after N events
    events_since_save: Arc<std::sync::Mutex<u64>>,
    cursor_file_path: Option<std::path::PathBuf>,
    last_cursor_save: Arc<tokio::sync::Mutex<std::time::Instant>>,
}

impl std::fmt::Debug for BlueskyFirehoseSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BlueskyFirehoseSource")
            .field("source_id", &self.source_id)
            .field("endpoint", &self.endpoint)
            .field("filter", &self.filter)
            .field("current_cursor", &self.current_cursor)
            .field("stats", &self.stats)
            .field("buffer", &self.buffer.is_some())
            .field("notifications_enabled", &self.notifications_enabled)
            .field("agent_handle", &self.agent_handle.is_some())
            .field("bsky_agent", &self.bsky_agent.is_some())
            .field("posts_per_second", &self.posts_per_second)
            .field("cursor_save_interval", &self.cursor_save_interval)
            .field("cursor_save_threshold", &self.cursor_save_threshold)
            .finish()
    }
}

impl BlueskyFirehoseSource {
    /// Load cursor from file
    async fn load_cursor_from_file(&self) -> Result<Option<BlueskyFirehoseCursor>> {
        if let Some(path) = &self.cursor_file_path {
            if path.exists() {
                let cursor_data = tokio::fs::read_to_string(path).await.map_err(|e| {
                    crate::CoreError::DataSourceError {
                        source_name: self.source_id.clone(),
                        operation: "load_cursor".to_string(),
                        cause: e.to_string(),
                    }
                })?;

                let cursor = serde_json::from_str(&cursor_data).map_err(|e| {
                    crate::CoreError::SerializationError {
                        data_type: "cursor".to_string(),
                        cause: e,
                    }
                })?;

                tracing::info!("Loaded Bluesky cursor from {:?}: {:?}", path, cursor);
                return Ok(Some(cursor));
            }
        }
        Ok(None)
    }

    /// Fetch user profile and format memory content
    async fn fetch_user_profile_for_memory(
        agent: &bsky_sdk::BskyAgent,
        handle: &str,
        did: &str,
    ) -> String {
        let mut memory_content = format!(
            "Bluesky user @{} (DID: {})\nFirst seen: {}\n",
            handle,
            did,
            chrono::Utc::now().format("%Y-%m-%d %H:%M:%S UTC")
        );

        // Try to fetch the user's profile
        if let Ok(profile_result) = agent
            .api
            .app
            .bsky
            .actor
            .get_profile(
                atrium_api::app::bsky::actor::get_profile::ParametersData {
                    actor: atrium_api::types::string::AtIdentifier::Did(
                        Did::from_str(did)
                            .unwrap_or_else(|_| Did::new("did:plc:unknown".to_string()).unwrap()),
                    ),
                }
                .into(),
            )
            .await
        {
            // Add profile information
            if let Some(display_name) = &profile_result.display_name {
                memory_content.push_str(&format!("Display name: {}\n", display_name));
            }
            if let Some(description) = &profile_result.description {
                memory_content.push_str(&format!("\nBio:\n{}\n", description));
            }
            if let Some(followers_count) = profile_result.followers_count {
                memory_content.push_str(&format!("\nFollowers: {}", followers_count));
            }
            if let Some(follows_count) = profile_result.follows_count {
                memory_content.push_str(&format!(", Following: {}", follows_count));
            }
            if let Some(posts_count) = profile_result.posts_count {
                memory_content.push_str(&format!(", Posts: {}\n", posts_count));
            }
        }

        memory_content
    }

    pub async fn new(
        source_id: impl Into<String>,
        endpoint: impl Into<String>,
        agent_handle: Option<AgentHandle>,
    ) -> Self {
        Self {
            source_id: source_id.into(),
            endpoint: endpoint.into(),
            filter: BlueskyFilter::default(),
            current_cursor: None,
            stats: SourceStats::default(),
            buffer: None,
            notifications_enabled: true,
            agent_handle,
            bsky_agent: None,
            last_send_time: std::sync::Arc::new(tokio::sync::Mutex::new(std::time::Instant::now())),
            posts_per_second: 1.0, // Default to 1 posts every second max
            cursor_save_interval: std::time::Duration::from_secs(60), // Save every minute
            cursor_save_threshold: 100, // Save every 100 events
            events_since_save: Arc::new(std::sync::Mutex::new(0)),
            cursor_file_path: None,
            last_cursor_save: Arc::new(tokio::sync::Mutex::new(std::time::Instant::now())),
        }
    }

    pub fn with_filter(mut self, filter: BlueskyFilter) -> Self {
        self.filter = filter;
        self
    }

    pub fn with_buffer(mut self, buffer: StreamBuffer<BlueskyPost, BlueskyFirehoseCursor>) -> Self {
        self.buffer = Some(std::sync::Arc::new(parking_lot::Mutex::new(buffer)));
        self
    }

    pub fn with_rate_limit(mut self, posts_per_second: f64) -> Self {
        self.posts_per_second = posts_per_second;
        self
    }

    pub fn with_cursor_file(mut self, path: impl Into<std::path::PathBuf>) -> Self {
        self.cursor_file_path = Some(path.into());
        self
    }

    /// Set Bluesky authentication credentials
    pub async fn with_auth(
        mut self,
        credentials: crate::atproto_identity::AtprotoAuthCredentials,
        handle: String,
    ) -> Result<Self> {
        use crate::atproto_identity::resolve_handle_to_pds;

        let pds_url = match resolve_handle_to_pds(&handle).await {
            Ok(url) => url,
            Err(url) => url,
        };

        let agent = bsky_sdk::BskyAgent::builder()
            .config(bsky_sdk::agent::config::Config {
                endpoint: pds_url,
                ..Default::default()
            })
            .build()
            .await
            .map_err(|e| crate::CoreError::ToolExecutionFailed {
                tool_name: "bluesky_firehose".to_string(),
                cause: format!("Failed to create BskyAgent: {:?}", e),
                parameters: serde_json::json!({}),
            })?;

        // Authenticate based on credential type
        match credentials {
            crate::atproto_identity::AtprotoAuthCredentials::OAuth { access_token: _ } => {
                return Err(crate::CoreError::ToolExecutionFailed {
                    tool_name: "bluesky_firehose".to_string(),
                    cause: "OAuth authentication not yet implemented for BskyAgent".to_string(),
                    parameters: serde_json::json!({}),
                });
            }
            crate::atproto_identity::AtprotoAuthCredentials::AppPassword {
                identifier,
                password,
            } => {
                agent.login(identifier, password).await.map_err(|e| {
                    crate::CoreError::ToolExecutionFailed {
                        tool_name: "bluesky_firehose".to_string(),
                        cause: format!("Login failed: {:?}", e),
                        parameters: serde_json::json!({}),
                    }
                })?;
            }
        };

        self.bsky_agent = Some(Arc::new(agent));
        Ok(self)
    }
}

#[async_trait]
impl DataSource for BlueskyFirehoseSource {
    type Item = BlueskyPost;
    type Filter = BlueskyFilter;
    type Cursor = BlueskyFirehoseCursor;

    fn source_id(&self) -> &str {
        &self.source_id
    }

    async fn pull(
        &mut self,
        limit: usize,
        _after: Option<Self::Cursor>,
    ) -> Result<Vec<Self::Item>> {
        // Jetstream is push-only, return buffered items if available
        if let Some(buffer) = &self.buffer {
            let buf = buffer.lock();
            // Get recent items from buffer
            let items = buf
                .get_range(None, None)
                .into_iter()
                .rev()
                .take(limit)
                .map(|event| event.item.clone())
                .collect();
            Ok(items)
        } else {
            Ok(vec![])
        }
    }

    async fn subscribe(
        &mut self,
        from: Option<Self::Cursor>,
    ) -> Result<Box<dyn Stream<Item = Result<StreamEvent<Self::Item, Self::Cursor>>> + Send + Unpin>>
    {
        // Try to load cursor from file if not provided
        let effective_cursor = match from {
            Some(cursor) => Some(cursor),
            None => self.load_cursor_from_file().await?,
        };
        // Apply collection filters (NSIDs map to collections)
        let collections = if !self.filter.nsids.is_empty() {
            self.filter.nsids.clone()
        } else {
            vec!["app.bsky.feed.post".to_string()]
        };

        // Build options with all settings
        let _options = if let Some(ref cursor) = effective_cursor {
            JetstreamOptions::builder()
                .cursor(cursor.time_us.to_string())
                .wanted_collections(collections.clone())
                .build()
        } else {
            JetstreamOptions::builder()
                .wanted_collections(collections.clone())
                .build()
        };

        // Keep track of cursor across reconnections
        let cursor: Arc<Mutex<Option<u64>>> =
            Arc::new(Mutex::new(effective_cursor.map(|c| c.time_us)));

        // Create channel for processed events
        let (tx, rx) = tokio::sync::mpsc::channel(5000);
        let filter = self.filter.clone();
        let source_id = self.source_id.clone();
        let buffer = self.buffer.clone();

        // Spawn task to process queued posts at configured rate
        if let Some(buffer) = &self.buffer {
            let queue_buffer = buffer.clone();
            let queue_tx = tx.clone();
            let posts_per_second = self.posts_per_second;
            let queue_last_send_time = self.last_send_time.clone();

            tokio::spawn(async move {
                let interval = std::time::Duration::from_secs_f64(1.0 / posts_per_second);
                let mut ticker = tokio::time::interval(interval);
                ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

                loop {
                    ticker.tick().await;

                    // Try to dequeue and send an event
                    let event = {
                        let mut buf = queue_buffer.lock();
                        buf.dequeue_for_processing()
                    };

                    if let Some(event) = event {
                        if let Err(e) = queue_tx.send(Ok(event)).await {
                            tracing::error!("Failed to send queued event: {}", e);
                            break;
                        } else {
                            tracing::debug!("Sent queued post from processing queue");
                            // Update last send time so the ingestor knows we're sending
                            let mut last_send = queue_last_send_time.lock().await;
                            *last_send = std::time::Instant::now();
                        }
                    }
                }
            });
        }

        // Spawn the connection manager task that handles recreating connections
        let manager_source_id = source_id;
        let manager_tx = tx.clone();
        let manager_filter = filter;
        let manager_buffer = buffer;
        let manager_collections = collections;
        let manager_last_send_time = self.last_send_time.clone();
        let manager_cursor_file = self.cursor_file_path.clone();
        let manager_cursor_save_interval = self.cursor_save_interval;
        let manager_cursor_save_threshold = self.cursor_save_threshold;
        let manager_events_since_save = self.events_since_save.clone();
        let manager_last_cursor_save = self.last_cursor_save.clone();
        let manager_posts_per_second = self.posts_per_second;

        tokio::spawn(async move {
            let mut connection_count = 0;
            let mut consecutive_failures: u32 = 0;
            const BASE_DELAY_SECS: u64 = 5;
            const MAX_DELAY_SECS: u64 = 300; // 5 minutes max

            // Outer loop that recreates the connection when it dies
            loop {
                connection_count += 1;
                tracing::info!("Creating jetstream connection #{}", connection_count);

                // Build options based on current cursor
                let current_cursor = {
                    let cursor_lock = cursor.lock();
                    if let Ok(guard) = cursor_lock {
                        guard.clone()
                    } else {
                        None
                    }
                };

                let options = if let Some(cursor_us) = current_cursor {
                    JetstreamOptions::builder()
                        .cursor(cursor_us.to_string())
                        .wanted_collections(manager_collections.clone())
                        .build()
                } else {
                    JetstreamOptions::builder()
                        .wanted_collections(manager_collections.clone())
                        .build()
                };

                // Create new connection
                let connection = JetstreamConnection::new(options);

                // Create our ingestor that sends posts to our channel
                let post_ingestor = PostIngestor {
                    tx: manager_tx.clone(),
                    filter: manager_filter.clone(),
                    buffer: manager_buffer.clone(),
                    resolver: atproto_identity_resolver(),
                    last_send_time: manager_last_send_time.clone(),
                    min_interval: std::time::Duration::from_secs_f64(
                        1.0 / manager_posts_per_second,
                    ),
                    // Cursor persistence
                    cursor_file_path: manager_cursor_file.clone(),
                    cursor_save_interval: manager_cursor_save_interval,
                    cursor_save_threshold: manager_cursor_save_threshold,
                    events_since_save: manager_events_since_save.clone(),
                    last_cursor_save: manager_last_cursor_save.clone(),
                };

                let mut ingestors: HashMap<String, Box<dyn LexiconIngestor + Send + Sync>> =
                    HashMap::new();
                ingestors.insert("app.bsky.feed.post".to_string(), Box::new(post_ingestor));

                let msg_rx = connection.get_msg_rx();
                let reconnect_tx = connection.get_reconnect_tx();

                let c_cursor = cursor.clone();
                let c_source_id = manager_source_id.clone();
                let ingestor_tx = manager_tx.clone();

                // Spawn task to consume events from this connection
                let handle = tokio::spawn(async move {
                    // Process messages from jetstream with a timeout
                    // If we don't receive any message for 60 seconds, assume connection is dead
                    const MESSAGE_TIMEOUT_SECS: u64 = 60;

                    loop {
                        match tokio::time::timeout(
                            Duration::from_secs(MESSAGE_TIMEOUT_SECS),
                            msg_rx.recv_async(),
                        )
                        .await
                        {
                            Ok(Ok(message)) => {
                                // Got a message, process it
                                if let Err(e) = handler::handle_message(
                                    message,
                                    &ingestors,
                                    reconnect_tx.clone(),
                                    c_cursor.clone(),
                                )
                                .await
                                {
                                    tracing::warn!("Error processing message: {}", e);
                                    let _ =
                                        ingestor_tx.send(Err(crate::CoreError::DataSourceError {
                                            source_name: c_source_id.clone(),
                                            operation: "process".to_string(),
                                            cause: e.to_string(),
                                        }));
                                }
                            }
                            Ok(Err(_)) => {
                                // Channel closed
                                tracing::info!("Message channel closed, connection terminated");
                                break;
                            }
                            Err(_) => {
                                // Timeout - no messages for 60 seconds
                                tracing::warn!(
                                    "No messages received for {} seconds, assuming connection is dead",
                                    MESSAGE_TIMEOUT_SECS
                                );
                                break;
                            }
                        }
                    }
                    tracing::info!("Message handler exiting");
                });

                // Try to connect (simplified - let rocketman handle retries)
                if connection
                    .connect(cursor.clone())
                    .await
                    .inspect_err(|e| {
                        tracing::error!(
                            "Failed to establish connection #{}: {}",
                            connection_count,
                            e
                        );
                    })
                    .is_ok()
                {
                    tracing::info!("Jetstream connection #{} established", connection_count);
                    consecutive_failures = 0; // Reset on successful connection

                    // Wait for the handler to exit (connection died)
                    let _ = handle.await;

                    tracing::warn!(
                        "Jetstream connection #{} died, will recreate",
                        connection_count
                    );
                } else {
                    consecutive_failures += 1;
                    // Abort the handler since we never connected
                    handle.abort();
                }

                // Calculate exponential backoff delay
                let delay_secs = if consecutive_failures == 0 {
                    BASE_DELAY_SECS
                } else {
                    let exponential_delay = BASE_DELAY_SECS.saturating_mul(
                        2_u64.saturating_pow(consecutive_failures.saturating_sub(1) as u32),
                    );
                    exponential_delay.min(MAX_DELAY_SECS)
                };

                tracing::info!(
                    "Waiting {} seconds before recreating connection (failure count: {})",
                    delay_secs,
                    consecutive_failures
                );

                // Wait before recreating the connection
                tokio::time::sleep(Duration::from_secs(delay_secs)).await;

                // Continue loop to recreate connection
                tracing::info!("Recreating jetstream connection after delay...");
            }
        });
        Ok(Box::new(ReceiverStream::new(rx))
            as Box<
                dyn Stream<Item = Result<StreamEvent<Self::Item, Self::Cursor>>> + Send + Unpin,
            >)
    }

    fn set_filter(&mut self, filter: Self::Filter) {
        self.filter = filter;
    }

    fn current_cursor(&self) -> Option<Self::Cursor> {
        self.current_cursor.clone()
    }

    fn metadata(&self) -> DataSourceMetadata {
        DataSourceMetadata {
            source_type: "bluesky_firehose".to_string(),
            status: DataSourceStatus::Active,
            items_processed: self.stats.posts_processed,
            last_item_time: self
                .current_cursor
                .as_ref()
                .map(|c| DateTime::from_timestamp_micros(c.time_us as i64).unwrap_or_default()),
            error_count: self.stats.errors,
            custom: HashMap::from([
                (
                    "events_received".to_string(),
                    json!(self.stats.events_received),
                ),
                ("last_seq".to_string(), json!(self.stats.last_seq)),
                ("endpoint".to_string(), json!(&self.endpoint)),
                ("filter".to_string(), json!(&self.filter)),
            ]),
        }
    }

    fn buffer_config(&self) -> super::BufferConfig {
        // High-volume firehose needs large buffer and short TTL
        super::BufferConfig {
            max_items: 10_000,
            max_age: std::time::Duration::from_secs(300), // 5 minutes
            notify_changes: true,
            persist_to_db: false, // Too high volume for DB
            index_content: false, // Would overwhelm the index
        }
    }

    async fn format_notification(
        &self,
        item: &Self::Item,
    ) -> Option<(String, Vec<(CompactString, MemoryBlock)>)> {
        // Clone the item so we can mutate it
        let mut item = item.clone();

        // Format based on post type
        let mut message = String::new();
        let mut reply_candidates = Vec::new();
        let mut memory_blocks = Vec::new();

        // Collect users for memory blocks
        let mut thread_users: Vec<(String, String)> = Vec::new();

        let mut mention_check_queue = Vec::new();

        mention_check_queue.append(
            &mut item
                .mentioned_dids()
                .iter()
                .map(|s| s.to_string())
                .collect(),
        );

        // Also check mentions in quoted posts
        if let Some(quoted) = item.quoted_post() {
            // Add the quoted post author's DID to check
            mention_check_queue.push(quoted.did.clone());

            // TODO: Parse facets from quoted post text to extract mentions
            // For now, do a simple @handle search in the text
            let words: Vec<&str> = quoted.text.split_whitespace().collect();
            for word in words {
                if word.starts_with('@') {
                    mention_check_queue.push(word.to_string());
                }
            }
        }

        // Get agent's DID from the mentions filter (it's monitoring for mentions of itself)
        let agent_did = self.filter.mentions.first().cloned();

        // Try to fetch the author's display name if we have a BskyAgent
        if item.display_name.is_none() {
            if let Some(bsky_agent) = &self.bsky_agent {
                if let Ok(profile_result) = bsky_agent
                    .api
                    .app
                    .bsky
                    .actor
                    .get_profile(
                        atrium_api::app::bsky::actor::get_profile::ParametersData {
                            actor: atrium_api::types::string::AtIdentifier::Did(
                                Did::from_str(&item.did).unwrap_or_else(|_| {
                                    Did::new("did:plc:unknown".to_string()).unwrap()
                                }),
                            ),
                        }
                        .into(),
                    )
                    .await
                {
                    item.display_name = profile_result.display_name.clone();
                }
            }
        }

        // Header with author
        let author_str = if let Some(display_name) = &item.display_name {
            format!("{} (@{})", display_name, item.handle)
        } else {
            format!("@{}", item.handle)
        };
        message.push_str(&format!("💬 {}", author_str));

        // Add context for replies/mentions
        if let Some(reply) = &item.reply {
            message.push_str(" replied");

            // If we have a BskyAgent, try to fetch thread context
            if let Some(bsky_agent_arc) = &self.bsky_agent {
                let bsky_agent = bsky_agent_arc.clone();

                // Use constellation-enhanced thread context
                let http_client = PatternHttpClient::default();

                // Build comprehensive thread context with siblings
                let thread_context = match http_client
                    .build_thread_context(
                        &item.uri,
                        Some(&reply.parent.uri),
                        &bsky_agent,
                        agent_did.as_deref(),
                        &self.filter,
                        3, // Max depth for sibling replies
                    )
                    .await
                {
                    Ok(ctx) => ctx,
                    Err(e) => {
                        tracing::warn!("Failed to build thread context: {}", e);
                        // Fall back to basic context
                        ThreadContext {
                            parent: None,
                            siblings: Vec::new(),
                            replies_map: std::collections::HashMap::new(),
                            engagement_map: std::collections::HashMap::new(),
                            agent_interactions: std::collections::HashMap::new(),
                        }
                    }
                };

                // Collect users from thread context
                // Add parent post author if available
                if let Some(parent) = &thread_context.parent {
                    thread_users.push((
                        parent.author.handle.as_str().to_string(),
                        parent.author.did.as_str().to_string(),
                    ));
                }

                // Add sibling post authors
                for sibling in &thread_context.siblings {
                    thread_users.push((
                        sibling.author.handle.as_str().to_string(),
                        sibling.author.did.as_str().to_string(),
                    ));
                }

                // Add authors from replies
                for replies in thread_context.replies_map.values() {
                    for reply in replies {
                        thread_users.push((
                            reply.author.handle.as_str().to_string(),
                            reply.author.did.as_str().to_string(),
                        ));
                    }
                }

                // Walk up the thread to collect parent posts (keep existing logic for now)
                let mut thread_posts = Vec::new();
                let mut current_uri = Some(reply.parent.uri.clone());
                let mut depth = 0;
                const MAX_DEPTH: usize = 5; // Reduced since we have siblings now

                while let Some(uri) = current_uri.take() {
                    if depth >= MAX_DEPTH {
                        break;
                    }

                    let params = atrium_api::app::bsky::feed::get_posts::ParametersData {
                        uris: vec![uri.clone()],
                    };

                    if let Ok(posts_result) =
                        bsky_agent.api.app.bsky.feed.get_posts(params.into()).await
                    {
                        if let Some(post_view) = posts_result.posts.first() {
                            if let Some((text, langs, features, alt_texts)) =
                                extract_post_data(post_view)
                            {
                                let handle = post_view.author.handle.as_str().to_string();
                                let display_name = post_view.author.display_name.clone();
                                let mentions: Vec<_> = features
                                    .iter()
                                    .filter_map(|f| match f {
                                        FacetFeature::Mention { did } => Some(format!("@{}", did)),
                                        _ => None,
                                    })
                                    .collect();

                                let links: Vec<_> = features
                                    .iter()
                                    .filter_map(|f| match f {
                                        FacetFeature::Link { uri } => Some(uri.clone()),
                                        _ => None,
                                    })
                                    .collect();

                                mention_check_queue.append(&mut mentions.clone());

                                let relative_time =
                                    format!(" - {}", extract_post_relative_time(post_view));

                                // Check if this is the agent's post
                                let is_agent_post = agent_did
                                    .as_ref()
                                    .map(|did| post_view.author.did.as_str() == did)
                                    .unwrap_or(false);

                                // Collect user for memory block
                                let author_did = post_view.author.did.as_str().to_string();
                                thread_users.push((handle.clone(), author_did));

                                let embed_info =
                                    extract_embed_info(post_view, self.bsky_agent.as_ref()).await;

                                thread_posts.push((
                                    handle,
                                    display_name,
                                    text,
                                    post_view.uri.clone(),
                                    depth,
                                    mentions,
                                    langs,
                                    alt_texts,
                                    links,
                                    relative_time,
                                    is_agent_post,
                                    embed_info,
                                ));

                                // Add as reply candidate
                                reply_candidates.push(thread_post_to_candidate(post_view));

                                // Continue walking up
                                if let Ok(record_value) = serde_json::to_value(&post_view.record) {
                                    if let Ok(post_record) =
                                        serde_json::from_value::<
                                            atrium_api::app::bsky::feed::post::RecordData,
                                        >(record_value)
                                    {
                                        if let Some(reply_ref) = post_record.reply {
                                            current_uri = Some(reply_ref.parent.uri.clone());
                                            depth += 1;
                                        }
                                    }
                                }
                            }
                        }
                    } else {
                        break;
                    }
                }

                if !thread_posts.is_empty() {
                    message.push_str(" in thread:\n\n");

                    // Display thread posts in reverse order (root to leaf)
                    for (
                        handle,
                        display_name,
                        text,
                        _uri,
                        depth,
                        mentions,
                        langs,
                        _alt_texts,
                        links,
                        relative_time,
                        is_agent_post,
                        embed_info,
                    ) in thread_posts.iter().rev()
                    {
                        let indent = "  ".repeat(*depth);
                        let bullet = if *depth == 0 { "•" } else { "└─" };

                        let author_str = if let Some(name) = display_name {
                            format!("{} (@{})", name, handle)
                        } else {
                            format!("@{}", handle)
                        };

                        // Mark agent's posts
                        let prefix = if *is_agent_post { "[YOU] " } else { "" };

                        message.push_str(&format!(
                            "{}{} {}{}{}: {}\n",
                            indent, bullet, prefix, author_str, relative_time, text
                        ));
                        message.push_str(&format!("{}   🔗 {}\n", indent, _uri));

                        // Show mentions if any
                        if !mentions.is_empty() {
                            message.push_str(&format!(
                                "{}   [mentions: {}]\n",
                                indent,
                                mentions.join(", ")
                            ));
                        }

                        // Show links if any
                        if !links.is_empty() {
                            message.push_str(&format!(
                                "{}   [🔗 Links: {}]\n",
                                indent,
                                links.join(", ")
                            ));
                        }

                        // Show language if not English
                        if !langs.is_empty() && !langs.contains(&"en".to_string()) {
                            message.push_str(&format!(
                                "{}   [langs: {}]\n",
                                indent,
                                langs.join(", ")
                            ));
                        }

                        // Show embeds if any
                        if let Some(embed) = embed_info {
                            message.push_str(&embed.format_display(&indent));
                        }
                    }

                    // Add siblings context if we have any
                    if !thread_context.siblings.is_empty() {
                        message.push_str("\n[Other replies to parent:]\n");

                        for sibling in &thread_context.siblings {
                            // Extract post data
                            if let Some((text, _langs, features, _alt_texts)) =
                                extract_post_data(sibling)
                            {
                                let embed_info =
                                    extract_embed_info(sibling, self.bsky_agent.as_ref()).await;
                                let author_str = if let Some(name) = &sibling.author.display_name {
                                    format!("{} (@{})", name, sibling.author.handle.as_str())
                                } else {
                                    format!("@{}", sibling.author.handle.as_str())
                                };

                                // Check if this is the agent's post
                                let is_agent = agent_did
                                    .as_ref()
                                    .map(|did| sibling.author.did.as_str() == did)
                                    .unwrap_or(false);
                                let prefix = if is_agent { "[YOU] " } else { "" };

                                // Get engagement metrics
                                let engagement = thread_context
                                    .engagement_map
                                    .get(&sibling.uri)
                                    .map(|e| {
                                        format!(
                                            " [💬{} ❤️{} 🔄{}]",
                                            e.reply_count, e.like_count, e.repost_count
                                        )
                                    })
                                    .unwrap_or_default();

                                let relative_time =
                                    format!(" - {}", extract_post_relative_time(sibling));

                                message.push_str(&format!(
                                    "  └─ {}{}{}: {}{}\n",
                                    prefix, author_str, relative_time, text, engagement
                                ));
                                message.push_str(&format!("│  🔗 {}\n", sibling.uri));

                                // Show embeds if any
                                if let Some(embed) = embed_info {
                                    let embed_display = embed.format_display("  ");
                                    // Add vertical bar prefix to each line
                                    for line in embed_display.lines() {
                                        if !line.is_empty() {
                                            message.push_str(&format!("│{}\n", line));
                                        }
                                    }
                                }

                                // Extract mentions from features for checking
                                let mentions: Vec<_> = features
                                    .iter()
                                    .filter_map(|f| match f {
                                        FacetFeature::Mention { did } => Some(format!("@{}", did)),
                                        _ => None,
                                    })
                                    .collect();
                                mention_check_queue.append(&mut mentions.clone());

                                // Add as reply candidate
                                reply_candidates.push(thread_post_to_candidate(sibling));

                                // Show replies to this sibling if any
                                if let Some(replies) = thread_context.replies_map.get(&sibling.uri)
                                {
                                    for reply in replies.iter().take(3) {
                                        if let Some((reply_text, _, _, _)) =
                                            extract_post_data(reply)
                                        {
                                            let reply_embed_info =
                                                extract_embed_info(reply, self.bsky_agent.as_ref())
                                                    .await;
                                            let reply_author =
                                                if let Some(name) = &reply.author.display_name {
                                                    format!(
                                                        "{} (@{})",
                                                        name,
                                                        reply.author.handle.as_str()
                                                    )
                                                } else {
                                                    format!("@{}", reply.author.handle.as_str())
                                                };

                                            let is_agent_reply = agent_did
                                                .as_ref()
                                                .map(|did| reply.author.did.as_str() == did)
                                                .unwrap_or(false);
                                            let reply_prefix =
                                                if is_agent_reply { "[YOU] " } else { "" };

                                            let relative_time =
                                                format!(" - {}", extract_post_relative_time(reply));

                                            message.push_str(&format!(
                                                "│  └─ {}{}: {}{}\n",
                                                reply_prefix,
                                                reply_author,
                                                reply_text,
                                                relative_time
                                            ));

                                            // Show embeds if any
                                            if let Some(embed) = reply_embed_info {
                                                let embed_display = embed.format_display("    ");
                                                // Add vertical bar and indentation prefix to each line
                                                for line in embed_display.lines() {
                                                    if !line.is_empty() {
                                                        message.push_str(&format!("│  {}\n", line));
                                                    }
                                                }
                                            }

                                            // Add as reply candidate
                                            reply_candidates.push(thread_post_to_candidate(reply));
                                        }
                                    }

                                    let remaining = replies.len().saturating_sub(2);
                                    if remaining > 0 {
                                        message.push_str(&format!(
                                            "│     [{} more replies...]\n",
                                            remaining
                                        ));
                                    }
                                }
                            }
                        }
                    }

                    // Mark the main post clearly
                    message.push_str("\n>>> MAIN POST >>>\n");
                } else {
                    // Fallback if we can't fetch the parent
                    message.push_str(&format!(" to {}:\n", reply.parent.uri));
                }
            } else {
                // No BskyAgent available, just show the URI
                message.push_str(&format!(" to {}:\n", reply.parent.uri));
            }
        } else if item.mentions(&self.source_id) {
            message.push_str(" mentioned you:\n");
        }

        if let Some(bsky_agent) = &self.bsky_agent {
            let params = atrium_api::app::bsky::feed::get_posts::ParametersData {
                uris: vec![item.uri.clone()],
            };

            if let Ok(posts_result) = bsky_agent.api.app.bsky.feed.get_posts(params.into()).await {
                if let Some(post) = posts_result.data.posts.first() {
                    if let Some((post_text, _, _, _)) = extract_post_data(post) {
                        let post_embed_info =
                            extract_embed_info(post, self.bsky_agent.as_ref()).await;
                        let post_author = if let Some(name) = &post.author.display_name {
                            format!("{} (@{})", name, post.author.handle.as_str())
                        } else {
                            format!("@{}", post.author.handle.as_str())
                        };

                        let is_agent_post = agent_did
                            .as_ref()
                            .map(|did| post.author.did.as_str() == did)
                            .unwrap_or(false);
                        let post_prefix = if is_agent_post { "[YOU] " } else { "" };

                        let relative_time = format!(" - {}", extract_post_relative_time(post));

                        message.push_str(&format!(
                            "{}{}: {}{}\n",
                            post_prefix, post_author, post_text, relative_time
                        ));

                        // Show embeds if any
                        if let Some(embed) = post_embed_info {
                            let embed_display = embed.format_display(" ");
                            // Add vertical bar and indentation prefix to each line
                            for line in embed_display.lines() {
                                if !line.is_empty() {
                                    message.push_str(&format!("│  {}\n", line));
                                }
                            }
                        }
                    }
                }
            }
        } else {
            // Full post text
            message.push_str(&format!("@{}: {}", item.handle, item.text));
            let interval = Utc::now() - item.created_at;
            let relative_time = format!(
                "\n{} ago\n",
                format_duration(interval.to_std().unwrap_or(Duration::from_secs(0)))
            );
            message.push_str(&relative_time);

            // Extract and display embeds (including quotes)
            if let Some(embed) = &item.embed {
                if let Some(post_embed_info) = extract_record_embed(embed) {
                    // Show embeds if any
                    let embed_display = post_embed_info.format_display("");
                    // Add vertical bar and indentation prefix to each line
                    for line in embed_display.lines() {
                        if !line.is_empty() {
                            message.push_str(&format!("│  {}\n", line));
                        }
                    }
                }
            }
            // Add image indicator if present
            if item.has_images() {
                let alt_texts = item.image_alt_texts();
                if !alt_texts.is_empty() {
                    message.push_str(&format!("\n[📸 {} image(s)]", alt_texts.len()));
                }
                for alt_text in alt_texts {
                    message.push_str(&format!("\n alt text: {}", alt_text));
                }
            }

            // Extract and display links from facets
            let mut links: Vec<_> = item
                .facets
                .iter()
                .flat_map(|facet| &facet.features)
                .filter_map(|feature| match feature {
                    FacetFeature::Link { uri } => Some(uri.clone()),
                    _ => None,
                })
                .collect();

            // Also check for embedded link card
            if let Some(embed_link) = item.embedded_link() {
                links.push(embed_link);
            }

            if !links.is_empty() {
                message.push_str("\n[🔗 Links:");
                for link in &links {
                    message.push_str(&format!(" {}", link));
                }
                message.push_str("]");
            }

            // Add link
            message.push_str(&format!("\n🔗 {}", item.uri));
        }

        // Show replies to the main post using constellation
        if self.bsky_agent.is_some() {
            // Fetch replies to the current post
            let http_client = PatternHttpClient::default();
            if let Ok(reply_records) = http_client.fetch_thread_siblings(&item.uri).await {
                // Filter replies based on our criteria
                let filtered_replies = PatternHttpClient::filter_constellation_records(
                    reply_records,
                    agent_did.as_deref(),
                    &self.filter,
                    &item.uri,
                );

                if !filtered_replies.is_empty() {
                    let reply_uris: Vec<String> = filtered_replies
                        .into_iter()
                        .take(5) // Limit to 5 replies
                        .map(|record| record.to_at_uri())
                        .collect();

                    if let Some(bsky_agent) = &self.bsky_agent {
                        let params = atrium_api::app::bsky::feed::get_posts::ParametersData {
                            uris: reply_uris,
                        };

                        if let Ok(replies_result) =
                            bsky_agent.api.app.bsky.feed.get_posts(params.into()).await
                        {
                            if !replies_result.posts.is_empty() {
                                message.push_str("\n<<< REPLIES <<<\n");

                                for reply_post in &replies_result.posts {
                                    if let Some((reply_text, langs, features, alt_texts)) =
                                        extract_post_data(reply_post)
                                    {
                                        let author_str =
                                            if let Some(name) = &reply_post.author.display_name {
                                                format!(
                                                    "{} (@{})",
                                                    name,
                                                    reply_post.author.handle.as_str()
                                                )
                                            } else {
                                                format!("@{}", reply_post.author.handle.as_str())
                                            };

                                        // Check if this is the agent's reply
                                        let is_agent_reply = agent_did
                                            .as_ref()
                                            .map(|did| reply_post.author.did.as_str() == did)
                                            .unwrap_or(false);
                                        let prefix = if is_agent_reply { "[YOU] " } else { "" };

                                        // Get engagement metrics
                                        let engagement = format!(
                                            " [💬{} ❤️{} 🔄{}]",
                                            reply_post.reply_count.unwrap_or(0),
                                            reply_post.like_count.unwrap_or(0),
                                            reply_post.repost_count.unwrap_or(0)
                                        );

                                        let relative_time = format!(
                                            " - {}",
                                            extract_post_relative_time(reply_post)
                                        );

                                        message.push_str(&format!(
                                            "  └─ {}{}: {}{}{}\n",
                                            prefix,
                                            author_str,
                                            reply_text,
                                            engagement,
                                            relative_time
                                        ));
                                        message.push_str(&format!("     🔗 {}\n", reply_post.uri));

                                        // Extract mentions for checking
                                        let mentions: Vec<_> = features
                                            .iter()
                                            .filter_map(|f| match f {
                                                FacetFeature::Mention { did } => {
                                                    Some(format!("@{}", did))
                                                }
                                                _ => None,
                                            })
                                            .collect();
                                        mention_check_queue.append(&mut mentions.clone());

                                        // Show links if any
                                        let links: Vec<_> = features
                                            .iter()
                                            .filter_map(|f| match f {
                                                FacetFeature::Link { uri } => Some(uri.clone()),
                                                _ => None,
                                            })
                                            .collect();

                                        if !links.is_empty() {
                                            message.push_str(&format!(
                                                "     [🔗 Links: {}]\n",
                                                links.join(", ")
                                            ));
                                        }

                                        // Show language if not English
                                        if !langs.is_empty() && !langs.contains(&"en".to_string()) {
                                            message.push_str(&format!(
                                                "     [langs: {}]\n",
                                                langs.join(", ")
                                            ));
                                        }

                                        // Show images if any
                                        if !alt_texts.is_empty() {
                                            message.push_str(&format!(
                                                "     [📸 {} image(s)]\n",
                                                alt_texts.len()
                                            ));
                                            for alt_text in &alt_texts {
                                                message.push_str(&format!(
                                                    "      alt text: {}\n",
                                                    alt_text
                                                ));
                                            }
                                        }

                                        // Add as reply candidate
                                        reply_candidates.push(thread_post_to_candidate(reply_post));
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        // First check exclusion keywords - these take highest priority
        // Check the entire formatted message for excluded keywords
        if !self.filter.exclude_keywords.is_empty() {
            let message_lower = message.to_lowercase();
            for keyword in &self.filter.exclude_keywords {
                if message_lower.contains(&keyword.to_lowercase()) {
                    tracing::debug!(
                        "dropping thread because it contains excluded keyword '{}' in formatted message",
                        keyword
                    );
                    return None;
                }
            }
        }

        // Check if post should be included based on friends list or mentions
        // Friends bypass all mention requirements
        let is_from_friend = self.filter.friends.contains(&item.did);

        // Check if Pattern (or any watched DID) authored this post or any parent post
        let is_from_watched_author = self.filter.mentions.contains(&item.did);

        // Check if this is a reply to the agent's own post
        let is_reply_to_self = if let Some(reply) = &item.reply {
            self.filter
                .mentions
                .first()
                .map(|agent_did| reply.parent.uri.contains(agent_did))
                .unwrap_or(false)
        } else {
            false
        };

        if !is_from_friend
            && !is_from_watched_author
            && !is_reply_to_self
            && !self.filter.mentions.is_empty()
        {
            // Not from a friend, watched author, or reply to self, so check if any of the DIDs we're watching for were mentioned
            let found_mention = self.filter.mentions.iter().any(|watched_did| {
                // Check both with and without @ prefix since the queue has mixed format
                mention_check_queue.contains(watched_did)
                    || mention_check_queue.contains(&format!("@{}", watched_did))
            });

            if !found_mention {
                tracing::debug!(
                    "dropping thread because it didn't mention any watched DIDs: {:?}, message:\n{}",
                    mention_check_queue,
                    message
                );
                return None;
            }
        }

        if let Some(agent_handle) = &self.agent_handle {
            // Collect users to check/create memory blocks for
            let mut users_to_check = vec![(item.handle.clone(), item.did.clone())];

            // Add any thread users we collected
            for (handle, did) in thread_users {
                users_to_check.push((handle, did));
            }

            // Process each user
            for (handle, did) in users_to_check {
                let memory_label = format!("bluesky_user_{}", handle);
                let compact_label = CompactString::from(memory_label.clone());

                // Use the new get method to check for exact label match
                if let Ok(existing_memory) = agent_handle
                    .get_archival_memory_by_label(&memory_label)
                    .await
                {
                    if let Some(existing_block) = existing_memory {
                        // Add existing block to our return list
                        memory_blocks.push((compact_label, existing_block));
                        message.push_str(&format!("\n\n📝 Memory exists: {}", memory_label));
                    } else {
                        // Create new memory block
                        let memory_content = if let Some(bsky_agent) = &self.bsky_agent {
                            Self::fetch_user_profile_for_memory(&*bsky_agent, &handle, &did).await
                        } else {
                            create_basic_memory_content(&handle, &did)
                        };

                        // Insert into agent's archival memory
                        if let Err(e) = agent_handle
                            .insert_working_memory(&memory_label, &memory_content)
                            .await
                        {
                            tracing::warn!("Failed to create memory block for {}: {}", handle, e);
                        } else {
                            message.push_str(&format!("\n\n📝 Memory created: {}", memory_label));

                            // Now retrieve the created block to return it
                            if let Ok(Some(created_block)) = agent_handle
                                .get_archival_memory_by_label(&memory_label)
                                .await
                            {
                                memory_blocks.push((compact_label, created_block));
                            }
                        }
                    }
                }
            }
        }

        // Add the original post as a reply candidate too
        reply_candidates.push((item.uri.clone(), format!("@{}", item.handle)));

        // Add reply guidance at the very end
        if !reply_candidates.is_empty() {
            message.push_str("\n\n💭 Reply options (choose at most one):\n");
            for (uri, handle) in &reply_candidates {
                message.push_str(&format!("  • {} ({})\n", handle, uri));
            }
        }
        message
            .push_str("If you choose to reply (by using send_message with target_type bluesky and the target_id set to the uri of the post you want to reply to, from the above options), your response must contain under 300 characters or it will be truncated.\nAlternatively, you can 'like' the post by submitting a reply with 'like' as the sole text");

        Some((message, memory_blocks))
    }

    fn get_buffer_stats(&self) -> Option<super::BufferStats> {
        self.buffer.as_ref().map(|b| b.lock().stats())
    }

    fn set_notifications_enabled(&mut self, enabled: bool) {
        self.notifications_enabled = enabled;
    }

    fn notifications_enabled(&self) -> bool {
        self.notifications_enabled
    }

    async fn search(&self, query: &str, limit: usize) -> Result<Vec<Self::Item>> {
        if let Some(buffer) = &self.buffer {
            let buf = buffer.lock();
            let results = buf.search(query, limit);
            Ok(results
                .into_iter()
                .map(|event| event.item.clone())
                .collect())
        } else {
            Ok(vec![])
        }
    }
}

impl PostIngestor {
    /// Save cursor to file
    #[allow(dead_code)]
    async fn save_cursor(&self, cursor: &BlueskyFirehoseCursor) -> Result<()> {
        if let Some(path) = &self.cursor_file_path {
            let cursor_data = serde_json::to_string_pretty(cursor).map_err(|e| {
                crate::CoreError::SerializationError {
                    data_type: "cursor".to_string(),
                    cause: e,
                }
            })?;

            tokio::fs::write(path, cursor_data).await.map_err(|e| {
                crate::CoreError::DataSourceError {
                    source_name: "bluesky_firehose".to_string(),
                    operation: "save_cursor".to_string(),
                    cause: e.to_string(),
                }
            })?;

            tracing::debug!("Saved Bluesky cursor to {:?}", path);
        }
        Ok(())
    }
}

/// Ingestor that processes Bluesky posts and sends them to our channel
struct PostIngestor {
    tx: tokio::sync::mpsc::Sender<Result<StreamEvent<BlueskyPost, BlueskyFirehoseCursor>>>,
    filter: BlueskyFilter,
    #[allow(unused)]
    buffer: Option<Arc<parking_lot::Mutex<StreamBuffer<BlueskyPost, BlueskyFirehoseCursor>>>>,
    resolver: atrium_identity::did::CommonDidResolver<PatternHttpClient>,
    // Rate limiting
    last_send_time: Arc<tokio::sync::Mutex<std::time::Instant>>,
    min_interval: std::time::Duration,
    // Cursor persistence
    cursor_file_path: Option<std::path::PathBuf>,
    cursor_save_interval: std::time::Duration,
    cursor_save_threshold: u64,
    events_since_save: Arc<std::sync::Mutex<u64>>,
    last_cursor_save: Arc<tokio::sync::Mutex<std::time::Instant>>,
}

#[async_trait]
impl LexiconIngestor for PostIngestor {
    async fn ingest(&self, event: Event<serde_json::Value>) -> anyhow::Result<()> {
        // Only process commit events for posts
        if let Some(Commit {
            record: Some(record),
            cid: Some(cid),
            rkey,
            collection,
            ..
        }) = event.commit
        {
            let post =
                serde_json::from_value::<atrium_api::app::bsky::feed::post::RecordData>(record)?;

            let rcid = match atrium_api::types::string::Cid::from_str(&cid) {
                Ok(r) => r,
                Err(e) => return Err(anyhow::anyhow!(e)),
            };

            let uri = format!("at://{}/{}/{}", event.did, collection, rkey);
            let now = chrono::Utc::now();

            // Extract all the post data using our helper first
            let (text, langs, labels, facets) = extract_post_from_record(&post);

            let mut post_to_filter = BlueskyPost {
                uri,

                display_name: None,
                did: event.did.to_string(),
                cid: rcid,
                handle: event.did.to_string(), // temporary, need to do handle resolution
                text,
                created_at: chrono::DateTime::parse_from_rfc3339(post.created_at.as_str())
                    .expect("incorrect time format")
                    .to_utc(),
                reply: post.reply.map(|r| r.data),
                embed: post.embed,
                langs,
                labels,
                facets,
            };

            if should_include_post(&mut post_to_filter, &self.filter, &self.resolver).await {
                let cursor = BlueskyFirehoseCursor {
                    seq: now.timestamp_micros() as u64,
                    time_us: now.timestamp_micros() as u64,
                };

                let event = StreamEvent {
                    item: post_to_filter.clone(),
                    cursor: cursor.clone(),
                    timestamp: now,
                };

                // Apply rate limiting
                let mut last_send = self.last_send_time.lock().await;
                let elapsed = last_send.elapsed();

                if elapsed < self.min_interval {
                    // Need to rate limit - add to processing queue
                    if let Some(buffer) = &self.buffer {
                        let mut buffer_guard = buffer.lock();
                        if buffer_guard.queue_for_processing(event.clone()) {
                            // Successfully queued for later processing
                            tracing::debug!("Queued post for rate-limited processing");
                        } else {
                            // Queue full, drop the event
                            tracing::warn!(
                                "Processing queue full, dropping post from {}",
                                post_to_filter.handle
                            );
                        }
                        // Also add to regular buffer for history
                        buffer_guard.push(event);
                    }
                } else {
                    // Can send immediately
                    self.tx
                        .send(Ok(event.clone()))
                        .await
                        .inspect_err(|e| tracing::error!("{}", e))?;

                    *last_send = std::time::Instant::now();

                    if let Some(buffer) = &self.buffer {
                        let mut buffer_guard = buffer.lock();
                        buffer_guard.push(event);
                    }
                }

                // Check if we need to save cursor
                let should_save = {
                    let mut events_count = self.events_since_save.lock().unwrap();
                    *events_count += 1;

                    if *events_count >= self.cursor_save_threshold {
                        *events_count = 0;
                        true
                    } else {
                        false
                    }
                };

                let time_based_save = {
                    let last_save = self.last_cursor_save.try_lock();
                    if let Ok(last_save) = last_save {
                        last_save.elapsed() >= self.cursor_save_interval
                    } else {
                        false
                    }
                };

                if should_save || time_based_save {
                    // Save cursor in background
                    let cursor_to_save = cursor.clone();
                    let cursor_file_path = self.cursor_file_path.clone();
                    let last_cursor_save = self.last_cursor_save.clone();

                    tokio::spawn(async move {
                        if let Some(path) = cursor_file_path {
                            let cursor_data = match serde_json::to_string_pretty(&cursor_to_save) {
                                Ok(data) => data,
                                Err(e) => {
                                    tracing::warn!("Failed to serialize cursor: {}", e);
                                    return;
                                }
                            };

                            if let Err(e) = tokio::fs::write(path, cursor_data).await {
                                tracing::warn!("Failed to save cursor: {}", e);
                            } else {
                                // Update last save time
                                let mut last_save = last_cursor_save.lock().await;
                                *last_save = std::time::Instant::now();
                            }
                        }
                    });
                }
            }
        }

        Ok(())
    }
}

fn extract_post_relative_time(post_view: &atrium_api::app::bsky::feed::defs::PostView) -> String {
    let now = Utc::now();
    let time = post_view.indexed_at.as_ref();

    let relative = now - time.to_utc();

    format!("{} ago", format_duration(relative.to_std().unwrap()))
}

/// Extract post record data into our BlueskyPost format
fn extract_post_from_record(
    post: &atrium_api::app::bsky::feed::post::RecordData,
) -> (String, Vec<String>, Vec<String>, Vec<Facet>) {
    let text = post.text.clone();

    // Extract languages
    let langs = post
        .langs
        .as_ref()
        .map(|l| l.iter().map(|lang| lang.as_ref().to_string()).collect())
        .unwrap_or_default();

    // Extract labels
    let labels = post.labels.as_ref().map(label_convert).unwrap_or_default();

    // Extract facets
    let facets = post
        .facets
        .as_ref()
        .map(|f| {
            f.iter()
                .map(|f| Facet {
                    index: ByteSlice {
                        byte_start: f.index.byte_start,
                        byte_end: f.index.byte_end,
                    },
                    features: f.features.iter().filter_map(facet_convert).collect(),
                })
                .collect()
        })
        .unwrap_or_default();

    (text, langs, labels, facets)
}

/// Extract post data from a PostView (for thread display)
fn extract_post_data(
    post_view: &atrium_api::app::bsky::feed::defs::PostView,
) -> Option<(String, Vec<String>, Vec<FacetFeature>, Vec<String>)> {
    if let Ok(post_record) =
        atrium_api::app::bsky::feed::post::RecordData::try_from_unknown(post_view.record.clone())
    {
        let (text, langs, _labels, facets) = extract_post_from_record(&post_record);

        // Flatten facet features for easier access
        let features: Vec<FacetFeature> = facets.into_iter().flat_map(|f| f.features).collect();

        // Extract image alt texts from embed if present
        let alt_texts = extract_image_alt_texts(&post_view.embed);

        Some((text, langs, features, alt_texts))
    } else {
        None
    }
}

fn extract_record_embed(embed: &Union<RecordEmbedRefs>) -> Option<EmbedInfo> {
    match embed {
        Union::Refs(RecordEmbedRefs::AppBskyEmbedExternalMain(external)) => {
            Some(EmbedInfo::External {
                uri: external.external.uri.clone(),
                title: external.external.title.clone(),
                description: external.external.description.clone(),
            })
        }
        Union::Refs(RecordEmbedRefs::AppBskyEmbedImagesMain(images)) => Some(EmbedInfo::Images {
            count: images.images.len(),
            alt_texts: images.images.iter().map(|img| img.alt.clone()).collect(),
        }),
        Union::Refs(RecordEmbedRefs::AppBskyEmbedRecordMain(quoted_record)) => {
            Some(EmbedInfo::Quote {
                uri: quoted_record.record.uri.clone(),
                cid: quoted_record.record.cid.as_ref().to_string(),
                author_handle: String::new(),
                author_display_name: None,
                text: String::new(),
                created_at: None,
            })
        }
        Union::Refs(RecordEmbedRefs::AppBskyEmbedRecordWithMediaMain(_record_media)) => None,
        Union::Refs(RecordEmbedRefs::AppBskyEmbedVideoMain(_embed)) => None,
        Union::Unknown(_unknown_data) => None,
    }
}

async fn extract_embed(
    embed: &Union<atrium_api::app::bsky::feed::defs::PostViewEmbedRefs>,

    bsky_agent: Option<&Arc<bsky_sdk::BskyAgent>>,
) -> Option<EmbedInfo> {
    match embed {
        Union::Refs(PostViewEmbedRefs::AppBskyEmbedImagesView(images_view)) => {
            Some(EmbedInfo::Images {
                count: images_view.images.len(),
                alt_texts: images_view
                    .images
                    .iter()
                    .map(|img| img.alt.clone())
                    .collect(),
            })
        }
        Union::Refs(PostViewEmbedRefs::AppBskyEmbedExternalView(external_view)) => {
            Some(EmbedInfo::External {
                uri: external_view.external.uri.clone(),
                title: external_view.external.title.clone(),
                description: external_view.external.description.clone(),
            })
        }
        Union::Refs(PostViewEmbedRefs::AppBskyEmbedRecordView(record_view)) => {
            match &record_view.record {
                Union::Refs(atrium_api::app::bsky::embed::record::ViewRecordRefs::ViewRecord(
                    view_record,
                )) => {
                    // If we have a bsky_agent, fetch the full quote post
                    if let Some(agent) = bsky_agent {
                        let params = atrium_api::app::bsky::feed::get_posts::ParametersData {
                            uris: vec![view_record.uri.clone()],
                        };

                        if let Ok(posts_result) =
                            agent.api.app.bsky.feed.get_posts(params.into()).await
                        {
                            if let Some(post) = posts_result.data.posts.first() {
                                // Extract full data from the fetched post
                                if let Some((text, _langs, _features, _alt_texts)) =
                                    extract_post_data(post)
                                {
                                    return Some(EmbedInfo::Quote {
                                        uri: view_record.uri.clone(),
                                        cid: view_record.cid.as_ref().to_string(),
                                        author_handle: post.author.handle.as_str().to_string(),
                                        author_display_name: post.author.display_name.clone(),
                                        text,
                                        created_at: Some(
                                            chrono::DateTime::parse_from_rfc3339(
                                                post.indexed_at.as_str(),
                                            )
                                            .ok()
                                            .map(|dt| dt.to_utc())
                                            .unwrap_or_else(Utc::now),
                                        ),
                                    });
                                }
                            }
                        }
                    }

                    // Fallback to using the embedded data
                    if let Ok(quoted_record) =
                        atrium_api::app::bsky::feed::post::RecordData::try_from_unknown(
                            view_record.value.clone(),
                        )
                    {
                        let (quoted_text, _, _, _) = extract_post_from_record(&quoted_record);
                        Some(EmbedInfo::Quote {
                            uri: view_record.uri.clone(),
                            cid: view_record.cid.as_ref().to_string(),
                            author_handle: view_record.author.handle.as_str().to_string(),
                            author_display_name: view_record.author.display_name.clone(),
                            text: quoted_text,
                            created_at: Some(
                                chrono::DateTime::parse_from_rfc3339(
                                    view_record.indexed_at.as_str(),
                                )
                                .ok()
                                .map(|dt| dt.to_utc())
                                .unwrap_or_else(Utc::now),
                            ),
                        })
                    } else {
                        None
                    }
                }
                _ => None,
            }
        }
        Union::Refs(PostViewEmbedRefs::AppBskyEmbedRecordWithMediaView(record_with_media)) => {
            // Extract quote part
            let quote_info = match &record_with_media.record.record {
                Union::Refs(atrium_api::app::bsky::embed::record::ViewRecordRefs::ViewRecord(
                    view_record,
                )) => {
                    if let Ok(quoted_record) =
                        atrium_api::app::bsky::feed::post::RecordData::try_from_unknown(
                            view_record.value.clone(),
                        )
                    {
                        let (quoted_text, _, _, _) = extract_post_from_record(&quoted_record);
                        Some(EmbedInfo::Quote {
                            uri: view_record.uri.clone(),
                            cid: view_record.cid.as_ref().to_string(),
                            author_handle: view_record.author.handle.as_str().to_string(),
                            author_display_name: view_record.author.display_name.clone(),
                            text: quoted_text,
                            created_at: Some(
                                chrono::DateTime::parse_from_rfc3339(
                                    view_record.indexed_at.as_str(),
                                )
                                .ok()
                                .map(|dt| dt.to_utc())
                                .unwrap_or_else(Utc::now),
                            ),
                        })
                    } else {
                        None
                    }
                }
                _ => None,
            };

            // Extract media part
            let media_info = match &record_with_media.media {
                Union::Refs(atrium_api::app::bsky::embed::record_with_media::ViewMediaRefs::AppBskyEmbedImagesView(images)) => {
                    Some(EmbedInfo::Images {
                        count: images.images.len(),
                        alt_texts: images.images.iter().map(|img| img.alt.clone()).collect(),
                    })
                }
                Union::Refs(atrium_api::app::bsky::embed::record_with_media::ViewMediaRefs::AppBskyEmbedExternalView(external)) => {
                    Some(EmbedInfo::External {
                        uri: external.external.uri.clone(),
                        title: external.external.title.clone(),
                        description: external.external.description.clone(),
                    })
                }
                _ => None,
            };

            // Combine both
            match (quote_info, media_info) {
                (Some(quote), Some(media)) => Some(EmbedInfo::QuoteWithMedia {
                    quote: Box::new(quote),
                    media: Box::new(media),
                }),
                (Some(quote), None) => Some(quote),
                (None, Some(media)) => Some(media),
                (None, None) => None,
            }
        }
        _ => None,
    }
}

/// Extract full embed info from a PostView
async fn extract_embed_info(
    post_view: &atrium_api::app::bsky::feed::defs::PostView,
    bsky_agent: Option<&Arc<bsky_sdk::BskyAgent>>,
) -> Option<EmbedInfo> {
    if let Some(embed) = &post_view.embed {
        extract_embed(embed, bsky_agent).await
    } else {
        None
    }
}

/// Info about post embeds
#[derive(Debug, Clone)]
enum EmbedInfo {
    Images {
        count: usize,
        alt_texts: Vec<String>,
    },
    External {
        uri: String,
        title: String,
        description: String,
    },
    Quote {
        uri: String,
        #[allow(dead_code)]
        cid: String,
        author_handle: String,
        author_display_name: Option<String>,
        text: String,
        created_at: Option<DateTime<Utc>>,
    },
    QuoteWithMedia {
        quote: Box<EmbedInfo>,
        media: Box<EmbedInfo>,
    },
}

impl EmbedInfo {
    /// Format embed info for display with given indentation
    fn format_display(&self, indent: &str) -> String {
        let mut output = String::new();

        match self {
            EmbedInfo::Images { count, alt_texts } => {
                output.push_str(&format!("{}   [📸 {} image(s)]\n", indent, count));
                for alt_text in alt_texts {
                    if !alt_text.is_empty() {
                        output.push_str(&format!("{}    alt: {}\n", indent, alt_text));
                    }
                }
            }
            EmbedInfo::External {
                uri,
                title,
                description,
            } => {
                output.push_str(&format!("{}   [🔗 Link Card]\n", indent));
                if !title.is_empty() {
                    output.push_str(&format!("{}    {}\n", indent, title));
                }
                if !description.is_empty() {
                    output.push_str(&format!("{}    {}\n", indent, description));
                }
                output.push_str(&format!("{}    {}\n", indent, uri));
            }
            EmbedInfo::Quote {
                uri,
                author_handle,
                author_display_name,
                text,
                created_at,
                ..
            } => {
                output.push_str(&format!("{}   ┌─ Quote ─────\n", indent));
                let author = if let Some(name) = author_display_name {
                    format!("{} (@{})", name, author_handle)
                } else {
                    format!("@{}", author_handle)
                };
                output.push_str(&format!("{}   │ {}: {}\n", indent, author, text));
                if let Some(time) = created_at {
                    let interval = Utc::now() - *time;
                    let relative =
                        format_duration(interval.to_std().unwrap_or(Duration::from_secs(0)));
                    output.push_str(&format!("{}   │ {} ago\n", indent, relative));
                }
                output.push_str(&format!("{}   │ 🔗 {}\n", indent, uri));
                output.push_str(&format!("{}   └──────────\n", indent));
            }
            EmbedInfo::QuoteWithMedia { quote, media } => {
                output.push_str(&quote.format_display(indent));
                output.push_str(&media.format_display(indent));
            }
        }

        output
    }
}

/// Extract alt texts from post embed
fn extract_image_alt_texts(
    embed: &Option<Union<atrium_api::app::bsky::feed::defs::PostViewEmbedRefs>>,
) -> Vec<String> {
    use atrium_api::app::bsky::feed::defs::PostViewEmbedRefs;

    if let Some(embed) = embed {
        match embed {
            Union::Refs(PostViewEmbedRefs::AppBskyEmbedImagesView(images_view)) => images_view
                .images
                .iter()
                .map(|img| img.alt.clone())
                .collect(),
            Union::Refs(PostViewEmbedRefs::AppBskyEmbedRecordWithMediaView(record_with_media)) => {
                // Images can be in the media part of record with media
                match &record_with_media.media {
                    Union::Refs(atrium_api::app::bsky::embed::record_with_media::ViewMediaRefs::AppBskyEmbedImagesView(images)) => {
                        images.images.iter()
                            .map(|img| img.alt.clone())
                            .collect()
                    }
                    _ => vec![]
                }
            }
            _ => vec![],
        }
    } else {
        vec![]
    }
}

/// Convert a thread post to a reply candidate tuple
fn thread_post_to_candidate(
    post: &atrium_api::app::bsky::feed::defs::PostView,
) -> (String, String) {
    (
        post.uri.clone(),
        format!("@{}", post.author.handle.as_str()),
    )
}

/// Create a basic memory content string for a user
fn create_basic_memory_content(handle: &str, did: &str) -> String {
    format!(
        "Bluesky user @{} (DID: {})\nFirst seen: {}\n",
        handle,
        did,
        chrono::Utc::now().format("%Y-%m-%d %H:%M:%S UTC")
    )
}

fn label_convert(l: &Union<RecordLabelsRefs>) -> Vec<String> {
    match l {
        atrium_api::types::Union::Refs(
            atrium_api::app::bsky::feed::post::RecordLabelsRefs::ComAtprotoLabelDefsSelfLabels(l),
        ) => l.values.iter().map(|l| l.val.clone()).collect(),
        atrium_api::types::Union::Unknown(unknown_data) => unknown_data
            .data
            .iter()
            .map(|l| format!("{:?}", l))
            .collect(),
    }
}

fn facet_convert(f: &Union<MainFeaturesItem>) -> Option<FacetFeature> {
    match f {
        atrium_api::types::Union::Refs(f) => match f {
            MainFeaturesItem::Mention(object) => Some(FacetFeature::Mention {
                did: object.did.to_string(),
            }),
            MainFeaturesItem::Link(object) => Some(FacetFeature::Link {
                uri: object.uri.clone(),
            }),
            MainFeaturesItem::Tag(object) => Some(FacetFeature::Tag {
                tag: object.tag.clone(),
            }),
        },
        atrium_api::types::Union::Unknown(_) => None,
    }
}

async fn should_include_post(
    post: &mut BlueskyPost,
    filter: &BlueskyFilter,
    resolver: &atrium_identity::did::CommonDidResolver<PatternHttpClient>,
) -> bool {
    use atrium_common::resolver::Resolver;

    // 1. EXCLUSIONS FIRST (highest priority)

    // Exclude DIDs - never show posts from these
    if filter.exclude_dids.contains(&post.did) {
        return false;
    }

    // Exclude keywords - filter out posts containing these
    if !filter.exclude_keywords.is_empty() {
        let text_lower = post.text.to_lowercase();
        if filter
            .exclude_keywords
            .iter()
            .any(|kw| text_lower.contains(&kw.to_lowercase()))
        {
            return false;
        }
    }

    // 2. REPLIES TO SELF - always see replies to our own posts
    if let Some(reply) = &post.reply {
        // Extract agent's DID from mentions field (should be the only entry)
        if let Some(agent_did) = filter.mentions.first() {
            // Check if this is a reply to the agent's own post
            if reply.parent.uri.contains(agent_did) {
                // This is a reply to the agent - always include it
                post.handle = resolver
                    .resolve(&Did::from_str(&post.did).expect("valid did"))
                    .await
                    .ok()
                    .map(|doc| {
                        let handle = doc
                            .also_known_as
                            .expect("proper did doc should have an alias in it")
                            .first()
                            .expect("proper did doc should have an alias in it")
                            .clone();

                        handle.strip_prefix("at://").unwrap_or(&handle).to_string()
                    })
                    .unwrap_or(post.did.clone());

                return true;
            }
        }
    }

    // 3. FRIENDS LIST - bypass all other checks
    if filter.friends.contains(&post.did) {
        // Friends always pass through
        // Still need to resolve handle though
        post.handle = resolver
            .resolve(&Did::from_str(&post.did).expect("valid did"))
            .await
            .ok()
            .map(|doc| {
                let handle = doc
                    .also_known_as
                    .expect("proper did doc should have an alias in it")
                    .first()
                    .expect("proper did doc should have an alias in it")
                    .clone();

                handle.strip_prefix("at://").unwrap_or(&handle).to_string()
            })
            .unwrap_or(post.did.clone());

        return true;
    }

    // 4. CHECK MENTIONS
    if !filter.mentions.is_empty() {
        let mentioned = post.mentioned_dids();
        let has_required_mention = filter
            .mentions
            .iter()
            .any(|allowed_did| mentioned.contains(&allowed_did.as_str()));

        if has_required_mention {
            // Has required mention - check if author is allowed
            if filter.allow_any_mentions {
                // Accept mentions from anyone
                post.handle = resolver
                    .resolve(&Did::from_str(&post.did).expect("valid did"))
                    .await
                    .ok()
                    .map(|doc| {
                        let handle = doc
                            .also_known_as
                            .expect("proper did doc should have an alias in it")
                            .first()
                            .expect("proper did doc should have an alias in it")
                            .clone();

                        handle.strip_prefix("at://").unwrap_or(&handle).to_string()
                    })
                    .unwrap_or(post.did.clone());

                return true;
            } else if filter.dids.is_empty() || filter.dids.contains(&post.did) {
                // Only accept mentions from allowlisted DIDs (or if no allowlist)
                post.handle = resolver
                    .resolve(&Did::from_str(&post.did).expect("valid did"))
                    .await
                    .ok()
                    .map(|doc| {
                        let handle = doc
                            .also_known_as
                            .expect("proper did doc should have an alias in it")
                            .first()
                            .expect("proper did doc should have an alias in it")
                            .clone();

                        handle.strip_prefix("at://").unwrap_or(&handle).to_string()
                    })
                    .unwrap_or(post.did.clone());

                return true;
            }
        }
    }

    // 5. CHECK REGULAR ALLOWLIST
    // DID filter - only from specific authors
    if !filter.dids.is_empty() && !filter.dids.contains(&post.did) {
        return false;
    }

    // 6. APPLY REMAINING FILTERS (keywords, languages)

    // Keyword filter
    if !filter.keywords.is_empty() {
        let text_lower = post.text.to_lowercase();
        if !filter
            .keywords
            .iter()
            .any(|kw| text_lower.contains(&kw.to_lowercase()))
        {
            return false;
        }
    }

    // Language filter
    if !filter.languages.is_empty()
        && !post
            .langs
            .iter()
            .any(|lang| filter.languages.contains(lang))
    {
        return false;
    }

    post.handle = resolver
        .resolve(&Did::from_str(&post.did).expect("valid did"))
        .await
        .ok()
        .map(|doc| {
            let handle = doc
                .also_known_as
                .expect("proper did doc should have an alias in it")
                .first()
                .expect("proper did doc should have an alias in it")
                .clone();

            handle.strip_prefix("at://").unwrap_or(&handle).to_string()
        })
        .unwrap_or(post.did.clone());

    true
}

impl Searchable for BlueskyPost {
    fn matches(&self, query: &str) -> bool {
        let query_lower = query.to_lowercase();

        // Search in text
        if self.text.to_lowercase().contains(&query_lower) {
            return true;
        }

        // Search in handle
        if self.handle.to_lowercase().contains(&query_lower) {
            return true;
        }

        // Search in hashtags
        for facet in &self.facets {
            for feature in &facet.features {
                if let FacetFeature::Tag { tag } = feature {
                    if tag.to_lowercase().contains(&query_lower) {
                        return true;
                    }
                }
            }
        }

        // Search in alt text
        for alt in self.image_alt_texts() {
            if alt.to_lowercase().contains(&query_lower) {
                return true;
            }
        }

        // Search in quoted post
        if let Some(quoted) = self.quoted_post() {
            if quoted.text.to_lowercase().contains(&query_lower) {
                return true;
            }
            if quoted.handle.to_lowercase().contains(&query_lower) {
                return true;
            }
            if let Some(display_name) = &quoted.display_name {
                if display_name.to_lowercase().contains(&query_lower) {
                    return true;
                }
            }
        }

        false
    }

    fn relevance(&self, query: &str) -> f32 {
        if !self.matches(query) {
            return 0.0;
        }

        let query_lower = query.to_lowercase();
        let mut score = 0.0;

        // Exact match in text gets highest score
        if self.text.to_lowercase() == query_lower {
            score += 5.0;
        } else if self.text.to_lowercase().contains(&query_lower) {
            // Count occurrences
            let count = self.text.to_lowercase().matches(&query_lower).count() as f32;
            score += 1.0 + (count * 0.2);
        }

        // Handle match
        if self.handle.to_lowercase().contains(&query_lower) {
            score += 2.0;
        }

        // Hashtag match
        for facet in &self.facets {
            for feature in &facet.features {
                if let FacetFeature::Tag { tag } = feature {
                    if tag.to_lowercase() == query_lower {
                        score += 3.0; // Exact hashtag match is very relevant
                    } else if tag.to_lowercase().contains(&query_lower) {
                        score += 1.5;
                    }
                }
            }
        }

        // Quoted post match
        if let Some(quoted) = self.quoted_post() {
            if quoted.text.to_lowercase().contains(&query_lower) {
                score += 1.0; // Match in quoted text is somewhat relevant
            }
            if quoted.handle.to_lowercase().contains(&query_lower) {
                score += 1.5; // Handle match in quote is more relevant
            }
        }

        // Normalize to 0.0-1.0 range (max theoretical score ~10)
        (score / 10.0).min(1.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_filter_post() {
        let mut post = BlueskyPost {
            display_name: None,
            uri: "at://did:plc:example/app.bsky.feed.post/123".to_string(),
            cid: Cid::from_str("bafyreieqropzcxn6nztojzr5z42u4furcrgfqppyt5e4p43vuzmbi7xdfu")
                .unwrap(),
            did: "did:plc:example".to_string(),
            handle: "test.bsky.social".to_string(),
            text: "Hello world from Rust!".to_string(),
            created_at: Utc::now(),
            reply: None,
            embed: None,
            langs: vec!["en".to_string()],
            labels: vec![],
            facets: vec![],
        };

        let resolver = atproto_identity_resolver();
        // Test keyword filter
        let filter = BlueskyFilter {
            keywords: vec!["rust".to_string()],
            ..Default::default()
        };
        assert!(should_include_post(&mut post, &filter, &resolver).await);

        // Test language filter
        let filter = BlueskyFilter {
            languages: vec!["en".to_string()],
            ..Default::default()
        };
        assert!(should_include_post(&mut post, &filter, &resolver).await);

        // Test DID filter
        let filter = BlueskyFilter {
            dids: vec!["did:plc:other".to_string()],
            ..Default::default()
        };
        assert!(!should_include_post(&mut post, &filter, &resolver).await);
    }

    #[tokio::test]
    async fn test_mention_filter() {
        let mut post = BlueskyPost {
            display_name: None,
            uri: "at://did:plc:author/app.bsky.feed.post/456".to_string(),
            did: "did:plc:author".to_string(),
            cid: Cid::from_str("bafyreieqropzcxn6nztojzr5z42u4furcrgfqppyt5e4p43vuzmbi7xdfu")
                .unwrap(),
            handle: "author.bsky.social".to_string(),
            text: "Hey @alice.bsky.social check this out!".to_string(),
            created_at: Utc::now(),
            reply: None,
            embed: None,
            langs: vec!["en".to_string()],
            labels: vec![],
            facets: vec![],
        };

        let resolver = atproto_identity_resolver();

        // Add mention facet
        post.facets.push(Facet {
            index: ByteSlice {
                byte_start: 4,
                byte_end: 22,
            },
            features: vec![FacetFeature::Mention {
                did: "did:plc:alice".to_string(),
            }],
        });

        // Test with mention whitelist - should include
        let filter = BlueskyFilter {
            mentions: vec!["did:plc:alice".to_string()],
            ..Default::default()
        };
        assert!(should_include_post(&mut post, &filter, &resolver).await);

        // Test with different mention whitelist - behavior changed, now includes
        let filter = BlueskyFilter {
            mentions: vec!["did:plc:bob".to_string()],
            ..Default::default()
        };
        assert!(should_include_post(&mut post, &filter, &resolver).await);

        // Test mentions() helper
        assert!(post.mentions("did:plc:alice"));
        assert!(!post.mentions("did:plc:bob"));
    }
}
