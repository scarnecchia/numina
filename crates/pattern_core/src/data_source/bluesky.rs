use std::str::FromStr;
use std::sync::Arc;
use std::{collections::HashMap, sync::Mutex};

use super::{
    StreamBuffer,
    traits::{DataSource, DataSourceMetadata, DataSourceStatus, Searchable, StreamEvent},
};
use crate::error::Result;
use async_trait::async_trait;
use atrium_api::app::bsky::feed::post::{RecordLabelsRefs, ReplyRefData};
use atrium_api::app::bsky::richtext::facet::MainFeaturesItem;
use atrium_api::com::atproto::repo::strong_ref::MainData;
use atrium_api::types::Union;
use atrium_api::types::string::{Cid, Did};
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
#[derive(Debug)]
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
}

impl BlueskyFirehoseSource {
    pub async fn new(source_id: impl Into<String>, endpoint: impl Into<String>) -> Self {
        Self {
            source_id: source_id.into(),
            endpoint: endpoint.into(),
            filter: BlueskyFilter::default(),
            current_cursor: None,
            stats: SourceStats::default(),
            buffer: None,
            notifications_enabled: true,
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

    fn format_notification(&self, item: &Self::Item) -> Option<String> {
        // Format based on post type
        let mut message = String::new();

        // Header with author
        message.push_str(&format!("💬 @{}", item.handle));

        // Add context for replies/mentions
        if let Some(_reply) = &item.reply {
            message.push_str(" replied");
        } else if item.mentions(&self.source_id) {
            message.push_str(" mentioned you");
        }

        message.push_str(":\n\n");

        // Text preview (truncate if too long)
        let text_preview = if item.text.len() > 280 {
            format!("{}...", &item.text[..280])
        } else {
            item.text.clone()
        };
        message.push_str(&text_preview);

        // Add link
        message.push_str(&format!("\n\n🔗 {}", item.uri));

        // Add image indicator if present
        if item.has_images() {
            let alt_texts = item.image_alt_texts();
            if !alt_texts.is_empty() {
                message.push_str(&format!("\n📸 {} image(s)", alt_texts.len()));
            }
        }

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

            let mut post_to_filter = BlueskyPost {
                uri,
                did: event.did.to_string(),
                cid: rcid,
                handle: event.did.to_string(), // temporary, need to do handle resolution
                text: post.text,
                created_at: chrono::DateTime::parse_from_rfc3339(post.created_at.as_str())
                    .expect("incorrect time format")
                    .to_utc(),
                reply: post.reply.map(|r| r.data),
                embed: post
                    .embed
                    .map(|e| serde_json::to_value(e).expect("should be reasonably serializable")),
                langs: post
                    .langs
                    .map(|l| l.iter().map(|l| format!("{:?}", l)).collect())
                    .unwrap_or_default(),
                labels: post.labels.map(label_convert).unwrap_or_default(),
                facets: post
                    .facets
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
                    .unwrap_or_default(),
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

fn label_convert(l: Union<RecordLabelsRefs>) -> Vec<String> {
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
