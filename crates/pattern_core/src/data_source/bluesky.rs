use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use atrium_api::app::bsky::feed::post::ReplyRefData;
use atrium_api::com::atproto::repo::strong_ref::MainData;
use chrono::{DateTime, Utc};
use futures::Stream;
use rocketman::{
    connection::JetstreamConnection,
    handler,
    ingestion::LexiconIngestor,
    options::JetstreamOptions,
    types::event::{Commit, Event, Operation},
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio_stream::wrappers::UnboundedReceiverStream;

use super::{
    StreamBuffer,
    traits::{DataSource, DataSourceMetadata, DataSourceStatus, StreamEvent},
};
use crate::error::Result;

/// A post from Bluesky/ATProto
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlueskyPost {
    pub uri: String,
    pub did: String,    // Author DID
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
}

impl BlueskyFirehoseSource {
    pub fn new(source_id: impl Into<String>, endpoint: impl Into<String>) -> Self {
        Self {
            source_id: source_id.into(),
            endpoint: endpoint.into(),
            filter: BlueskyFilter::default(),
            current_cursor: None,
            stats: SourceStats::default(),
            buffer: None,
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
        // Build options using builder
        let builder = JetstreamOptions::builder();

        // Apply collection filters (NSIDs map to collections)
        let collections = if !self.filter.nsids.is_empty() {
            self.filter.nsids.clone()
        } else {
            vec!["app.bsky.feed.post".to_string()]
        };

        // Build options with all settings
        let options = if let Some(cursor) = from {
            builder
                .cursor(cursor.time_us.to_string())
                .wanted_collections(collections)
                .build()
        } else {
            builder.wanted_collections(collections).build()
        };

        // Create connection - new() is sync
        let connection = JetstreamConnection::new(options);

        // Create channel for processed events
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let filter = self.filter.clone();
        let source_id = self.source_id.clone();
        let buffer = self.buffer.clone();

        // Spawn task to consume events
        tokio::spawn(async move {
            // Connect and get message channel
            let cursor_arc = std::sync::Arc::new(std::sync::Mutex::new(None::<u64>));
            if let Err(e) = connection.connect(cursor_arc.clone()).await {
                let _ = tx.send(Err(crate::CoreError::DataSourceError {
                    source_name: source_id.clone(),
                    operation: "connect".to_string(),
                    cause: e.to_string(),
                }));
                return;
            }

            let msg_rx = connection.get_msg_rx();
            let reconnect_tx = connection.get_reconnect_tx();

            // Create our ingestor that sends posts to our channel
            let post_ingestor = PostIngestor {
                tx: tx.clone(),
                filter,
                buffer,
            };

            let mut ingestors: HashMap<String, Box<dyn LexiconIngestor + Send + Sync>> =
                HashMap::new();
            ingestors.insert("app.bsky.feed.post".to_string(), Box::new(post_ingestor));

            // Process messages from jetstream
            while let Ok(message) = msg_rx.recv_async().await {
                if let Err(e) = handler::handle_message(
                    message,
                    &ingestors,
                    reconnect_tx.clone(),
                    cursor_arc.clone(),
                )
                .await
                {
                    tracing::warn!("Error processing message: {}", e);
                    let _ = tx.send(Err(crate::CoreError::DataSourceError {
                        source_name: source_id.clone(),
                        operation: "process".to_string(),
                        cause: e.to_string(),
                    }));
                }
            }
        });

        Ok(Box::new(UnboundedReceiverStream::new(rx)))
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
}

fn parse_bluesky_post<T>(commit: &Commit<T>, event: &Event<T>) -> Result<BlueskyPost>
where
    T: serde::de::DeserializeOwned + serde::Serialize,
{
    // Extract post data from commit record
    let record_value = match &commit.record {
        Some(rec) => serde_json::to_value(rec).unwrap_or(serde_json::Value::Null),
        None => serde_json::Value::Null,
    };

    let record_obj = record_value
        .as_object()
        .ok_or_else(|| crate::CoreError::DataSourceError {
            source_name: "bluesky".to_string(),
            operation: "parse_post".to_string(),
            cause: "Record is not an object".to_string(),
        })?;

    // Extract text (required field)
    let text = record_obj
        .get("text")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    // Extract languages
    let langs = record_obj
        .get("langs")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str())
                .map(|s| s.to_string())
                .collect()
        })
        .unwrap_or_default();

    // Extract reply information - parse full reply structure
    let reply = record_obj
        .get("reply")
        .and_then(|v| v.as_object())
        .and_then(|reply_obj| {
            let root = reply_obj
                .get("root")
                .and_then(|r| serde_json::from_value::<MainData>(r.clone()).ok())?;
            let parent = reply_obj
                .get("parent")
                .and_then(|p| serde_json::from_value::<MainData>(p.clone()).ok())?;
            Some(ReplyRefData {
                root: root.into(),
                parent: parent.into(),
            })
        });

    // Extract embed as-is
    // TODO: Parse embed structure to extract:
    // - Alt text from images (for accessibility)
    // - Image data/references (for vision-capable agents)
    // - External link cards
    // - Quote posts
    let embed = record_obj.get("embed").cloned();

    // Parse facets for rich text features
    let facets = record_obj
        .get("facets")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|facet| serde_json::from_value::<Facet>(facet.clone()).ok())
                .collect()
        })
        .unwrap_or_default();

    // Extract labels (if present)
    let labels = record_obj
        .get("labels")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str())
                .map(|s| s.to_string())
                .collect()
        })
        .unwrap_or_default();

    // Parse created_at timestamp
    let created_at = record_obj
        .get("createdAt")
        .and_then(|v| v.as_str())
        .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
        .map(|dt| dt.with_timezone(&Utc))
        .unwrap_or_else(|| Utc::now());

    // Build post URI
    let uri = format!("at://{}/{}/{}", event.did, commit.collection, commit.rkey);

    Ok(BlueskyPost {
        uri,
        did: event.did.clone(),
        handle: event.did.clone(), // TODO: Resolve handle from DID - rocketman might provide this
        text,
        created_at,
        reply,
        embed,
        langs,
        labels,
        facets,
    })
}

