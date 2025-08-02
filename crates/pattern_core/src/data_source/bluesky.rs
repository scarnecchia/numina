use std::str::FromStr;
use std::sync::Arc;
use std::{collections::HashMap, sync::Mutex};

use super::{
    StreamBuffer,
    traits::{DataSource, DataSourceMetadata, DataSourceStatus, Searchable, StreamEvent},
};
use crate::context::AgentHandle;
use crate::error::Result;
use async_trait::async_trait;
use atrium_api::app::bsky::feed::post::{RecordLabelsRefs, ReplyRefData};
use atrium_api::app::bsky::richtext::facet::MainFeaturesItem;
use atrium_api::com::atproto::repo::strong_ref::MainData;
use atrium_api::types::string::{Cid, Did};
use atrium_api::types::{TryFromUnknown, Union};
use atrium_identity::did::{CommonDidResolver, CommonDidResolverConfig, DEFAULT_PLC_DIRECTORY_URL};

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

pub struct PatternHttpClient {
    pub client: reqwest::Client,
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
            client: reqwest::Client::new(),
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
    pub handle: String, // Author handle
    pub text: String,
    pub created_at: DateTime<Utc>,
    pub reply: Option<ReplyRef>, // Full reply reference with root and parent
    pub embed: Option<serde_json::Value>,
    pub langs: Vec<String>,
    pub labels: Vec<String>,
    pub facets: Vec<Facet>, // Rich text annotations (mentions, links, hashtags)
}

/// Reply reference - alias for atrium type
pub type ReplyRef = ReplyRefData;

/// Post reference - alias for atrium type
pub type PostRef = MainData;

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
        self.facets
            .iter()
            .flat_map(|facet| &facet.features)
            .filter_map(|feature| match feature {
                FacetFeature::Mention { did } => Some(did.as_str()),
                _ => None,
            })
            .collect()
    }

    /// Check if post has images
    pub fn has_images(&self) -> bool {
        self.embed
            .as_ref()
            .and_then(|e| e.get("$type"))
            .and_then(|t| t.as_str())
            .map(|t| t == "app.bsky.embed.images")
            .unwrap_or(false)
    }

    /// Extract alt text from image embeds (for accessibility)
    pub fn image_alt_texts(&self) -> Vec<String> {
        self.embed
            .as_ref()
            .and_then(|e| e.get("images"))
            .and_then(|imgs| imgs.as_array())
            .map(|images| {
                images
                    .iter()
                    .filter_map(|img| img.get("alt"))
                    .filter_map(|alt| alt.as_str())
                    .map(|s| s.to_string())
                    .collect()
            })
            .unwrap_or_default()
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
    bsky_agent: Option<Arc<tokio::sync::RwLock<bsky_sdk::BskyAgent>>>,
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
            .finish()
    }
}

impl BlueskyFirehoseSource {
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

        self.bsky_agent = Some(Arc::new(tokio::sync::RwLock::new(agent)));
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
        // Apply collection filters (NSIDs map to collections)
        let collections = if !self.filter.nsids.is_empty() {
            self.filter.nsids.clone()
        } else {
            vec!["app.bsky.feed.post".to_string()]
        };

        // Build options with all settings
        let options = if let Some(ref cursor) = from {
            JetstreamOptions::builder()
                .cursor(cursor.time_us.to_string())
                .wanted_collections(collections)
                .build()
        } else {
            JetstreamOptions::builder()
                .wanted_collections(collections)
                .build()
        };

        // Create connection - new() is sync
        let connection = JetstreamConnection::new(options);

        // Create channel for processed events
        let (tx, rx) = tokio::sync::mpsc::channel(5000);
        let filter = self.filter.clone();
        let source_id = self.source_id.clone();
        let buffer = self.buffer.clone();

        // Create our ingestor that sends posts to our channel
        let post_ingestor = PostIngestor {
            tx: tx.clone(),
            filter,
            buffer,
            resolver: atproto_identity_resolver(),
        };

