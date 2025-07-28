# Bluesky/ATProto Integration Plan

This document outlines the plan for integrating Bluesky's Jetstream service with Pattern agents using the rocketman crate.

## Overview

We'll implement a `BlueskyFirehoseSource` that consumes the Bluesky Jetstream firehose using rocketman and feeds posts/events to Pattern agents through the data source abstraction.

## Implementation Plan

### Phase 1: Create BlueskyFirehoseSource

#### 1.1 Project Structure
```
pattern/crates/pattern_core/src/data_source/
├── bluesky.rs          # BlueskyFirehoseSource implementation
├── mod.rs              # Export the new module
└── ...
```

#### 1.2 Dependencies
Add to `pattern_core/Cargo.toml`:
```toml
[dependencies]
rocketman = "0.2.3"
tokio-stream = "0.1"  # For stream utilities
```

#### 1.3 Core Types
```rust
// pattern_core/src/data_source/bluesky.rs

use rocketman::{
    connection::JetstreamConnection,
    types::event::{Event, Kind, Operation, Commit},
    options::JetstreamOptions,
};

#[derive(Debug, Clone)]
pub struct BlueskyFirehoseSource {
    source_id: String,
    connection: Option<JetstreamConnection>,
    options: JetstreamOptions,
    filter: BlueskyFilter,
    current_cursor: Option<BlueskyFirehoseCursor>,
    stats: SourceStats,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlueskyPost {
    pub uri: String,
    pub did: String,         // Author DID
    pub handle: String,      // Author handle
    pub text: String,
    pub created_at: DateTime<Utc>,
    pub reply_to: Option<String>,
    pub embed: Option<serde_json::Value>,
    pub langs: Vec<String>,
    pub labels: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlueskyFirehoseCursor {
    pub seq: u64,        // Jetstream sequence number
    pub time_us: u64,    // Unix microseconds timestamp
}

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
}

#[derive(Debug, Default)]
struct SourceStats {
    events_received: u64,
    posts_processed: u64,
    errors: u64,
    last_seq: Option<u64>,
}
```

#### 1.4 DataSource Implementation
```rust
#[async_trait]
impl DataSource for BlueskyFirehoseSource {
    type Item = BlueskyPost;
    type Filter = BlueskyFilter;
    type Cursor = BlueskyFirehoseCursor;

    fn source_id(&self) -> &str {
        &self.source_id
    }

    async fn pull(&mut self, _limit: usize, _after: Option<Self::Cursor>) -> Result<Vec<Self::Item>> {
        // Jetstream is push-only, so pull returns empty
        // Could potentially return buffered items in the future
        Ok(vec![])
    }

    async fn subscribe(
        &mut self,
        from: Option<Self::Cursor>,
    ) -> Result<Box<dyn Stream<Item = Result<StreamEvent<Self::Item, Self::Cursor>>> + Send + Unpin>> {
        // 1. Configure rocketman connection options
        let mut options = self.options.clone();
        
        // Set cursor if resuming
        if let Some(cursor) = from {
            options = options.cursor(cursor.time_us);
        }
        
        // Apply filters
        if !self.filter.nsids.is_empty() {
            for nsid in &self.filter.nsids {
                options = options.want_nsid(nsid);
            }
        }
        
        // 2. Create connection
        let connection = JetstreamConnection::new(options).await?;
        self.connection = Some(connection.clone());
        
        // 3. Start consuming events
        let filter = self.filter.clone();
        let stream = connection
            .consume()
            .await?
            .filter_map(move |event_result| {
                // Convert rocketman events to our StreamEvent
                match event_result {
                    Ok(event) => handle_jetstream_event(event, &filter),
                    Err(e) => Some(Err(e.into())),
                }
            });
        
        Ok(Box::new(stream))
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
            status: if self.connection.is_some() {
                DataSourceStatus::Active
            } else {
                DataSourceStatus::Disconnected
            },
            items_processed: self.stats.posts_processed,
            last_item_time: self.current_cursor.as_ref().map(|c| {
                DateTime::from_timestamp_micros(c.time_us as i64).unwrap_or_default()
            }),
            error_count: self.stats.errors,
            custom: HashMap::from([
                ("events_received".to_string(), json!(self.stats.events_received)),
                ("last_seq".to_string(), json!(self.stats.last_seq)),
            ]),
        }
    }
}
```

