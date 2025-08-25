use std::str::FromStr;
use std::sync::Arc;
use std::time::{Duration, Instant};
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
use atrium_common::resolver::Resolver;
use atrium_identity::did::{CommonDidResolver, CommonDidResolverConfig, DEFAULT_PLC_DIRECTORY_URL};
use compact_str::CompactString;
use dashmap::DashMap;

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
    pub parent: Option<BlueskyPost>,
    /// Direct siblings (other replies to the same parent)
    pub siblings: Vec<BlueskyPost>,
    /// Map of post URI to its direct replies
    pub replies_map: std::collections::HashMap<String, Vec<BlueskyPost>>,
    /// Engagement metrics for posts (likes, replies, reposts)
    pub engagement_map: std::collections::HashMap<String, PostEngagement>,
    /// Agent's interactions with posts
    pub agent_interactions: std::collections::HashMap<String, AgentInteraction>,
}

impl ThreadContext {
    /// Append full thread tree to buffer
    pub fn append_thread_tree(
        &self,
        buf: &mut String,
        main_post: &BlueskyPost,
        agent_did: Option<&str>,
    ) {
        // If there's a parent, show it first
        if let Some(parent) = &self.parent {
            buf.push_str("  • 💬 Thread context:\n\n");
            parent.append_as_parent(buf, agent_did, "        ");
        }

        // Show siblings (other replies to the parent)
        if !self.siblings.is_empty() {
            for (i, sibling) in self.siblings.iter().enumerate() {
                let is_last = i == self.siblings.len() - 1;
                sibling.append_as_sibling(buf, agent_did, "      ", is_last);

                // Show replies to this sibling
                if let Some(replies) = self.replies_map.get(&sibling.uri) {
                    for reply in replies {
                        let indent = if is_last { "      " } else { "      │" };
                        reply.append_as_reply(buf, agent_did, indent, 1);
                    }
                }
            }
        }

        // Show the main post
        main_post.append_as_main(buf, agent_did);

        // Show replies to the main post
        if let Some(main_replies) = self.replies_map.get(&main_post.uri) {
            for reply in main_replies {
                reply.append_as_reply(buf, agent_did, "  ", 1);
            }
        }
    }

    /// Format reply options for the agent
    pub fn format_reply_options(&self, buf: &mut String, main_post: &BlueskyPost) {
        buf.push_str("\n💭 Reply options (choose at most one):\n");

        // Add parent as an option
        if let Some(parent) = &self.parent {
            buf.push_str(&format!("  • @{} ({})\n", parent.handle, parent.uri));
        }

        // Add siblings as options
        for sibling in &self.siblings {
            buf.push_str(&format!("  • @{} ({})\n", sibling.handle, sibling.uri));
        }

        // Add main post as option
        buf.push_str(&format!("  • @{} ({})\n", main_post.handle, main_post.uri));

        // Add replies as options
        for replies in self.replies_map.values() {
            for reply in replies {
                buf.push_str(&format!("  • @{} ({})\n", reply.handle, reply.uri));
            }
        }

        buf.push_str("If you choose to reply (by using send_message with target_type bluesky and the target_id set to the uri of the post you want to reply to, from the above options), your response must contain under 300 characters or it will be truncated.\n");
        buf.push_str("Alternatively, you can 'like' the post by submitting a reply with 'like' as the sole text\n");
    }
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

/// Hydration state for tracking what data needs fetching
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub struct HydrationState {
    pub handle_resolved: bool,
    pub profile_fetched: bool,
    pub embed_enriched: bool,
}

impl Default for HydrationState {
    fn default() -> Self {
        Self {
            handle_resolved: false,
            profile_fetched: false,
            embed_enriched: false,
        }
    }
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
    pub embed: Option<EmbedInfo>, // Cleaned up embed representation
    pub langs: Vec<String>,
    pub labels: Vec<String>,
    pub facets: Vec<Facet>, // Rich text annotations (mentions, links, hashtags)
    pub indexed_at: Option<DateTime<Utc>>, // When Bluesky indexed it
    #[serde(default)]
    pub hydration: HydrationState, // Tracks what data has been fetched
    // Engagement metrics (from PostView)
    pub like_count: Option<i64>,
    pub reply_count: Option<i64>,
    pub repost_count: Option<i64>,
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
    /// Get the thread root URI for this post
    pub fn thread_root(&self) -> String {
        if let Some(reply) = &self.reply {
            // If this is a reply, return the root of the thread
            reply.root.uri.clone()
        } else {
            // This is a root post itself
            self.uri.clone()
        }
    }

    /// Create from a firehose record (minimal data)
    /// Note: CID must be provided from the firehose event since it's not in the record
    /// Handle is left empty initially - call resolve_handle() to populate it
    pub fn from_record(
        record: &atrium_api::app::bsky::feed::post::RecordData,
        uri: String,
        did: String,
        cid: Cid,
    ) -> Self {
        // Extract facets directly from the record
        let facets = record.facets.as_ref().map_or(Vec::new(), |facets| {
            facets
                .iter()
                .map(|f| Facet {
                    index: ByteSlice {
                        byte_start: f.index.byte_start as usize,
                        byte_end: f.index.byte_end as usize,
                    },
                    features: f
                        .features
                        .iter()
                        .filter_map(|feat| {
                            use atrium_api::app::bsky::richtext::facet::MainFeaturesItem;
                            match feat {
                                Union::Refs(MainFeaturesItem::Mention(mention)) => {
                                    Some(FacetFeature::Mention {
                                        did: mention.did.as_str().to_string(),
                                    })
                                }
                                Union::Refs(MainFeaturesItem::Link(link)) => {
                                    Some(FacetFeature::Link {
                                        uri: link.uri.clone(),
                                    })
                                }
                                Union::Refs(MainFeaturesItem::Tag(tag)) => {
                                    Some(FacetFeature::Tag {
                                        tag: tag.tag.clone(),
                                    })
                                }
                                _ => None,
                            }
                        })
                        .collect(),
                })
                .collect()
        });

        // Extract labels from self-labels if present
        let labels = record
            .labels
            .as_ref()
            .map_or(Vec::new(), |label_refs| match label_refs {
                Union::Refs(RecordLabelsRefs::ComAtprotoLabelDefsSelfLabels(self_labels)) => {
                    self_labels.values.iter().map(|v| v.val.clone()).collect()
                }
                _ => Vec::new(),
            });

        // Convert embed to EmbedInfo - note this will be minimal without hydration
        // Record embeds only have URIs/CIDs, not author info or image URLs
        let embed = record.embed.as_ref().and_then(|embed_union| {
            match embed_union {
                Union::Refs(RecordEmbedRefs::AppBskyEmbedExternalMain(external)) => {
                    Some(EmbedInfo::External {
                        uri: external.external.uri.clone(),
                        title: external.external.title.clone(),
                        description: external.external.description.clone(),
                    })
                }
                Union::Refs(RecordEmbedRefs::AppBskyEmbedImagesMain(images)) => {
                    Some(EmbedInfo::Images {
                        count: images.images.len(),
                        alt_texts: images.images.iter().map(|img| img.alt.clone()).collect(),
                        urls: vec![], // No URLs available from record - need hydration
                    })
                }
                Union::Refs(RecordEmbedRefs::AppBskyEmbedRecordMain(quoted_record)) => {
                    // Quote post - we only have URI/CID, no author info or content
                    Some(EmbedInfo::Quote {
                        uri: quoted_record.record.uri.clone(),
                        cid: quoted_record.record.cid.as_ref().to_string(),
                        author_handle: String::new(), // Needs hydration
                        author_display_name: None,    // Needs hydration
                        text: String::new(),          // Needs hydration
                        created_at: None,             // Needs hydration
                    })
                }
                Union::Refs(RecordEmbedRefs::AppBskyEmbedRecordWithMediaMain(
                    record_with_media,
                )) => {
                    // Quote with media - extract both
                    use atrium_api::app::bsky::embed::record_with_media::MainMediaRefs;

                    // The record field is a direct Object, not a Union
                    let quote = Box::new(EmbedInfo::Quote {
                        uri: record_with_media.record.data.record.uri.clone(),
                        cid: record_with_media
                            .record
                            .data
                            .record
                            .cid
                            .as_ref()
                            .to_string(),
                        author_handle: String::new(), // Needs hydration
                        author_display_name: None,    // Needs hydration
                        text: String::new(),          // Needs hydration
                        created_at: None,             // Needs hydration
                    });

                    let media = match &record_with_media.media {
                        Union::Refs(MainMediaRefs::AppBskyEmbedImagesMain(images)) => {
                            Box::new(EmbedInfo::Images {
                                count: images.images.len(),
                                alt_texts: images
                                    .images
                                    .iter()
                                    .map(|img| img.alt.clone())
                                    .collect(),
                                urls: vec![], // No URLs available from record
                            })
                        }
                        Union::Refs(MainMediaRefs::AppBskyEmbedExternalMain(external)) => {
                            Box::new(EmbedInfo::External {
                                uri: external.external.uri.clone(),
                                title: external.external.title.clone(),
                                description: external.external.description.clone(),
                            })
                        }
                        _ => return None,
                    };

                    Some(EmbedInfo::QuoteWithMedia { quote, media })
                }
                Union::Refs(RecordEmbedRefs::AppBskyEmbedVideoMain(_video)) => {
                    // Video embeds not yet supported
                    None
                }
                _ => None,
            }
        });

        Self {
            uri,
            did: did.clone(),
            cid,
            handle: String::new(), // Empty initially - use resolve_handle() to populate
            display_name: None,    // Requires profile fetch
            text: record.text.clone(),
            created_at: chrono::DateTime::parse_from_rfc3339(record.created_at.as_str())
                .unwrap_or_else(|_| chrono::Utc::now().into())
                .to_utc(),
            reply: record.reply.as_ref().map(|r| r.data.clone()),
            embed,
            langs: record.langs.as_ref().map_or(Vec::new(), |langs| {
                langs.iter().map(|l| l.as_ref().to_string()).collect()
            }),
            labels,
            facets,
            indexed_at: Some(chrono::Utc::now()),
            hydration: HydrationState::default(), // Not hydrated from firehose
            like_count: None,                     // Not available from firehose
            reply_count: None,                    // Not available from firehose
            repost_count: None,                   // Not available from firehose
        }
    }