/// Ingestor that processes Bluesky posts and sends them to our channel
struct PostIngestor {
    tx: tokio::sync::mpsc::UnboundedSender<Result<StreamEvent<BlueskyPost, BlueskyFirehoseCursor>>>,
    filter: BlueskyFilter,
    buffer: Option<Arc<parking_lot::Mutex<StreamBuffer<BlueskyPost, BlueskyFirehoseCursor>>>>,
}

#[async_trait]
impl LexiconIngestor for PostIngestor {
    async fn ingest(&self, event: Event<serde_json::Value>) -> anyhow::Result<()> {
        // Only process commit events for posts
        if let Some(ref commit) = event.commit {
            if commit.collection == "app.bsky.feed.post"
                && matches!(commit.operation, Operation::Create)
            {
                // Parse the post from the event
                match parse_bluesky_post(&commit, &event) {
                    Ok(post) => {
                        // Apply filters
                        if should_include_post(&post, &self.filter) {
                            let cursor = BlueskyFirehoseCursor {
                                seq: event.time_us.unwrap_or(0), // TODO: Get actual seq
                                time_us: event.time_us.unwrap_or(0),
                            };

                            let stream_event = StreamEvent {
                                item: post,
                                cursor,
                                timestamp: Utc::now(),
                            };

                            // Add to buffer if present
                            if let Some(ref buf) = self.buffer {
                                buf.lock().push(stream_event.clone());
                            }

                            // Send to stream
                            if self.tx.send(Ok(stream_event)).is_err() {
                                tracing::debug!("Receiver dropped, stopping Bluesky ingestor");
                            }
                        }
                    }
                    Err(e) => {
                        tracing::warn!("Failed to parse post: {}", e);
                    }
                }
            }
        }
        Ok(())
    }
}

fn should_include_post(post: &BlueskyPost, filter: &BlueskyFilter) -> bool {
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

    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_filter_post() {
        let post = BlueskyPost {
            uri: "at://did:plc:example/app.bsky.feed.post/123".to_string(),
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

        // Test keyword filter
        let filter = BlueskyFilter {
            keywords: vec!["rust".to_string()],
            ..Default::default()
        };
        assert!(should_include_post(&post, &filter));

        // Test language filter
        let filter = BlueskyFilter {
            languages: vec!["en".to_string()],
            ..Default::default()
        };
        assert!(should_include_post(&post, &filter));

        // Test DID filter
        let filter = BlueskyFilter {
            dids: vec!["did:plc:other".to_string()],
            ..Default::default()
        };
        assert!(!should_include_post(&post, &filter));
    }

    #[test]
    fn test_mention_filter() {
        let mut post = BlueskyPost {
            uri: "at://did:plc:author/app.bsky.feed.post/456".to_string(),
            did: "did:plc:author".to_string(),
            handle: "author.bsky.social".to_string(),
            text: "Hey @alice.bsky.social check this out!".to_string(),
            created_at: Utc::now(),
            reply: None,
            embed: None,
            langs: vec!["en".to_string()],
            labels: vec![],
            facets: vec![],
        };

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
        assert!(should_include_post(&post, &filter));

        // Test with different mention whitelist - should exclude
        let filter = BlueskyFilter {
            mentions: vec!["did:plc:bob".to_string()],
            ..Default::default()
        };
        assert!(!should_include_post(&post, &filter));

        // Test mentions() helper
        assert!(post.mentions("did:plc:alice"));
        assert!(!post.mentions("did:plc:bob"));
    }
}