#### 1.5 Event Processing
```rust
fn handle_jetstream_event(
    event: Event,
    filter: &BlueskyFilter,
) -> Option<Result<StreamEvent<BlueskyPost, BlueskyFirehoseCursor>>> {
    // Extract post from commit if it matches filters
    match event.kind {
        Kind::Commit => {
            if let Some(commit) = event.commit {
                // Check if this is a post creation
                if commit.collection == "app.bsky.feed.post" 
                    && matches!(commit.operation, Operation::Create) {
                    
                    // Parse the post data
                    if let Ok(post) = parse_bluesky_post(&commit, &event) {
                        // Apply filters
                        if should_include_post(&post, filter) {
                            let cursor = BlueskyFirehoseCursor {
                                seq: event.time_us, // Assuming this maps to seq
                                time_us: event.time_us,
                            };
                            
                            return Some(Ok(StreamEvent {
                                item: post,
                                cursor,
                                timestamp: Utc::now(),
                            }));
                        }
                    }
                }
            }
        }
        _ => {} // Handle other event types as needed
    }
    
    None
}

fn parse_bluesky_post(commit: &Commit, event: &Event) -> Result<BlueskyPost> {
    // Extract post data from commit record
    // This will need to parse the actual ATProto record format
    todo!("Parse commit.record into BlueskyPost")
}

fn should_include_post(post: &BlueskyPost, filter: &BlueskyFilter) -> bool {
    // Apply all filters
    
    // DID filter
    if !filter.dids.is_empty() && !filter.dids.contains(&post.did) {
        return false;
    }
    
    // Keyword filter
    if !filter.keywords.is_empty() {
        let text_lower = post.text.to_lowercase();
        if !filter.keywords.iter().any(|kw| text_lower.contains(&kw.to_lowercase())) {
            return false;
        }
    }
    
    // Language filter
    if !filter.languages.is_empty() 
        && !post.langs.iter().any(|lang| filter.languages.contains(lang)) {
        return false;
    }
    
    true
}
```

### Phase 2: Prompt Templates for Bluesky

Add specialized templates in `PromptTemplate::with_defaults()`:

```rust
// New post notification
registry.register(
    PromptTemplate::new(
        "bluesky_post",
        r#"New post from @{{ handle }} on Bluesky:
{{ text }}

[{{ uri }}]"#,
    )?
    .with_description("Bluesky post notification"),
);

// Reply notification
registry.register(
    PromptTemplate::new(
        "bluesky_reply",
        r#"@{{ handle }} replied to {{ reply_to }}:
{{ text }}

[{{ uri }}]"#,
    )?
    .with_description("Bluesky reply notification"),
);

// Mention notification
registry.register(
    PromptTemplate::new(
        "bluesky_mention",
        r#"You were mentioned by @{{ handle }}:
{{ text }}

[{{ uri }}]"#,
    )?
    .with_description("Bluesky mention notification"),
);
```

### Phase 3: BlueskyEndpoint for Message Router

#### Key Dependencies from Gork

Based on the Gork implementation, we'll need:
- `atrium-api` for proper type definitions (already have)
- `bsky-sdk` for the BskyAgent posting interface
- Proper OAuth/session handling

```rust
// From Gork's setup
use bsky_sdk::BskyAgent;

async fn setup_bsky_sess() -> anyhow::Result<BskyAgent> {
    let agent = BskyAgent::builder().build().await?;
    let res = agent
        .login(std::env::var("ATP_USER")?, std::env::var("ATP_PASSWORD")?)
        .await?;
    Ok(agent)
}

// Creating a post with proper reply threading
self.agent
    .create_record(atrium_api::app::bsky::feed::post::RecordData {
        created_at: Datetime::now(),
        embed: None,
        entities: None,
        facets: None,
        labels: None,
        langs: None,
        reply,  // This is the critical part!
        tags: None,
        text: "your text here".to_string(),
    })
    .await?;
```

#### 3.1 Endpoint Implementation
```rust
// pattern_core/src/context/message_router/endpoints/bluesky.rs

use atrium_api::client::AtpClient;

#[derive(Debug, Clone)]
pub struct BlueskyEndpoint {
    client: Arc<AtpClient>,
    did: String,
    handle: String,
}

#[async_trait]
impl MessageEndpoint for BlueskyEndpoint {
    fn endpoint_type(&self) -> &'static str {
        "bluesky"
    }

    async fn send(
        &self,
        message: Message,
        metadata: Option<serde_json::Value>,
    ) -> Result<()> {
        // Extract text from message
        let text = extract_text_from_message(&message);
        
        // CRITICAL: Handle reply threading correctly
        // Check metadata for reply_to information
        let reply = if let Some(meta) = metadata {
            if let Some(reply_to_uri) = meta.get("reply_to").and_then(|v| v.as_str()) {
                // Parse the reply_to URI to extract CID
                // Format: at://did/collection/rkey
                
                // For replies, MUST set BOTH parent and root
                Some(ReplyRefData {
                    parent: MainData {
                        cid: reply_to_cid,
                        uri: reply_to_uri.to_string(),
                    },
                    root: MainData {
                        // If this is already a reply thread, get the original root
                        // Otherwise, root = parent (first reply in thread)
                        cid: root_cid,
                        uri: root_uri,
                    },
                })
            } else {
                None
            }
        } else {
            None
        };
        
        // Create post using atrium-api
        self.client
            .create_record(CreateRecordInput {
                collection: "app.bsky.feed.post".to_string(),
                record: PostRecord {
                    text,
                    created_at: Utc::now(),
                    reply,
                    ..Default::default()
                },
            })
            .await?;
        
        Ok(())
    }

    async fn receive(&self) -> Result<Option<Message>> {
        // Could poll for mentions/replies
        // But better to use firehose for real-time
        Ok(None)
    }
}
```