    /// Resolve handle from DID using the provided resolver
    pub async fn resolve_handle(
        &mut self,
        resolver: &atrium_identity::did::CommonDidResolver<PatternHttpClient>,
    ) {
        let did = match atrium_api::types::string::Did::from_str(&self.did) {
            Ok(d) => d,
            Err(_) => {
                // If DID is invalid, just use it as the handle
                self.handle = self.did.clone();
                return;
            }
        };

        match resolver.resolve(&did).await {
            Ok(doc) => {
                if let Some(also_known_as) = doc.also_known_as {
                    if let Some(first) = also_known_as.first() {
                        let handle = first
                            .trim_start_matches("at://")
                            .trim_start_matches('@')
                            .to_string();
                        self.handle = handle;
                        self.hydration.handle_resolved = true;
                    } else {
                        self.handle = self.did.clone();
                    }
                } else {
                    self.handle = self.did.clone();
                }
            }
            Err(_) => {
                // On any error, fall back to using DID as handle
                self.handle = self.did.clone();
            }
        }
    }

    /// Create from a PostView (full API data)
    pub fn from_post_view(post_view: &atrium_api::app::bsky::feed::defs::PostView) -> Option<Self> {
        // Extract the actual post record from the PostView's Unknown type
        let record = atrium_api::app::bsky::feed::post::RecordData::try_from_unknown(
            post_view.record.clone(),
        )
        .ok()?;

        // Create the post using from_record with PostView's CID
        let mut post = Self::from_record(
            &record,
            post_view.uri.clone(),
            post_view.author.did.as_str().to_string(),
            post_view.cid.clone(),
        );

        // PostView has the handle already resolved
        post.handle = post_view.author.handle.as_str().to_string();
        post.hydration.handle_resolved = true;

        // Enrich with PostView-specific data
        post.display_name = post_view.author.display_name.clone();
        if post.display_name.is_some() {
            post.hydration.profile_fetched = true;
        }

        // Add engagement metrics from PostView
        post.like_count = post_view.like_count;
        post.reply_count = post_view.reply_count;
        post.repost_count = post_view.repost_count;

        // Update indexed_at from PostView
        post.indexed_at = chrono::DateTime::parse_from_rfc3339(post_view.indexed_at.as_str())
            .ok()
            .map(|dt| dt.to_utc());

        // Convert PostView embed to enriched EmbedInfo (has more data than record embeds)
        if let Some(embed) = &post_view.embed {
            if let Some(enriched_embed) = Self::convert_postview_embed(embed) {
                post.embed = Some(enriched_embed);
                post.hydration.embed_enriched = true;
            }
        }

        Some(post)
    }
    /// Convert PostView embed to EmbedInfo without network calls
    fn convert_postview_embed(embed: &Union<PostViewEmbedRefs>) -> Option<EmbedInfo> {
        match embed {
            Union::Refs(PostViewEmbedRefs::AppBskyEmbedImagesView(images_view)) => {
                // PostView has image thumbnails but not full URLs
                Some(EmbedInfo::Images {
                    count: images_view.images.len(),
                    alt_texts: images_view
                        .images
                        .iter()
                        .map(|img| img.alt.clone())
                        .collect(),
                    urls: images_view
                        .images
                        .iter()
                        .map(|img| img.thumb.clone()) // Use thumb URLs for now
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
                    Union::Refs(
                        atrium_api::app::bsky::embed::record::ViewRecordRefs::ViewRecord(
                            view_record,
                        ),
                    ) => {
                        // Extract text from the embedded record if possible
                        let text = if let Ok(quoted_record) =
                            atrium_api::app::bsky::feed::post::RecordData::try_from_unknown(
                                view_record.value.clone(),
                            ) {
                            quoted_record.text
                        } else {
                            String::new()
                        };

                        Some(EmbedInfo::Quote {
                            uri: view_record.uri.clone(),
                            cid: view_record.cid.as_ref().to_string(),
                            author_handle: view_record.author.handle.as_str().to_string(),
                            author_display_name: view_record.author.display_name.clone(),
                            text,
                            created_at: chrono::DateTime::parse_from_rfc3339(
                                view_record.indexed_at.as_str(),
                            )
                            .ok()
                            .map(|dt| dt.to_utc()),
                        })
                    }
                    _ => None,
                }
            }
            Union::Refs(PostViewEmbedRefs::AppBskyEmbedRecordWithMediaView(record_with_media)) => {
                // Extract quote part
                let quote = match &record_with_media.record.record {
                    Union::Refs(
                        atrium_api::app::bsky::embed::record::ViewRecordRefs::ViewRecord(
                            view_record,
                        ),
                    ) => {
                        let text = if let Ok(quoted_record) =
                            atrium_api::app::bsky::feed::post::RecordData::try_from_unknown(
                                view_record.value.clone(),
                            ) {
                            quoted_record.text
                        } else {
                            String::new()
                        };

                        Box::new(EmbedInfo::Quote {
                            uri: view_record.uri.clone(),
                            cid: view_record.cid.as_ref().to_string(),
                            author_handle: view_record.author.handle.as_str().to_string(),
                            author_display_name: view_record.author.display_name.clone(),
                            text,
                            created_at: chrono::DateTime::parse_from_rfc3339(
                                view_record.indexed_at.as_str(),
                            )
                            .ok()
                            .map(|dt| dt.to_utc()),
                        })
                    }
                    _ => return None,
                };

                // Extract media part - reuse the same logic
                let media = match &record_with_media.media {
                    Union::Refs(atrium_api::app::bsky::embed::record_with_media::ViewMediaRefs::AppBskyEmbedImagesView(images)) => {
                        Box::new(EmbedInfo::Images {
                            count: images.images.len(),
                            alt_texts: images.images.iter().map(|img| img.alt.clone()).collect(),
                            urls: images.images.iter().map(|img| img.thumb.clone()).collect(),
                        })
                    }
                    Union::Refs(atrium_api::app::bsky::embed::record_with_media::ViewMediaRefs::AppBskyEmbedExternalView(external)) => {
                        Box::new(EmbedInfo::External {
                            uri: external.external.uri.clone(),
                            title: external.external.title.clone(),
                            description: external.external.description.clone(),
                        })
                    }
                    _ => return None,
                };

                Some(EmbedInfo::QuoteWithMedia { quote, media })
            }
            _ => None,
        }
    }

    /// Check if post needs any hydration
    pub fn needs_hydration(&self) -> bool {
        !self.hydration.handle_resolved
            || !self.hydration.profile_fetched
            || (!self.hydration.embed_enriched && self.embed.is_some())
    }

    /// Audit what hydration is actually present and update state to match
    /// Returns the current HydrationState after auditing
    pub fn audit_hydration(&mut self) -> HydrationState {
        // Check if handle is actually resolved (not empty and not the DID)
        self.hydration.handle_resolved = !self.handle.is_empty() && self.handle != self.did;

        // Check if profile data is present
        self.hydration.profile_fetched = self.display_name.is_some();

        // Check if embed is fully enriched
        if let Some(embed) = &self.embed {
            self.hydration.embed_enriched = match embed {
                EmbedInfo::Quote { author_handle, .. } => !author_handle.is_empty(),
                EmbedInfo::Images { urls, .. } => !urls.is_empty(),
                EmbedInfo::QuoteWithMedia { quote, media } => {
                    let quote_enriched = match quote.as_ref() {
                        EmbedInfo::Quote { author_handle, .. } => !author_handle.is_empty(),
                        _ => true,
                    };
                    let media_enriched = match media.as_ref() {
                        EmbedInfo::Images { urls, .. } => !urls.is_empty(),
                        _ => true,
                    };
                    quote_enriched && media_enriched
                }
                EmbedInfo::External { .. } => true, // External links are always complete
            };
        } else {
            // No embed means nothing to enrich
            self.hydration.embed_enriched = true;
        }

        // Return the current hydration state
        self.hydration
    }

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
        self.embed.as_ref().map_or(false, |e| {
            matches!(
                e,
                EmbedInfo::Images { .. } | EmbedInfo::QuoteWithMedia { .. }
            )
        })
    }