        let mut ingestors: HashMap<String, Box<dyn LexiconIngestor + Send + Sync>> = HashMap::new();
        ingestors.insert("app.bsky.feed.post".to_string(), Box::new(post_ingestor));

        // tracks the last message we've processed
        let cursor: Arc<Mutex<Option<u64>>> = Arc::new(Mutex::new(from.map(|c| c.time_us)));

        let msg_rx = connection.get_msg_rx();
        let reconnect_tx = connection.get_reconnect_tx();

        let c_cursor = cursor.clone();
        let c_source_id = source_id.clone();
        let ingestor_tx = tx.clone();
        // Spawn task to consume events
        let handle = tokio::spawn(async move {
            // Process messages from jetstream
            while let Ok(message) = msg_rx.recv_async().await {
                if let Err(e) = handler::handle_message(
                    message,
                    &ingestors,
                    reconnect_tx.clone(),
                    c_cursor.clone(),
                )
                .await
                {
                    tracing::warn!("Error processing message: {}", e);
                    let _ = ingestor_tx.send(Err(crate::CoreError::DataSourceError {
                        source_name: c_source_id.clone(),
                        operation: "process".to_string(),
                        cause: e.to_string(),
                    }));
                };
            }
        });

        // Spawn connect in its own task so it doesn't block
        let connect_source_id = source_id.clone();
        let connect_tx = tx.clone();
        tokio::spawn(async move {
            if let Err(e) = connection.connect(cursor.clone()).await {
                let error_msg = e.to_string();
                tracing::error!("Jetstream connection error: {}", error_msg);
                // Use try_send to avoid await point while holding non-Send error
                let _ = connect_tx.try_send(Err(crate::CoreError::DataSourceError {
                    source_name: connect_source_id,
                    operation: "connect".to_string(),
                    cause: error_msg,
                }));
            }
            handle.abort();
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

    async fn format_notification(&self, item: &Self::Item) -> Option<String> {
        // Format based on post type
        let mut message = String::new();
        let mut reply_candidates = Vec::new();

        // Header with author
        message.push_str(&format!("💬 @{}", item.handle));

        // Add context for replies/mentions
        if let Some(reply) = &item.reply {
            message.push_str(" replied");

            // If we have a BskyAgent, try to fetch thread context
            if let Some(bsky_agent) = &self.bsky_agent {
                if let Ok(agent) = bsky_agent.try_read() {
                    // Try to fetch the parent post for context
                    if let Ok(thread_result) = agent
                        .api
                        .app
                        .bsky
                        .feed
                        .get_post_thread(
                            atrium_api::app::bsky::feed::get_post_thread::ParametersData {
                                uri: reply.parent.uri.clone(),
                                depth: None, // Use default depth
                                parent_height: None,
                            }
                            .into(),
                        )
                        .await
                    {
                        // Extract thread context
                        use atrium_api::app::bsky::feed::get_post_thread::OutputThreadRefs;
                        if let Union::Refs(OutputThreadRefs::AppBskyFeedDefsThreadViewPost(
                            thread_view,
                        )) = &thread_result.thread
                        {
                            message.push_str(" in thread:\n\n");

                            // Walk up the parent chain to collect thread context
                            let mut current_thread = thread_view;
                            let mut thread_posts = Vec::new();
                            let mut depth = 0;

                            // Collect the immediate parent and up to 4 posts up the chain
                            loop {
                                if let Some((text, langs, features, alt_texts)) =
                                    extract_post_data(&current_thread.post)
                                {
                                    let handle =
                                        current_thread.post.author.handle.as_str().to_string();
                                    let mentions: Vec<_> = features
                                        .iter()
                                        .filter_map(|f| match f {
                                            FacetFeature::Mention { did } => {
                                                Some(format!("@{}", did))
                                            }
                                            _ => None,
                                        })
                                        .collect();

                                    thread_posts.push((
                                        handle,
                                        text,
                                        current_thread.post.uri.clone(),
                                        depth,
                                        mentions,
                                        langs,
                                        alt_texts,
                                    ));

                                    // Add as reply candidate
                                    reply_candidates
                                        .push(thread_post_to_candidate(&current_thread.post));
                                }

                                depth += 1;
                                if depth >= 4 {
                                    break;
                                }

                                // Check if there's a parent
                                if let Some(parent) = &current_thread.parent {
                                    use atrium_api::app::bsky::feed::defs::ThreadViewPostParentRefs;
                                    if let Union::Refs(ThreadViewPostParentRefs::ThreadViewPost(
                                        parent_thread,
                                    )) = parent
                                    {
                                        current_thread = parent_thread;
                                    } else {
                                        break;
                                    }
                                } else {
                                    break;
                                }
                            }

                            // Display thread posts in reverse order (root to leaf)
                            for (handle, text, _uri, depth, mentions, langs, alt_texts) in
                                thread_posts.iter().rev()
                            {
                                let indent = "  ".repeat(*depth);
                                let bullet = if *depth == 0 { "•" } else { "└─" };

                                message.push_str(&format!(
                                    "{}{} @{}: {}\n",
                                    indent, bullet, handle, text
                                ));

                                // Show mentions if any
                                if !mentions.is_empty() {
                                    message.push_str(&format!(
                                        "{}   [mentions: {}]\n",
                                        indent,
                                        mentions.join(", ")
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

                                // Show images if any
                                if !alt_texts.is_empty() {
                                    message.push_str(&format!(
                                        "{}   [📸 {} image(s)]\n",
                                        indent,
                                        alt_texts.len()
                                    ));
                                }
                            }

                            // Mark the main post clearly
                            message.push_str("\n>>> MAIN POST >>>\n");
                        }
                    }
                }
            }
        } else if item.mentions(&self.source_id) {
            message.push_str(" mentioned you");
        }

        message.push_str(":\n");

        // Full post text
        message.push_str(&format!("@{}: {}", item.handle, item.text));

        // Add image indicator if present
        if item.has_images() {
            let alt_texts = item.image_alt_texts();
            if !alt_texts.is_empty() {
                message.push_str(&format!("\n[📸 {} image(s)]", alt_texts.len()));
            }
        }

        // Add link
        message.push_str(&format!("\n🔗 {}", item.uri));

        // Show replies after main post if we're in a thread
        if let Some(reply) = &item.reply {
            if let Some(bsky_agent) = &self.bsky_agent {
                if let Ok(agent) = bsky_agent.try_read() {
                    if let Ok(thread_result) = agent
                        .api
                        .app
                        .bsky
                        .feed
                        .get_post_thread(
                            atrium_api::app::bsky::feed::get_post_thread::ParametersData {
                                uri: reply.parent.uri.clone(),
                                depth: None,
                                parent_height: None,
                            }
                            .into(),
                        )
                        .await
                    {
                        use atrium_api::app::bsky::feed::get_post_thread::OutputThreadRefs;
                        if let Union::Refs(OutputThreadRefs::AppBskyFeedDefsThreadViewPost(
                            thread_view,
                        )) = &thread_result.thread
                        {
                            // Add some reply context
                            if let Some(replies) = &thread_view.replies {
                                message.push_str("\n<<< REPLIES <<<\n");
                                use atrium_api::app::bsky::feed::defs::ThreadViewPostRepliesItem;
                                for reply in replies.iter().take(2) {
                                    if let Union::Refs(ThreadViewPostRepliesItem::ThreadViewPost(
                                        reply_thread,
                                    )) = reply
                                    {
                                        if let Some((reply_text, _langs, _features, _alt_texts)) =
                                            extract_post_data(&reply_thread.post)
                                        {
                                            message.push_str(&format!(
                                                "  └─ @{}: {}\n",
                                                reply_thread.post.author.handle.as_str(),
                                                reply_text
                                            ));

                                            // Add as reply candidate
                                            reply_candidates
                                                .push(thread_post_to_candidate(&reply_thread.post));
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        // Check for memory blocks if we have an agent handle
        if let Some(agent_handle) = &self.agent_handle {
            // Look for or create memory block for this user
            let memory_label = format!("bluesky_user_{}", item.handle);
            if let Ok(memories) = agent_handle.search_archival_memories(&item.handle, 1).await {
                if memories.is_empty() {
                    // Fetch user profile if we have a BskyAgent
                    let memory_content = if let Some(bsky_agent) = &self.bsky_agent {
                        if let Ok(agent) = bsky_agent.try_read() {
                            Self::fetch_user_profile_for_memory(&*agent, &item.handle, &item.did)
                                .await
                        } else {
                            create_basic_memory_content(&item.handle, &item.did)
                        }
                    } else {
                        create_basic_memory_content(&item.handle, &item.did)
                    };

                    if let Err(e) = agent_handle
                        .insert_archival_memory(&memory_label, &memory_content)
                        .await
                    {
                        tracing::warn!("Failed to create memory block for {}: {}", item.handle, e);
                    } else {
                        message.push_str(&format!("\n\n📝 Memory created: {}", memory_label));
                    }
                } else {
                    message.push_str(&format!("\n\n📝 Memory exists: {}", memory_label));
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
            .push_str("If you choose to reply, your response must contain under 300 characters or it will be truncated.\n");

        Some(message)
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

/// Ingestor that processes Bluesky posts and sends them to our channel
struct PostIngestor {
    tx: tokio::sync::mpsc::Sender<Result<StreamEvent<BlueskyPost, BlueskyFirehoseCursor>>>,
    filter: BlueskyFilter,
    #[allow(unused)]
    buffer: Option<Arc<parking_lot::Mutex<StreamBuffer<BlueskyPost, BlueskyFirehoseCursor>>>>,
    resolver: atrium_identity::did::CommonDidResolver<PatternHttpClient>,
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
                did: event.did.to_string(),
                cid: rcid,
                handle: event.did.to_string(), // temporary, need to do handle resolution
                text,
                created_at: chrono::DateTime::parse_from_rfc3339(post.created_at.as_str())
                    .expect("incorrect time format")
                    .to_utc(),
                reply: post.reply.map(|r| r.data),
                embed: post
                    .embed
                    .map(|e| serde_json::to_value(e).expect("should be reasonably serializable")),
                langs,
                labels,
                facets,
            };

            if should_include_post(&mut post_to_filter, &self.filter, &self.resolver).await {
                let event = StreamEvent {
                    item: post_to_filter.clone(),
                    cursor: BlueskyFirehoseCursor {
                        seq: now.timestamp_micros() as u64,
                        time_us: now.timestamp_micros() as u64,
                    },
                    timestamp: now,
                };
                self.tx
                    .send(Ok(event.clone()))
                    .await
                    .inspect_err(|e| tracing::error!("{}", e))?;
                if let Some(buffer) = &self.buffer {
                    let mut buffer_guard = buffer.lock();
                    buffer_guard.push(event);
                }
            }
        }

        Ok(())
    }
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
    // DID filter - only from specific authors
    if !filter.dids.is_empty() && !filter.dids.contains(&post.did) {
        return false;
    }

    // Mention filter - if set, only include posts that mention whitelisted DIDs/handles
    if !filter.mentions.is_empty() {
        let mentioned = post.mentioned_dids();
        if !filter
            .mentions
            .iter()
            .any(|allowed_did| mentioned.contains(&allowed_did.as_str()))
        {
            return false;
        }
    }

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

        // Test with different mention whitelist - should exclude
        let filter = BlueskyFilter {
            mentions: vec!["did:plc:bob".to_string()],
            ..Default::default()
        };
        assert!(!should_include_post(&mut post, &filter, &resolver).await);

        // Test mentions() helper
        assert!(post.mentions("did:plc:alice"));
        assert!(!post.mentions("did:plc:bob"));
    }
}