#### ⚠️ CRITICAL: Reply Threading

When implementing replies, you MUST set BOTH `parent` and `root` references:
- `parent`: The immediate post being replied to
- `root`: The original post that started the thread

If replying to a non-reply post: `root = parent`
If replying to a reply: `root = original thread starter` (NOT the parent!)

**Common fuck-ups:**
1. Only setting `parent` - breaks threading
2. Setting `root = parent` when parent is already a reply - creates orphaned reply
3. Not tracking the original root through the thread - loses context

If you set root to the same as parent when there's an actual root up the chain, your reply appears as a reply to nothing. The thread breaks and users can't follow the conversation.

#### 3.2 Tool Operation for Posting
```rust
// Add to DataSourceOperation enum
PostToBluesky {
    text: String,
    reply_to: Option<String>,
    embed: Option<serde_json::Value>,
}

// Handle in DataSourceTool::execute
DataSourceOperation::PostToBluesky { text, reply_to, embed } => {
    // Route through message router with BlueskyEndpoint
    let target = MessageTarget {
        target_type: TargetType::Channel,
        target_id: Some("bluesky".to_string()),
    };
    
    let message = Message::user(text);
    
    self.coordinator
        .read()
        .await
        .agent_router
        .send_message(target, message, embed)
        .await?;
    
    Ok(DataSourceOutput {
        success: true,
        result: DataSourceResult::Success {
            message: "Posted to Bluesky".to_string(),
        },
    })
}
```

### Phase 4: Integration Example

```rust
// Setting up Bluesky monitoring for an agent
let bluesky_source = BlueskyFirehoseSource::new(
    "bluesky_mentions".to_string(),
    JetstreamOptions::default()
        .endpoint("wss://jetstream.atproto.tools/subscribe"),
);

// Filter for mentions of the agent
let filter = BlueskyFilter {
    nsids: vec!["app.bsky.feed.post".to_string()],
    keywords: vec!["@myagent.bsky.social".to_string()],
    ..Default::default()
};

// Add to coordinator
coordinator.add_source(
    bluesky_source.with_filter(filter),
    BufferConfig {
        max_items: 1000,
        max_age: Duration::from_secs(3600),
        persist_to_db: true,
        index_content: false,
    },
    "bluesky_mention".to_string(),
).await?;
```

## Testing Strategy

1. **Unit Tests**
   - Test filter logic
   - Test cursor serialization/deserialization
   - Test event parsing

2. **Integration Tests**
   - Mock rocketman connection
   - Test full pipeline from event to agent notification

3. **Live Testing**
   - Create test account on Bluesky
   - Monitor specific hashtags
   - Test posting functionality

## Dependencies Summary

All dependencies ready:
- `rocketman = "0.2.3"` - Jetstream consumer ✅
- `atrium-api = "0.25.3"` - ATProto type definitions ✅
- `bsky-sdk` - For BskyAgent and posting functionality ✅

## Implementation Status ✅

### Completed (2025-07-28)

1. **BlueskyFirehoseSource** - Fully implemented with rocketman integration
   - Custom `PostIngestor` implementing `LexiconIngestor` trait
   - Proper WebSocket message handling with `handle_message`
   - Cursor management for stream resumption
   - Buffer support for historical data

2. **Rich Text Support** - Facets for mentions, links, and hashtags
   - `FacetFeature` enum with proper ATProto type tags
   - Helper methods: `mentions()`, `mentioned_dids()`
   - Byte slice indices for text annotations

3. **Advanced Filtering** 
   - Author DID filtering
   - Keyword matching (case-insensitive)
   - Language filtering
   - **Mention whitelist** - Only process posts mentioning specific DIDs
   - All filters properly tested

4. **Event Parsing**
   - Proper extraction from ATProto record format
   - Support for all post fields: text, createdAt, facets, reply, embed, langs, labels
   - Timestamp parsing from RFC3339 format
   - URI construction: `at://did/collection/rkey`

5. **Prompt Templates**
   - `bluesky_post` - New post notifications
   - `bluesky_reply` - Reply notifications
   - `bluesky_mention` - Mention notifications

### Still TODO

1. **Test with live Jetstream connection** - Need to connect to actual firehose
2. **Add BlueskyEndpoint for posting** - Implement message router endpoint (CRITICAL: handle reply threading correctly!)
3. **Handle resolution** - Currently using DID as handle placeholder
4. **Create example agent** - Build agent that monitors and responds to mentions
5. **Parse embed content** - Extract alt text from images for accessibility, potentially image data for vision agents
6. **Add rate limiting** - Prevent spam/abuse with per-DID cooldowns