    /// Extract alt text from image embeds (for accessibility)
    pub fn image_alt_texts(&self) -> Vec<String> {
        match &self.embed {
            Some(EmbedInfo::Images { alt_texts, .. }) => alt_texts.clone(),
            Some(EmbedInfo::QuoteWithMedia { media, .. }) => {
                if let EmbedInfo::Images { alt_texts, .. } = media.as_ref() {
                    alt_texts.clone()
                } else {
                    Vec::new()
                }
            }
            _ => Vec::new(),
        }
    }

    /// Extract external link from embed (link cards)
    pub fn embedded_link(&self) -> Option<String> {
        match &self.embed {
            Some(EmbedInfo::External { uri, .. }) => Some(uri.clone()),
            _ => None,
        }
    }

    /// Extract quoted post URI if this is a quote post
    pub fn quoted_post_uri(&self) -> Option<String> {
        match &self.embed {
            Some(EmbedInfo::Quote { uri, .. }) => Some(uri.clone()),
            Some(EmbedInfo::QuoteWithMedia { quote, .. }) => {
                if let EmbedInfo::Quote { uri, .. } = quote.as_ref() {
                    Some(uri.clone())
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    /// Get quoted post info if available
    pub fn quoted_post(&self) -> Option<QuotedPost> {
        match &self.embed {
            Some(EmbedInfo::Quote {
                uri,
                cid,
                author_handle,
                author_display_name,
                text,
                created_at,
                ..
            }) => {
                Some(QuotedPost {
                    uri: uri.clone(),
                    cid: cid.clone(),
                    did: String::new(), // TODO: Need to store DID in EmbedInfo::Quote
                    handle: author_handle.clone(),
                    display_name: author_display_name.clone(),
                    text: text.clone(),
                    created_at: *created_at,
                })
            }
            Some(EmbedInfo::QuoteWithMedia { quote, .. }) => {
                if let EmbedInfo::Quote {
                    uri,
                    cid,
                    author_handle,
                    author_display_name,
                    text,
                    created_at,
                    ..
                } = quote.as_ref()
                {
                    Some(QuotedPost {
                        uri: uri.clone(),
                        cid: cid.clone(),
                        did: String::new(), // TODO: Need to store DID in EmbedInfo::Quote
                        handle: author_handle.clone(),
                        display_name: author_display_name.clone(),
                        text: text.clone(),
                        created_at: *created_at,
                    })
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    /// Format author string with display name and handle
    fn format_author(&self) -> String {
        if let Some(name) = &self.display_name {
            format!("{} (@{})", name, self.handle)
        } else {
            format!("@{}", self.handle)
        }
    }

    /// Format relative timestamp
    fn format_timestamp(&self) -> String {
        let interval = Utc::now() - self.created_at;
        format_duration(interval.to_std().unwrap_or(Duration::from_secs(0)))
    }

    /// Append as standalone post (no thread context shown)
    fn append_as_standalone(&self, buf: &mut String, agent_did: Option<&str>) {
        // Check if this is the agent's post
        let is_agent = agent_did.map_or(false, |did| self.did == did);
        let marker = if is_agent { "[YOU] " } else { "" };

        buf.push_str(&format!(
            "{}{}: {} - {} ago\n",
            marker,
            self.format_author(),
            self.text,
            self.format_timestamp()
        ));

        // Add embed if present
        if let Some(embed) = &self.embed {
            embed.append_to_buffer(buf, "");
        }

        buf.push_str(&format!("   🔗 {}\n", self.uri));
    }

    /// Append as main post in a thread (shows ">>> MAIN POST >>>")
    fn append_as_main(&self, buf: &mut String, agent_did: Option<&str>) {
        buf.push_str("\n>>> MAIN POST >>>\n");

        let is_agent = agent_did.map_or(false, |did| self.did == did);
        let marker = if is_agent { "[YOU] " } else { "" };

        buf.push_str(&format!(
            "{}{}: {} - {} ago\n",
            marker,
            self.format_author(),
            self.text,
            self.format_timestamp()
        ));

        // Add embed with proper indentation
        if let Some(embed) = &self.embed {
            embed.append_to_buffer(buf, "│");
        }

        buf.push_str(&format!("│ 🔗 {}\n", self.uri));
    }

    /// Append as parent post (what main is replying to)
    fn append_as_parent(&self, buf: &mut String, agent_did: Option<&str>, indent: &str) {
        let is_agent = agent_did.map_or(false, |did| self.did == did);
        let marker = if is_agent { "[YOU] " } else { "" };

        buf.push_str(&format!(
            "{}└─ {}{} - {} ago: {}\n",
            indent,
            marker,
            self.format_author(),
            self.format_timestamp(),
            self.text
        ));

        // Add embed if present
        if let Some(embed) = &self.embed {
            let next_indent = format!("{}   ", indent);
            embed.append_to_buffer(buf, &next_indent);
        }

        buf.push_str(&format!("{}   🔗 {}\n", indent, self.uri));
    }

    /// Append as sibling (other replies to same parent)
    fn append_as_sibling(
        &self,
        buf: &mut String,
        agent_did: Option<&str>,
        indent: &str,
        is_last: bool,
    ) {
        let is_agent = agent_did.map_or(false, |did| self.did == did);
        let marker = if is_agent { "[YOU] " } else { "" };
        let connector = if is_last { "└─" } else { "├─" };

        buf.push_str(&format!(
            "{}{} {}{} - {} ago: {}\n",
            indent,
            connector,
            marker,
            self.format_author(),
            self.format_timestamp(),
            self.text
        ));

        // Add embed if present
        if let Some(embed) = &self.embed {
            let next_indent = if is_last {
                format!("{}   ", indent)
            } else {
                format!("{}│  ", indent)
            };
            embed.append_to_buffer(buf, &next_indent);
        }

        buf.push_str(&format!(
            "{}   🔗 {}\n",
            if is_last {
                format!("{}   ", indent)
            } else {
                format!("{}│  ", indent)
            },
            self.uri
        ));
    }

    /// Append as reply (replies to main post or siblings)
    fn append_as_reply(
        &self,
        buf: &mut String,
        agent_did: Option<&str>,
        indent: &str,
        depth: usize,
    ) {
        let is_agent = agent_did.map_or(false, |did| self.did == did);
        let marker = if is_agent { "[YOU] " } else { "" };

        // Use different connectors based on depth
        let connector = if depth == 1 { "└─" } else { "  └─" };

        buf.push_str(&format!(
            "{}{} {}{} - {} ago: {}\n",
            indent,
            connector,
            marker,
            self.format_author(),
            self.format_timestamp(),
            self.text
        ));

        // Add embed if present
        if let Some(embed) = &self.embed {
            let next_indent = format!("{}     ", indent);
            embed.append_to_buffer(buf, &next_indent);
        }

        buf.push_str(&format!("{}     🔗 {}\n", indent, self.uri));
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

/// Pending batch of posts grouped by thread and author
struct PendingBatch {
    /// Posts grouped by thread root URI
    posts_by_thread: DashMap<String, Vec<BlueskyPost>>,
    /// Posts grouped by author DID
    posts_by_author: DashMap<String, Vec<BlueskyPost>>,
    /// When each batch started collecting (thread URI -> start time)
    batch_timers: DashMap<String, Instant>,
    /// URIs we've already sent notifications for
    processed_uris: DashMap<String, Instant>,
    /// Posts waiting to be processed (in arrival order)
    pending_posts: Arc<tokio::sync::RwLock<Vec<BlueskyPost>>>,
}

impl Default for PendingBatch {
    fn default() -> Self {
        Self {
            posts_by_thread: DashMap::new(),
            posts_by_author: DashMap::new(),
            batch_timers: DashMap::new(),
            processed_uris: DashMap::new(),
            pending_posts: Arc::new(tokio::sync::RwLock::new(Vec::new())),
        }
    }
}

/// Cached thread context from constellation API
#[derive(Debug, Clone)]
struct CachedThreadContext {
    context: ThreadContext,
    cached_at: Instant,
    /// Root URI of the thread
    thread_root: String,
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
    // Batching state
    pending_batch: Arc<PendingBatch>,
    batch_window: Duration,
    thread_cache: Arc<DashMap<String, CachedThreadContext>>,
    thread_cache_ttl: Duration,
    // HTTP client for constellation API calls
    http_client: PatternHttpClient,
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
            .field("batch_window", &self.batch_window)
            .field("thread_cache_ttl", &self.thread_cache_ttl)
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
        // Determine the thread root - either the parent URI or the post URI itself if it's a root post
        let thread_root = parent_uri.unwrap_or(post_uri);

        // Check cache first
        if let Some(cached_context) = self.get_cached_thread_context(thread_root) {
            return Ok(cached_context);
        }
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
                context.parent = parent_result
                    .posts
                    .clone()
                    .into_iter()
                    .next()
                    .and_then(|pv| BlueskyPost::from_post_view(&pv));

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

            // Fetch siblings using cache-aware method
            context.siblings = self
                .get_or_fetch_siblings(
                    parent,
                    agent_did,
                    filter,
                    post_uri,
                    HydrationState {
                        handle_resolved: true,
                        profile_fetched: true,
                        embed_enriched: true,
                    },
                    bsky_agent,
                )
                .await?;

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

            // If depth > 0, fetch replies to each sibling recursively
            if max_depth > 0 {
                // Prioritize fetching replies to agent's posts
                let mut priority_siblings = Vec::new();
                let mut regular_siblings = Vec::new();

                for sibling in &context.siblings {
                    if let Some(agent) = agent_did {
                        if sibling.did.as_str() == agent {
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

        // Cache the result before returning
        self.cache_thread_context(thread_root.to_string(), context.clone());

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
            .http_client
            .fetch_thread_siblings(parent_uri)
            .await
            .unwrap_or_default();

        if reply_records.is_empty() {
            return;
        }

        // Filter reply records
        let filtered_replies = PatternHttpClient::filter_constellation_records(
            reply_records,
            agent_did,
            filter,
            parent_uri,
        );

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
            let reply_posts = replies_result.posts.clone();

            // Convert to BlueskyPost and collect engagement metrics
            let replies: Vec<BlueskyPost> = reply_posts
                .iter()
                .filter_map(|post| {
                    context.engagement_map.insert(
                        post.uri.clone(),
                        PostEngagement {
                            like_count: post.like_count.unwrap_or(0) as u32,
                            reply_count: post.reply_count.unwrap_or(0) as u32,
                            repost_count: post.repost_count.unwrap_or(0) as u32,
                        },
                    );
                    BlueskyPost::from_post_view(post)
                })
                .collect();

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
                                reply.did.as_str() == agent || current_depth <= 2
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
            pending_batch: Arc::new(PendingBatch::default()),
            batch_window: Duration::from_secs(3), // 3 second window to collect related posts
            thread_cache: Arc::new(DashMap::new()),
            thread_cache_ttl: Duration::from_secs(300), // Cache threads for 5 minutes
            http_client: PatternHttpClient::default(),
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

    /// Check if a post should be batched based on current batch state
    fn should_batch_post(&self, post: &BlueskyPost) -> Option<String> {
        // Find the thread root for this post
        let thread_root = self.find_thread_root(post);

        // Check if we're already batching posts from this thread
        if self
            .pending_batch
            .posts_by_thread
            .contains_key(&thread_root)
        {
            return Some(thread_root);
        }

        // Check if we have posts from this author recently
        if let Some(author_posts) = self.pending_batch.posts_by_author.get(&post.did) {
            // If author posted in last few seconds, batch them together
            if !author_posts.is_empty() {
                return Some(thread_root);
            }
        }

        // New thread/author - start a new batch if we should
        None
    }

    /// Find the root of a thread for batching purposes
    fn find_thread_root(&self, post: &BlueskyPost) -> String {
        if let Some(reply) = &post.reply {
            // If this is a reply, find the root of the thread
            reply.root.uri.clone()
        } else {
            // This is a root post itself
            post.uri.clone()
        }
    }

    /// Add a post to the pending batch
    async fn add_to_batch(&self, post: BlueskyPost, thread_root: String) {
        // Add to thread grouping
        self.pending_batch
            .posts_by_thread
            .entry(thread_root.clone())
            .or_insert_with(Vec::new)
            .push(post.clone());

        // Add to author grouping
        self.pending_batch
            .posts_by_author
            .entry(post.did.clone())
            .or_insert_with(Vec::new)
            .push(post.clone());

        // Start batch timer if this is the first post in the batch
        self.pending_batch
            .batch_timers
            .entry(thread_root.clone())
            .or_insert_with(Instant::now);

        // Add to pending posts list
        let mut pending = self.pending_batch.pending_posts.write().await;
        pending.push(post);
    }

    /// Get cached thread context if it exists and is fresh
    fn get_cached_thread_context(&self, thread_root: &str) -> Option<ThreadContext> {
        if let Some(cached) = self.thread_cache.get(thread_root) {
            let now = std::time::Instant::now();
            if now.duration_since(cached.cached_at) < self.thread_cache_ttl {
                tracing::debug!("📁 Cache HIT for thread {}", thread_root);
                return Some(cached.context.clone());
            } else {
                tracing::debug!("🕐 Cache EXPIRED for thread {}", thread_root);
                // Remove expired entry
                self.thread_cache.remove(thread_root);
            }
        } else {
            tracing::debug!("📁 Cache MISS for thread {}", thread_root);
        }
        None
    }

    /// Store thread context in cache
    fn cache_thread_context(&self, thread_root: String, context: ThreadContext) {
        let cached = CachedThreadContext {
            context: context.clone(),
            cached_at: std::time::Instant::now(),
            thread_root: thread_root.clone(),
        };

        self.thread_cache.insert(thread_root.clone(), cached);
        tracing::debug!("💾 Cached thread context for {}", thread_root);
    }

    /// Get cached post if it exists and meets hydration requirements
    fn get_cached_post(
        &self,
        uri: &str,
        required_hydration: HydrationState,
    ) -> Option<BlueskyPost> {
        // Check in pending posts first
        if let Ok(pending_posts) = self.pending_batch.pending_posts.try_read() {
            if let Some(post) = pending_posts.iter().find(|p| p.uri == uri) {
                if post.hydration >= required_hydration {
                    tracing::debug!(
                        "📁 Post cache HIT for {} (hydration: {:?})",
                        uri,
                        post.hydration
                    );
                    return Some(post.clone());
                } else {
                    tracing::debug!(
                        "🔄 Post cache PARTIAL for {} (has: {:?}, need: {:?})",
                        uri,
                        post.hydration,
                        required_hydration
                    );
                }
            }
        }

        // Check in batched posts
        for entry in self.pending_batch.posts_by_thread.iter() {
            if let Some(post) = entry.value().iter().find(|p| p.uri == uri) {
                if post.hydration >= required_hydration {
                    tracing::debug!(
                        "📁 Post cache HIT (batched) for {} (hydration: {:?})",
                        uri,
                        post.hydration
                    );
                    return Some(post.clone());
                } else {
                    tracing::debug!(
                        "🔄 Post cache PARTIAL (batched) for {} (has: {:?}, need: {:?})",
                        uri,
                        post.hydration,
                        required_hydration
                    );
                }
            }
        }

        tracing::debug!("📁 Post cache MISS for {}", uri);
        None
    }

    /// Get or fetch a single post with required hydration level
    async fn get_or_fetch_post(
        &self,
        uri: &str,
        required_hydration: HydrationState,
        bsky_agent: &Arc<bsky_sdk::BskyAgent>,
    ) -> Result<Option<BlueskyPost>> {
        // Check cache first
        if let Some(cached_post) = self.get_cached_post(uri, required_hydration) {
            return Ok(Some(cached_post));
        }

        // Fetch from API
        let params = atrium_api::app::bsky::feed::get_posts::ParametersData {
            uris: vec![uri.to_string()],
        };

        match bsky_agent.api.app.bsky.feed.get_posts(params.into()).await {
            Ok(result) => {
                if let Some(post_view) = result.posts.clone().into_iter().next() {
                    if let Some(mut post) = BlueskyPost::from_post_view(&post_view) {
                        post.hydration = required_hydration;

                        // Cache the post
                        if let Ok(mut pending_posts) = self.pending_batch.pending_posts.try_write()
                        {
                            pending_posts.push(post.clone());
                        }

                        tracing::debug!(
                            "🌐 Fetched and cached post {} (hydration: {:?})",
                            uri,
                            required_hydration
                        );
                        return Ok(Some(post));
                    }
                }
                Ok(None)
            }
            Err(e) => {
                tracing::warn!("Failed to fetch post {}: {}", uri, e);
                Ok(None)
            }
        }
    }

    /// Get or fetch multiple posts with intelligent batching
    async fn get_or_fetch_posts(
        &self,
        uris: Vec<String>,
        required_hydration: HydrationState,
        bsky_agent: &Arc<bsky_sdk::BskyAgent>,
    ) -> Result<Vec<BlueskyPost>> {
        let mut cached_posts = Vec::new();
        let mut need_fetch = Vec::new();

        // Check cache for each URI
        for uri in uris {
            if let Some(cached_post) = self.get_cached_post(&uri, required_hydration) {
                cached_posts.push(cached_post);
            } else {
                need_fetch.push(uri);
            }
        }

        // Fetch missing posts in one API call
        if !need_fetch.is_empty() {
            let params = atrium_api::app::bsky::feed::get_posts::ParametersData {
                uris: need_fetch.clone(),
            };

            match bsky_agent.api.app.bsky.feed.get_posts(params.into()).await {
                Ok(result) => {
                    for post_view in &result.posts {
                        if let Some(mut post) = BlueskyPost::from_post_view(&post_view) {
                            post.hydration = required_hydration;

                            // Cache the post
                            if let Ok(mut pending_posts) =
                                self.pending_batch.pending_posts.try_write()
                            {
                                pending_posts.push(post.clone());
                            }

                            cached_posts.push(post);
                        }
                    }
                    tracing::debug!(
                        "🌐 Batch fetched {} posts (hydration: {:?})",
                        need_fetch.len(),
                        required_hydration
                    );
                }
                Err(e) => {
                    tracing::warn!("Failed to batch fetch posts: {}", e);
                    return Err(crate::CoreError::DataSourceError {
                        source_name: self.source_id.clone(),
                        operation: "batch_fetch_posts".to_string(),
                        cause: e.to_string(),
                    });
                }
            }
        }

        Ok(cached_posts)
    }

    /// Get or fetch siblings with smart caching and constellation integration
    async fn get_or_fetch_siblings(
        &self,
        parent_uri: &str,
        agent_did: Option<&str>,
        filter: &BlueskyFilter,
        post_uri: &str,
        required_hydration: HydrationState,
        bsky_agent: &Arc<bsky_sdk::BskyAgent>,
    ) -> Result<Vec<BlueskyPost>> {
        // Fetch constellation records first
        let sibling_records = self.http_client.fetch_thread_siblings(parent_uri).await?;

        // Filter siblings based on our criteria
        let filtered_records = PatternHttpClient::filter_constellation_records(
            sibling_records,
            agent_did,
            filter,
            post_uri,
        );

        let sibling_uris: Vec<String> = filtered_records
            .into_iter()
            .map(|record| record.to_at_uri())
            .collect();

        if sibling_uris.is_empty() {
            return Ok(Vec::new());
        }

        // Use smart cache-aware fetching
        self.get_or_fetch_posts(sibling_uris, required_hydration, bsky_agent)
            .await
    }

    /// Check if any batches have expired and need to be flushed
    async fn get_expired_batches(&self) -> Vec<String> {
        let now = Instant::now();
        let mut expired = Vec::new();

        for entry in self.pending_batch.batch_timers.iter() {
            let (thread_root, started) = entry.pair();
            if now.duration_since(*started) >= self.batch_window {
                expired.push(thread_root.clone());
            }
        }

        expired
    }

    /// Flush a batch and create consolidated notification
    async fn flush_batch(
        &self,
        thread_root: &str,
    ) -> Option<(String, Vec<(CompactString, MemoryBlock)>)> {
        // Remove the batch from pending
        let posts = self.pending_batch.posts_by_thread.remove(thread_root)?;
        let (_, mut posts) = posts;

        // Remove batch timer
        self.pending_batch.batch_timers.remove(thread_root);

        // Clean up author groupings
        for post in &posts {
            if let Some(mut author_posts) = self.pending_batch.posts_by_author.get_mut(&post.did) {
                author_posts.retain(|p| p.uri != post.uri);
                if author_posts.is_empty() {
                    drop(author_posts); // Drop the lock before removing
                    self.pending_batch.posts_by_author.remove(&post.did);
                }
            }
        }

        // Sort posts by timestamp
        posts.sort_by_key(|p| p.created_at);

        // Create consolidated notification
        self.format_batch_notification(posts).await
    }

    /// Format a batch of posts into a single notification
    async fn format_batch_notification(
        &self,
        mut posts: Vec<BlueskyPost>,
    ) -> Option<(String, Vec<(CompactString, MemoryBlock)>)> {
        if posts.is_empty() {
            return None;
        }

        // Sort by timestamp to find the most recent post as the "main" post
        posts.sort_by_key(|p| p.created_at);
        let main_post = posts.last()?.clone();

        // Get agent DID from mentions filter (agent monitors for mentions of itself)
        let agent_did = self.filter.mentions.first().map(|s| s.as_str());

        // Try to get cached thread context or build it
        let thread_context =
            if let Some(ctx) = self.get_cached_thread_context(&main_post.thread_root()) {
                ctx
            } else if let Some(bsky_agent) = &self.bsky_agent {
                // Build thread context if we have auth
                match self
                    .build_thread_context(
                        &main_post.uri,
                        main_post.reply.as_ref().map(|r| r.parent.uri.as_str()),
                        bsky_agent,
                        agent_did,
                        &self.filter,
                        2, // Limited depth for batch notifications
                    )
                    .await
                {
                    Ok(ctx) => ctx,
                    Err(_) => ThreadContext {
                        parent: None,
                        siblings: posts[..posts.len() - 1].to_vec(), // All but the last as siblings
                        replies_map: std::collections::HashMap::new(),
                        engagement_map: std::collections::HashMap::new(),
                        agent_interactions: std::collections::HashMap::new(),
                    },
                }
            } else {
                // No auth, minimal context
                ThreadContext {
                    parent: None,
                    siblings: posts[..posts.len() - 1].to_vec(), // All but the last as siblings
                    replies_map: std::collections::HashMap::new(),
                    engagement_map: std::collections::HashMap::new(),
                    agent_interactions: std::collections::HashMap::new(),
                }
            };

        // Format the notification
        let mut message = String::new();

        // Add batch header
        message.push_str(&format!(
            "📦 Thread activity ({} posts in last 3 seconds)\n\n",
            posts.len()
        ));

        // Format the thread tree
        thread_context.append_thread_tree(&mut message, &main_post, agent_did);

        // Add reply options
        thread_context.format_reply_options(&mut message, &main_post);

        // Collect memory blocks
        let mut memory_blocks = Vec::new();
        let mut added_memory_labels = std::collections::HashSet::new();

        // Add user memory blocks for unique users in the batch
        if let Some(ref agent_handle) = self.agent_handle {
            let mut seen_users = std::collections::HashSet::new();
            for post in &posts {
                if seen_users.insert(post.did.clone()) {
                    let memory_label = format!("bluesky_user_{}", post.handle.replace('.', "_"));
                    let compact_label = CompactString::from(memory_label.clone());

                    // Check if memory already exists
                    if let Ok(existing_memory) = agent_handle
                        .get_archival_memory_by_label(&memory_label)
                        .await
                    {
                        if let Some(existing_block) = existing_memory {
                            // Memory exists - add to return list and note in message
                            if added_memory_labels.insert(memory_label.clone()) {
                                memory_blocks.push((compact_label, existing_block));
                                message
                                    .push_str(&format!("\n\n📝 Memory exists: {}", memory_label));
                            }
                        } else {
                            // Create new memory block
                            let memory_content = if let Some(bsky_agent) = &self.bsky_agent {
                                Self::fetch_user_profile_for_memory(
                                    bsky_agent,
                                    &post.handle,
                                    &post.did,
                                )
                                .await
                            } else {
                                create_basic_memory_content(&post.handle, &post.did)
                            };

                            // Insert into agent's archival memory
                            if let Err(e) = agent_handle
                                .insert_archival_memory(&memory_label, &memory_content)
                                .await
                            {
                                tracing::warn!(
                                    "Failed to create memory block for {}: {}",
                                    post.handle,
                                    e
                                );
                            } else {
                                // Mark as added and note in message
                                if added_memory_labels.insert(memory_label.clone()) {
                                    message.push_str(&format!(
                                        "\n\n📝 Memory created: {}",
                                        memory_label
                                    ));

                                    // Retrieve the created block to return it
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
            }
        }

        Some((message, memory_blocks))
    }

    /// Format a single post notification (without batching checks)
    async fn format_single_notification(
        &self,
        item: &BlueskyPost,
    ) -> Option<(String, Vec<(CompactString, MemoryBlock)>)> {
        // Mark this post as processed to avoid reprocessing
        self.pending_batch
            .processed_uris
            .insert(item.uri.clone(), Instant::now());

        // Early exit for obvious exclusions (saves computation)
        if self.filter.exclude_dids.contains(&item.did) {
            tracing::debug!("Early exit: post from excluded DID: {}", item.did);
            return None;
        }

        // Get agent DID from mentions filter (agent monitors for mentions of itself)
        let agent_did = self.filter.mentions.first().map(|s| s.as_str());

        // Build or get cached thread context if this is a reply
        let thread_context = if let Some(reply) = &item.reply {
            if let Some(ctx) = self.get_cached_thread_context(&item.thread_root()) {
                Some(ctx)
            } else if let Some(bsky_agent) = &self.bsky_agent {
                self.build_thread_context(
                    &item.uri,
                    Some(&reply.parent.uri),
                    bsky_agent,
                    agent_did,
                    &self.filter,
                    2,
                )
                .await
                .ok()
            } else {
                None
            }
        } else {
            None
        };

        // Now that we have full thread context, apply comprehensive filtering

        // Build list of all DIDs in the thread
        let mut thread_dids = vec![item.did.clone()];
        let mut mention_check_queue = item
            .mentioned_dids()
            .iter()
            .map(|s| s.to_string())
            .collect::<Vec<_>>();

        if let Some(ref ctx) = thread_context {
            // Add parent DID and mentions
            if let Some(ref parent) = ctx.parent {
                thread_dids.push(parent.did.clone());
                mention_check_queue.extend(parent.mentioned_dids().iter().map(|s| s.to_string()));
            }

            // Add sibling DIDs and mentions
            for sibling in &ctx.siblings {
                thread_dids.push(sibling.did.clone());
                mention_check_queue.extend(sibling.mentioned_dids().iter().map(|s| s.to_string()));
            }

            // Add reply DIDs and mentions
            for replies in ctx.replies_map.values() {
                for reply in replies {
                    thread_dids.push(reply.did.clone());
                    mention_check_queue
                        .extend(reply.mentioned_dids().iter().map(|s| s.to_string()));
                }
            }
        }

        // Check if quoted post mentions anyone
        if let Some(quoted) = item.quoted_post() {
            thread_dids.push(quoted.did.clone());
            mention_check_queue.push(quoted.did.clone());
        }

        // Apply exclusion filters on entire thread
        for did in &thread_dids {
            if self.filter.exclude_dids.contains(did) {
                tracing::debug!("Excluding: thread contains excluded DID: {}", did);
                return None;
            }
        }

        // Check for excluded keywords in the entire formatted message (will build it temporarily)
        if !self.filter.exclude_keywords.is_empty() {
            let mut temp_message = String::new();
            if let Some(ref ctx) = thread_context {
                ctx.append_thread_tree(&mut temp_message, item, agent_did);
            } else {
                item.append_as_standalone(&mut temp_message, agent_did);
            }

            let message_lower = temp_message.to_lowercase();
            for keyword in &self.filter.exclude_keywords {
                if message_lower.contains(&keyword.to_lowercase()) {
                    tracing::debug!("Excluding: thread contains excluded keyword: {}", keyword);
                    return None;
                }
            }
        }

        // Check if any friends are in the thread
        let has_friend_in_thread = thread_dids
            .iter()
            .any(|did| self.filter.friends.contains(did));

        // Check if agent is mentioned anywhere
        let agent_mentioned = agent_did.map_or(false, |aid| {
            mention_check_queue.contains(&aid.to_string())
                || mention_check_queue.contains(&format!("@{}", aid))
        });

        // Check if agent authored anything in thread
        let agent_in_thread = agent_did.map_or(false, |aid| thread_dids.contains(&aid.to_string()));

        // Determine if we should show this post
        if !has_friend_in_thread && !agent_mentioned && !agent_in_thread {
            tracing::debug!("Skipping: no friends in thread, agent not mentioned or involved");
            return None;
        }

        // Format the notification
        let mut message = String::new();

        if let Some(ctx) = thread_context {
            // This is a reply - show thread context
            message.push_str("  • 💬 New reply in thread:\n\n");
            ctx.append_thread_tree(&mut message, item, agent_did);
            ctx.format_reply_options(&mut message, item);
        } else {
            // Standalone post
            message.push_str("  • 📝 New post:\n\n");
            item.append_as_standalone(&mut message, agent_did);

            // Simple reply option for standalone posts
            message.push_str(&format!(
                "\n💭 Reply option: @{} ({})\n",
                item.handle, item.uri
            ));
            message.push_str("If you choose to reply (by using send_message with target_type bluesky and the target_id set to the uri), your response must contain under 300 characters or it will be truncated.\n");
        }

        // Collect memory blocks
        let mut memory_blocks = Vec::new();

        // Add user memory block
        if let Some(ref agent_handle) = self.agent_handle {
            let memory_label = format!("bluesky_user_{}", item.handle.replace('.', "_"));
            let compact_label = CompactString::from(memory_label.clone());

            // Check if memory already exists
            if let Ok(existing_memory) = agent_handle
                .get_archival_memory_by_label(&memory_label)
                .await
            {
                if let Some(existing_block) = existing_memory {
                    // Memory exists - add to return list and note in message
                    memory_blocks.push((compact_label, existing_block));
                    message.push_str(&format!("\n\n📝 Memory exists: {}", memory_label));
                } else {
                    // Create new memory block
                    let memory_content = if let Some(bsky_agent) = &self.bsky_agent {
                        Self::fetch_user_profile_for_memory(bsky_agent, &item.handle, &item.did)
                            .await
                    } else {
                        create_basic_memory_content(&item.handle, &item.did)
                    };

                    // Insert into agent's archival memory
                    if let Err(e) = agent_handle
                        .insert_archival_memory(&memory_label, &memory_content)
                        .await
                    {
                        tracing::warn!("Failed to create memory block for {}: {}", item.handle, e);
                    } else {
                        message.push_str(&format!("\n\n📝 Memory created: {}", memory_label));

                        // Retrieve the created block to return it
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

        Some((message, memory_blocks))
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
        tracing::info!(
            "BlueskyFirehoseSource::subscribe called for source_id: {}, endpoint: {}",
            self.source_id,
            self.endpoint
        );

        // Try to load cursor from file if not provided
        let effective_cursor = match from {
            Some(cursor) => Some(cursor),
            None => self.load_cursor_from_file().await?,
        };

        tracing::info!(
            "Effective cursor: {:?}, filter: {:?}",
            effective_cursor,
            self.filter
        );
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

                    // IMPORTANT: Acquire locks in same order as PostIngestor to avoid deadlock
                    // Always: last_send_time THEN buffer (never the reverse)

                    // First update the send time (acquire last_send_time lock)
                    let should_send = {
                        let mut last_send = queue_last_send_time.lock().await;
                        let elapsed = last_send.elapsed();
                        if elapsed >= interval {
                            *last_send = std::time::Instant::now();
                            true
                        } else {
                            false
                        }
                    };

                    if should_send {
                        // Now try to dequeue (acquire buffer lock AFTER releasing last_send_time)
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
                            }
                        }
                    }
                }
            });
        }

        // Spawn batch flushing task
        let batch_pending = self.pending_batch.clone();
        let batch_window = self.batch_window;
        let batch_tx = tx.clone();

        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(Duration::from_millis(500)); // Check every 500ms
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

            loop {
                ticker.tick().await;

                // Check for expired batches
                let now = Instant::now();
                let mut expired = Vec::new();

                for entry in batch_pending.batch_timers.iter() {
                    let (thread_root, started) = entry.pair();
                    if now.duration_since(*started) >= batch_window {
                        expired.push(thread_root.clone());
                    }
                }

                for thread_root in expired {
                    tracing::debug!("Flushing expired batch for thread: {}", thread_root);

                    // Remove the batch from pending
                    if let Some((_, mut posts)) = batch_pending.posts_by_thread.remove(&thread_root)
                    {
                        // Remove batch timer
                        batch_pending.batch_timers.remove(&thread_root);

                        // Clean up author groupings
                        for post in &posts {
                            if let Some(mut author_posts) =
                                batch_pending.posts_by_author.get_mut(&post.did)
                            {
                                author_posts.retain(|p| p.uri != post.uri);
                                if author_posts.is_empty() {
                                    drop(author_posts);
                                    batch_pending.posts_by_author.remove(&post.did);
                                }
                            }
                        }

                        // Sort posts by timestamp
                        posts.sort_by_key(|p| p.created_at);

                        // For now, just send the last post as a notification
                        // TODO: Implement proper batch formatting
                        if let Some(first_post) = posts.last() {
                            let event = StreamEvent {
                                item: first_post.clone(),
                                cursor: BlueskyFirehoseCursor {
                                    seq: 0, // TODO: Track actual sequence number
                                    time_us: chrono::Utc::now().timestamp_micros() as u64,
                                },
                                timestamp: chrono::Utc::now(),
                            };

                            if let Err(e) = batch_tx.send(Ok(event)).await {
                                tracing::error!("Failed to send batched notification: {}", e);
                                break;
                            }
                        }
                    }
                }
            }
        });

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

                // Create a shared flag for connection health
                let connection_alive = Arc::new(std::sync::atomic::AtomicBool::new(true));
                let connection_alive_clone = connection_alive.clone();

                // Spawn task to consume events from this connection
                let handle = tokio::spawn(async move {
                    // Process messages from jetstream with a timeout
                    // Firehose should ALWAYS have messages - if we don't get any for 10 seconds, it's dead
                    const MESSAGE_TIMEOUT_SECS: u64 = 10;
                    let mut last_message_time = std::time::Instant::now();
                    let mut messages_received = 0u64;

                    loop {
                        // Check if connection was marked dead by health check
                        if !connection_alive_clone.load(std::sync::atomic::Ordering::Relaxed) {
                            tracing::error!(
                                "Connection marked as dead by health check, exiting message handler"
                            );
                            break;
                        }

                        match tokio::time::timeout(
                            Duration::from_secs(MESSAGE_TIMEOUT_SECS),
                            msg_rx.recv_async(),
                        )
                        .await
                        {
                            Ok(Ok(message)) => {
                                //tracing::debug!("📨 Raw message received from jetstream");
                                // Got a message, process it
                                last_message_time = std::time::Instant::now();
                                messages_received += 1;

                                // Log periodic health check
                                if messages_received % 1000 == 0 {
                                    tracing::debug!(
                                        "Jetstream connection healthy: {} messages received",
                                        messages_received
                                    );
                                }

                                if let Err(e) = handler::handle_message(
                                    message,
                                    &ingestors,
                                    reconnect_tx.clone(),
                                    c_cursor.clone(),
                                )
                                .await
                                {
                                    tracing::warn!("Error processing message: {}", e);

                                    // Check if this is a connection error that should trigger reconnect
                                    let error_str = e.to_string().to_lowercase();
                                    if error_str.contains("websocket")
                                        || error_str.contains("connection")
                                        || error_str.contains("reset")
                                    {
                                        tracing::error!(
                                            "WebSocket error detected in message handler: {}, breaking connection loop",
                                            e
                                        );
                                        break;
                                    }

                                    let _ =
                                        ingestor_tx.send(Err(crate::CoreError::DataSourceError {
                                            source_name: c_source_id.clone(),
                                            operation: "process".to_string(),
                                            cause: e.to_string(),
                                        }));
                                }
                            }
                            Ok(Err(e)) => {
                                // Channel closed or error
                                tracing::warn!(
                                    "Message channel error: {:?}, connection terminated",
                                    e
                                );
                                break;
                            }
                            Err(_) => {
                                // Timeout - no messages for 10 seconds (firehose should never be this quiet)
                                let idle_duration =
                                    std::time::Instant::now().duration_since(last_message_time);
                                tracing::error!(
                                    "CRITICAL: No Jetstream messages for {} seconds (idle for {:?}), connection is definitely dead. Messages received before timeout: {}",
                                    MESSAGE_TIMEOUT_SECS,
                                    idle_duration,
                                    messages_received
                                );
                                break;
                            }
                        }
                    }
                    tracing::info!("Message handler exiting - will trigger reconnection");
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
        let start_time = std::time::Instant::now();

        // Check if already processed
        if self.pending_batch.processed_uris.contains_key(&item.uri) {
            tracing::debug!("Post already processed: {}", item.uri);
            return None;
        }

        // Check if this should be batched
        if let Some(thread_root) = self.should_batch_post(item) {
            tracing::debug!(
                "📦 Batching post @{}: {} with thread {}",
                item.handle,
                item.text.chars().take(50).collect::<String>(),
                thread_root
            );

            // Add to batch
            self.add_to_batch(item.clone(), thread_root.clone()).await;

            // Spawn opportunistic hydration task if we have auth
            if let Some(bsky_agent) = &self.bsky_agent {
                let post_uri = item.uri.clone();
                let agent = bsky_agent.clone();
                let batch = self.pending_batch.clone();

                tokio::spawn(async move {
                    // Short timeout for opportunistic hydration
                    let timeout = tokio::time::timeout(
                        std::time::Duration::from_millis(200),
                        async {
                            // Try to fetch full post data
                            let params = atrium_api::app::bsky::feed::get_posts::ParametersData {
                                uris: vec![post_uri.clone()],
                            };

                            if let Ok(result) = agent.api.app.bsky.feed.get_posts(params.into()).await {
                                if let Some(post_view) = result.posts.first() {
                                    if let Some(hydrated) = BlueskyPost::from_post_view(post_view) {
                                        // Update the cached post in posts_by_thread
                                        if let Some(mut thread_posts) = batch.posts_by_thread.get_mut(&post_uri) {
                                            // Find and replace the post in the thread's posts
                                            for post in thread_posts.iter_mut() {
                                                if post.uri == post_uri {
                                                    *post = hydrated.clone();
                                                    tracing::trace!("✨ Opportunistically hydrated post in thread: {}", post_uri);
                                                    break;
                                                }
                                            }
                                        }

                                        // Also check pending_posts vector
                                        let mut pending = batch.pending_posts.write().await;
                                        for post in pending.iter_mut() {
                                            if post.uri == post_uri {
                                                *post = hydrated;
                                                tracing::trace!("✨ Opportunistically hydrated post in pending: {}", post_uri);
                                                break;
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    ).await;

                    if timeout.is_err() {
                        tracing::trace!("Opportunistic hydration timed out for: {}", post_uri);
                    }
                });
            }

            // Check if batch should be flushed
            let should_flush = {
                let posts_by_thread = self.pending_batch.posts_by_thread.get(&thread_root);
                let batch_created = self.pending_batch.batch_timers.get(&thread_root);

                match (posts_by_thread, batch_created) {
                    (Some(posts), Some(created)) => {
                        let elapsed = created.elapsed();
                        let post_count = posts.len();

                        // Flush if: batch timeout (3 seconds) OR too many posts (10+)
                        elapsed > std::time::Duration::from_secs(3) || post_count >= 10
                    }
                    _ => false,
                }
            };

            if should_flush {
                tracing::info!("⏰ Flushing batch for thread: {}", thread_root);
                return self.flush_batch(&thread_root).await;
            }

            // Post is batched, no immediate notification
            return None;
        }

        // Not batchable - format single notification
        // This handles standalone posts, DMs, or posts that don't fit batching criteria
        tracing::debug!(
            "📝 Single notification for @{}: {}",
            item.handle,
            item.text.chars().take(50).collect::<String>()
        );

        self.format_single_notification(item).await

        /* OLD IMPLEMENTATION - COMMENTED OUT FOR REFERENCE
        let start_time = std::time::Instant::now();
        tracing::debug!(
            "🔄 format_notification started for post @{}: {}",
            item.handle,
            item.text.chars().take(50).collect::<String>()
        );

        // Check if this post has already been processed
        if self.pending_batch.processed_uris.contains_key(&item.uri) {
            tracing::debug!("Post already processed as part of a batch: {}", item.uri);
            return None;
        }

        // Check if this post should be batched
        if let Some(thread_root) = self.should_batch_post(item) {
            tracing::debug!("Post will be batched with thread: {}", thread_root);
            self.add_to_batch(item.clone(), thread_root).await;

            // Don't return a notification yet - wait for batch to complete
            return None;
        }

        // Clone the item so we can mutate it
        let mut item = item.clone();

        // Format based on post type
        let mut message = String::new();
        let mut reply_candidates = Vec::new();
        let mut memory_blocks = Vec::new();
        let mut added_memory_labels = std::collections::HashSet::<String>::new();

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
                // Build comprehensive thread context with siblings
                let thread_context = match self
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

                // Check for excluded users and keywords in thread context - bail if any are found
                // Check parent post author and content
                if let Some(parent) = &thread_context.parent {
                    if self
                        .filter
                        .exclude_dids
                        .contains(&parent.did.as_str().to_string())
                    {
                        tracing::debug!(
                            "❌ format_notification EXIT 1 (excluded parent user): @{} in {:?} - {}",
                            parent.did.as_str(),
                            start_time.elapsed(),
                            item.text.chars().take(30).collect::<String>()
                        );
                        return None;
                    }

                    // Check for excluded keywords in parent content
                    let text_lower = parent.text.to_lowercase();
                    for keyword in &self.filter.exclude_keywords {
                        if text_lower.contains(&keyword.to_lowercase()) {
                            tracing::debug!(
                                "❌ format_notification EXIT 2 (excluded keyword '{}'): in {:?} - {}",
                                keyword,
                                start_time.elapsed(),
                                item.text.chars().take(30).collect::<String>()
                            );
                            return None;
                        }
                    }
                    thread_users.push((
                        parent.handle.as_str().to_string(),
                        parent.did.as_str().to_string(),
                    ));
                }

                // Check sibling post authors
                for sibling in &thread_context.siblings {
                    if self
                        .filter
                        .exclude_dids
                        .contains(&sibling.did.as_str().to_string())
                    {
                        tracing::debug!(
                            "❌ format_notification EXIT 3 (excluded sibling user): @{} in {:?} - {}",
                            sibling.did.as_str(),
                            start_time.elapsed(),
                            item.text.chars().take(30).collect::<String>()
                        );
                        return None;
                    }
                    thread_users.push((
                        sibling.handle.as_str().to_string(),
                        sibling.did.as_str().to_string(),
                    ));
                }

                // Check authors from replies
                for replies in thread_context.replies_map.values() {
                    for reply in replies {
                        if self
                            .filter
                            .exclude_dids
                            .contains(&reply.did.as_str().to_string())
                        {
                            tracing::debug!(
                                "❌ format_notification EXIT 4 (excluded reply user): @{} in {:?} - {}",
                                reply.did.as_str(),
                                start_time.elapsed(),
                                item.text.chars().take(30).collect::<String>()
                            );
                            return None;
                        }
                        thread_users.push((
                            reply.handle.as_str().to_string(),
                            reply.did.as_str().to_string(),
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
                            {
                                let text = &sibling.text;
                                let features = &sibling.facets;
                                let embed_info = &sibling.embed;
                                let author_str = if let Some(name) = &sibling.display_name {
                                    format!("{} (@{})", name, sibling.handle.as_str())
                                } else {
                                    format!("@{}", sibling.handle.as_str())
                                };

                                // Check if this is the agent's post
                                let is_agent = agent_did
                                    .as_ref()
                                    .map(|did| sibling.did.as_str() == did)
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

                                let relative_time = {
                                    let now = chrono::Utc::now();
                                    let relative = now - sibling.created_at;
                                    format!(
                                        " - {} ago",
                                        format_duration(relative.to_std().unwrap())
                                    )
                                };

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

                                // Extract mentions from facets for checking
                                let mentions: Vec<_> = features
                                    .iter()
                                    .flat_map(|facet| &facet.features)
                                    .filter_map(|feature| match feature {
                                        FacetFeature::Mention { did } => Some(format!("@{}", did)),
                                        _ => None,
                                    })
                                    .collect();
                                mention_check_queue.append(&mut mentions.clone());

                                // Add as reply candidate
                                reply_candidates.push((
                                    sibling.uri.clone(),
                                    format!("@{}", sibling.handle.as_str()),
                                ));

                                // Show replies to this sibling if any
                                if let Some(replies) = thread_context.replies_map.get(&sibling.uri)
                                {
                                    for reply in replies.iter().take(3) {
                                        {
                                            let reply_text = &reply.text;
                                            let reply_embed_info = &reply.embed;
                                            let reply_author =
                                                if let Some(name) = &reply.display_name {
                                                    format!("{} (@{})", name, reply.handle.as_str())
                                                } else {
                                                    format!("@{}", reply.handle.as_str())
                                                };

                                            let is_agent_reply = agent_did
                                                .as_ref()
                                                .map(|did| reply.did.as_str() == did)
                                                .unwrap_or(false);
                                            let reply_prefix =
                                                if is_agent_reply { "[YOU] " } else { "" };

                                            let relative_time = {
                                                let now = chrono::Utc::now();
                                                let relative = now - reply.created_at;
                                                format!(
                                                    " - {} ago",
                                                    format_duration(relative.to_std().unwrap())
                                                )
                                            };

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
                                            reply_candidates.push((
                                                reply.uri.clone(),
                                                format!("@{}", reply.handle.as_str()),
                                            ));
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
            if let Some(embed_info) = &item.embed {
                // Show embeds if any
                let embed_display = embed_info.format_display("");
                // Add vertical bar and indentation prefix to each line
                for line in embed_display.lines() {
                    if !line.is_empty() {
                        message.push_str(&format!("│  {}\n", line));
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
                    tracing::info!(
                        "❌ format_notification EXIT 5 (excluded keyword '{}' in message): in {:?} - {}",
                        keyword,
                        start_time.elapsed(),
                        item.text.chars().take(30).collect::<String>()
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
                    "❌ format_notification EXIT 6 (no watched DID mentions): in {:?} - {}",
                    start_time.elapsed(),
                    item.text.chars().take(30).collect::<String>()
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

            users_to_check.dedup();

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
                        // Check if we've already added this memory block
                        if added_memory_labels.insert(memory_label.clone()) {
                            // Add existing block to our return list
                            memory_blocks.push((compact_label, existing_block));
                            message.push_str(&format!("\n\n📝 Memory exists: {}", memory_label));
                        }
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
                            // Check if we've already added this memory block
                            if added_memory_labels.insert(memory_label.clone()) {
                                message
                                    .push_str(&format!("\n\n📝 Memory created: {}", memory_label));

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

        tracing::debug!(
            "✅ format_notification SUCCESS: notification generated in {:?} for @{} - {}",
            start_time.elapsed(),
            item.handle,
            item.text.chars().take(30).collect::<String>()
        );
        Some((message, memory_blocks))
        */
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

            // Use our new from_record method to create the post
            let mut post_to_filter =
                BlueskyPost::from_record(&post, uri, event.did.to_string(), rcid);

            if should_include_post(&mut post_to_filter, &self.filter, &self.resolver).await {
                tracing::debug!(
                    "✓ Post passed filter, sending to agent: @{} - {}",
                    post_to_filter.handle,
                    post_to_filter.text.chars().take(100).collect::<String>()
                );
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
                    tracing::debug!("📤 Sending post to channel: @{}", post_to_filter.handle);
                    if let Err(e) = self.tx.send(Ok(event.clone())).await {
                        tracing::error!(
                            "Failed to send post to channel: {}. Channel likely closed, triggering reconnection.",
                            e
                        );
                        return Err(anyhow::anyhow!("Channel closed: {}", e));
                    }

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
            urls: vec![], // TODO: Extract URLs when available
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
                urls: vec![], // TODO: Extract URLs when available
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
                        urls: vec![], // TODO: Extract URLs when available
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
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum EmbedInfo {
    Images {
        count: usize,
        alt_texts: Vec<String>,
        urls: Vec<String>, // For future multimodal support
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
    /// Append embed info to buffer with given indentation
    fn append_to_buffer(&self, buf: &mut String, indent: &str) {
        match self {
            EmbedInfo::Images {
                count, alt_texts, ..
            } => {
                buf.push_str(&format!("{}   [📸 {} image(s)]\n", indent, count));
                for alt_text in alt_texts {
                    if !alt_text.is_empty() {
                        buf.push_str(&format!("{}    alt: {}\n", indent, alt_text));
                    }
                }
            }
            EmbedInfo::External {
                uri,
                title,
                description,
            } => {
                buf.push_str(&format!("{}   [🔗 Link Card]\n", indent));
                if !title.is_empty() {
                    buf.push_str(&format!("{}    {}\n", indent, title));
                }
                if !description.is_empty() {
                    buf.push_str(&format!("{}    {}\n", indent, description));
                }
                buf.push_str(&format!("{}    {}\n", indent, uri));
            }
            EmbedInfo::Quote {
                uri,
                author_handle,
                author_display_name,
                text,
                created_at,
                ..
            } => {
                buf.push_str(&format!("{}   ┌─ Quote ─────\n", indent));
                let author = if let Some(name) = author_display_name {
                    format!("{} (@{})", name, author_handle)
                } else {
                    format!("@{}", author_handle)
                };
                buf.push_str(&format!("{}   │ {}: {}\n", indent, author, text));
                if let Some(time) = created_at {
                    let interval = Utc::now() - *time;
                    let relative =
                        format_duration(interval.to_std().unwrap_or(Duration::from_secs(0)));
                    buf.push_str(&format!("{}   │ {} ago\n", indent, relative));
                }
                buf.push_str(&format!("{}   │ 🔗 {}\n", indent, uri));
                buf.push_str(&format!("{}   └──────────\n", indent));
            }
            EmbedInfo::QuoteWithMedia { quote, media } => {
                quote.append_to_buffer(buf, indent);
                media.append_to_buffer(buf, indent);
            }
        }
    }

    /// Format embed info for display with given indentation (convenience wrapper)
    fn format_display(&self, indent: &str) -> String {
        let mut output = String::new();
        self.append_to_buffer(&mut output, indent);
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
                post.resolve_handle(resolver).await;
                return true;
            }
        }
    }

    // 3. FRIENDS LIST - bypass all other checks
    if filter.friends.contains(&post.did) {
        // Friends always pass through
        // Still need to resolve handle though
        post.resolve_handle(resolver).await;
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
                post.resolve_handle(resolver).await;
                return true;
            } else if filter.dids.is_empty() || filter.dids.contains(&post.did) {
                // Only accept mentions from allowlisted DIDs (or if no allowlist)
                post.resolve_handle(resolver).await;
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

    // Post passed all filters - resolve handle before returning
    post.resolve_handle(resolver).await;
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
            indexed_at: Some(Utc::now()),
            hydration: Default::default(),
            like_count: Some(0),
            reply_count: Some(0),
            repost_count: Some(0),
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
            indexed_at: Some(Utc::now()),
            hydration: Default::default(),
            like_count: Some(0),
            reply_count: Some(0),
            repost_count: Some(0),
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
