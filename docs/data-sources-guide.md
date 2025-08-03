# Data Sources Configuration Guide

This guide explains how to configure and use data sources with Pattern agents to enable them to monitor and respond to external data streams.

## Overview

Pattern's data source system allows agents to:
- Monitor file systems for changes
- Create indexed knowledge bases with semantic search
- Connect to the Bluesky firehose for social media monitoring
- Process data through customizable templates
- Manage multiple sources simultaneously

## Quick Start

### 1. Simple File Monitoring

Monitor a directory and get notified when files change:

```rust
use pattern_core::data_source::monitor_directory;

let coordinator = monitor_directory(
    agent_id,
    agent_name,
    db,
    "/path/to/watch",
).await?;
```

### 2. Indexed Knowledge Base

Create a searchable knowledge base from documents:

```rust
use pattern_core::data_source::create_knowledge_base;

let coordinator = create_knowledge_base(
    agent_id,
    agent_name,
    db,
    "/path/to/documents",
    embedding_provider,
).await?;

// Search the knowledge base
let results = coordinator.search_source(
    "file:/path/to/documents",
    "search query",
    10  // max results
).await?;
```

### 3. Bluesky Monitoring

Monitor Bluesky for mentions and keywords:

```rust
use pattern_core::data_source::monitor_bluesky_mentions;

let coordinator = monitor_bluesky_mentions(
    agent_id,
    agent_name,
    db,
    "your.handle",
    Some(agent_handle),  // For enhanced notifications
).await?;
```

## Advanced Configuration

### Using the DataSourceBuilder

For complex setups with multiple sources:

```rust
use pattern_core::data_source::{DataSourceBuilder, BlueskyFilter};

let coordinator = DataSourceBuilder::new()
    // Add file sources
    .with_file_source("/data/static", false, true)      // indexed
    .with_file_source("/data/inbox", true, false)       // watched
    .with_templated_file_source(
        "/data/reports",
        "/templates/report.j2",
        true,
        false
    )
    // Add Bluesky sources
    .with_bluesky_source(
        "social_monitor".to_string(),
        BlueskyFilter {
            mentions: vec!["pattern.bsky.social".to_string()],
            keywords: vec!["ADHD".to_string()],
            languages: vec!["en".to_string()],
            ..Default::default()
        },
        true  // use agent handle
    )
    .build(
        agent_id,
        agent_name,
        db,
        Some(embedding_provider),
        Some(agent_handle),
    )
    .await?;
```

### Bluesky Filters

Configure what content to monitor:

```rust
BlueskyFilter {
    // AT Protocol NSIDs to monitor
    nsids: vec!["app.bsky.feed.post".to_string()],
    
    // Handles to watch for mentions
    mentions: vec!["alice.bsky.social".to_string()],
    
    // DIDs to follow
    dids: vec!["did:plc:abc123...".to_string()],
    
    // Keywords to match
    keywords: vec!["ADHD".to_string(), "executive function".to_string()],
    
    // Language codes
    languages: vec!["en".to_string(), "es".to_string()],
}
```

## Managing Data Sources

### Source Operations

```rust
// List all sources
let sources = coordinator.list_sources().await;
for (id, source_type) in sources {
    println!("{}: {}", id, source_type);
}

// Read from a source
let items = coordinator.read_source(
    "file:/data/inbox",
    10,      // limit
    None     // cursor
).await?;

// Search within a source
let results = coordinator.search_source(
    "file:/data/knowledge",
    "executive function strategies",
    5
).await?;

// Pause/resume monitoring
coordinator.pause_source("bluesky_monitor").await?;
coordinator.resume_source("bluesky_monitor").await?;

// Get buffer statistics
let stats = coordinator.get_buffer_stats("file:/data/inbox").await?;
```

### Dynamic Source Management

Add sources at runtime:

```rust
use pattern_core::data_source::{add_file_source, add_bluesky_source};

// Add a file source
add_file_source(
    &mut coordinator,
    "/new/path",
    true,    // watch
    false,   // indexed
    None     // template
).await?;

// Add a Bluesky source
add_bluesky_source(
    &mut coordinator,
    "new_monitor".to_string(),
    None,    // default endpoint
    filter,
    agent_handle
).await?;
```

## Integration with Agents

### Agent Handle for Enhanced Notifications

When you provide an agent handle to data sources, they can:
- Access agent memory for context
- Create memory blocks for tracked entities
- Fetch additional context (e.g., Bluesky thread history)
- Provide richer notifications to the agent

```rust
let agent_handle = agent.memory();

// Bluesky source with handle can fetch thread context
let coordinator = monitor_bluesky_mentions(
    agent_id,
    agent_name,
    db,
    handle,
    Some(agent_handle),  // Enhanced features enabled
).await?;
```

### Data Flow

1. **Source Detection**: Data sources monitor for new items
2. **Template Processing**: Items are formatted using Jinja2 templates
3. **Agent Notification**: Formatted messages sent to agent
4. **Agent Response**: Agent processes and potentially responds
5. **Memory Update**: Important information stored in agent memory

## File Storage Modes

### Ephemeral Mode
- Default for watched directories
- Items processed and discarded
- No indexing or search capability
- Low memory overhead

### Indexed Mode
- Requires embedding provider
- Documents chunked and embedded
- Full semantic search support
- Higher memory/compute requirements

```rust
// Ephemeral - good for transient data
.with_file_source("/tmp/uploads", true, false)

// Indexed - good for knowledge bases
.with_file_source("/data/docs", false, true)
```

## Template System

Templates control how data is presented to agents:

```jinja2
{# /templates/bluesky_post.j2 #}
New post from @{{ author_handle }}:

{{ post_text }}

{% if is_reply %}
In reply to: {{ parent_author }}
{% endif %}

{% if mentions %}
Mentions: {{ mentions | join(", ") }}
{% endif %}
```

## Performance Considerations

1. **Buffer Configuration**
   - Adjust buffer sizes based on data volume
   - Monitor buffer stats to detect backpressure
   - Use pagination for large result sets

2. **Indexing Strategy**
   - Only index stable, searchable content
   - Use ephemeral mode for transient data
   - Balance chunk size vs search granularity

3. **Notification Frequency**
   - Batch related items when possible
   - Use rate limiting for high-volume sources
   - Consider agent processing capacity

## Troubleshooting

### Common Issues

1. **"No embedding provider" warning**
   - Indexed mode requires embedding provider
   - Falls back to ephemeral automatically
   - Provide embeddings for search capability

2. **Buffer overflow**
   - Check buffer stats regularly
   - Increase buffer size if needed
   - Consider sampling high-volume streams

3. **Template errors**
   - Validate Jinja2 syntax
   - Ensure all variables are available
   - Check template file permissions

### Debug Commands

```rust
// Check source status
let sources = coordinator.list_sources().await;

// Monitor buffer health
let stats = coordinator.get_buffer_stats(source_id).await?;
println!("Buffered: {}, Dropped: {}", 
    stats["buffered_count"], 
    stats["dropped_count"]
);

// Test source connectivity
let test_items = coordinator.read_source(source_id, 1, None).await?;
```

## Best Practices

1. **Start Simple**: Begin with basic file monitoring, add complexity gradually
2. **Monitor Performance**: Track buffer stats and processing times
3. **Use Templates**: Keep agent prompts consistent and maintainable
4. **Index Wisely**: Only index content that needs semantic search
5. **Handle Errors**: Implement retry logic for network sources
6. **Test Locally**: Use file sources to prototype before connecting to live streams

## Example Use Cases

### ADHD Task Inbox
Monitor a directory where users drop task files, automatically parsing and organizing them.

### Knowledge Management
Index documentation and notes for instant retrieval during conversations.

### Social Media Support
Monitor mentions to provide timely ADHD support on Bluesky.

### Multi-Modal Pipeline
Combine file monitoring, knowledge base, and social media for comprehensive support.