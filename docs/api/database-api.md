# Database API Reference

This document provides a comprehensive reference for Pattern's database operations and types.

## Core Types

### ID Types

All entities use strongly-typed IDs for compile-time safety:

```rust
use pattern_core::id::{UserId, AgentId, MemoryId, ConversationId, MessageId, TaskId};

// Generate new IDs
let user_id = UserId::generate();
let agent_id = AgentId::generate();

// Parse from strings
let user_id = UserId::parse("user-123-456")?;

// Convert to string
let id_string = user_id.to_string();

// Get nil/empty ID
let nil_id = UserId::nil();
```

### Database Connection

```rust
use pattern_core::db::{DatabaseConfig, VectorStore};
use pattern_core::db::embedded::EmbeddedDatabase;

// Create embedded database
let config = DatabaseConfig::Embedded {
    path: "./pattern.db".to_string(),
    strict_mode: false,
};

let db = EmbeddedDatabase::connect(config).await?;

// Run migrations
use pattern_core::db::migration::MigrationRunner;
MigrationRunner::run_migrations(&db).await?;
```

## User Operations

### Create User

```rust
use pattern_core::db::ops::create_user;

let user = create_user(
    &db,
    None, // Optional ID, will generate if None
    serde_json::json!({
        "theme": "dark",
        "timezone": "America/New_York"
    }),
    serde_json::json!({
        "source": "discord",
        "discord_id": "123456789"
    }),
).await?;
```

### Get User

```rust
use pattern_core::db::ops::get_user;

let user = get_user(&db, user_id).await?;
match user {
    Some(user) => println!("Found user: {}", user.id),
    None => println!("User not found"),
}
```

### Get User with Agents

```rust
use pattern_core::db::ops::get_user_with_agents;

let (user, agents) = get_user_with_agents(&db, user_id)
    .await?
    .ok_or("User not found")?;

println!("User {} has {} agents", user.id, agents.len());
```

## Agent Operations

### Create Agent

```rust
use pattern_core::db::ops::create_agent;
use pattern_core::agent::{AgentType, AgentState};

let agent = create_agent(
    &db,
    user_id,
    AgentType::Pattern, // or AgentType::Custom("my-type".to_string())
    "Pattern".to_string(),
    "You are Pattern, the ADHD support orchestrator".to_string(),
    serde_json::json!({
        "model": "gpt-4",
        "temperature": 0.7
    }),
    AgentState::Ready,
).await?;
```

### Get User's Agents

```rust
use pattern_core::db::ops::get_user_agents;

let agents = get_user_agents(&db, user_id).await?;
for agent in agents {
    println!("{}: {} ({})", agent.id, agent.name, agent.agent_type.as_str());
}
```

### Update Agent State

```rust
use pattern_core::db::ops::update_agent_state;

update_agent_state(
    &db,
    agent_id,
    AgentState::Processing,
).await?;

// Or with cooldown
use chrono::{Utc, Duration};
update_agent_state(
    &db,
    agent_id,
    AgentState::Cooldown {
        until: Utc::now() + Duration::minutes(5)
    },
).await?;
```

## Memory Operations

### Create Memory Block

```rust
use pattern_core::db::ops::create_memory;

// Without embeddings
let memory = create_memory(
    &db,
    None, // No embedding provider
    agent_id,
    "user_context".to_string(),
    "User prefers dark themes and works in EST timezone".to_string(),
    Some("User preferences".to_string()),
    serde_json::json!({
        "importance": "high",
        "source": "onboarding"
    }),
).await?;

// With embeddings
use pattern_core::embeddings::EmbeddingProvider;
let memory = create_memory(
    &db,
    Some(&embedding_provider),
    agent_id,
    "user_context".to_string(),
    "User has ADHD and benefits from gentle reminders".to_string(),
    None,
    serde_json::json!({}),
).await?;
```

### Search Memories by Similarity

```rust
use pattern_core::db::ops::search_memories;

let results = search_memories(
    &db,
    &embedding_provider,
    agent_id,
    "user preferences and settings",
    10, // limit
).await?;

for (memory, score) in results {
    println!("Score {:.3}: {}", score, memory.content);
}
```

### Get Memory by Label

```rust
use pattern_core::db::ops::get_memory_by_label;

let memory = get_memory_by_label(
    &db,
    agent_id,
    "user_context",
).await?;
```

## Vector Search

### Direct Vector Search

```rust
use pattern_core::db::{VectorStore, SearchFilter};

// Search with filter
let filter = SearchFilter {
    where_clause: Some("is_active = true AND created_at > '2025-01-01'".to_string()),
    params: vec![],
};

let results = db.vector_search(
    "memory_blocks",
    "embedding",
    &query_vector, // f32 array
    20, // limit
    Some(filter),
).await?;
```

### Create Vector Index

```rust
use pattern_core::db::DistanceMetric;

db.create_vector_index(
    "memory_blocks",
    "embedding",
    384, // dimensions
    DistanceMetric::Cosine,
).await?;
```

## Schema Types

### User

```rust
pub struct User {
    pub id: UserId,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
    pub settings: HashMap<String, serde_json::Value>,
    pub metadata: HashMap<String, serde_json::Value>,
}
```

### Agent

```rust
pub struct Agent {
    pub id: AgentId,
    pub user_id: UserId,
    pub agent_type: AgentType,
    pub name: String,
    pub system_prompt: String,
    pub config: serde_json::Value,
    pub state: AgentState,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
    pub is_active: bool,
}
```

### MemoryBlock

```rust
pub struct MemoryBlock {
    pub id: MemoryId,
    pub agent_id: AgentId,
    pub label: String,
    pub content: String,
    pub description: Option<String>,
    pub embedding: Vec<f32>,
    pub embedding_model: String,
    pub metadata: serde_json::Value,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
    pub is_active: bool,
}
```

## Error Handling

All database operations return `Result<T, DatabaseError>`:

```rust
use pattern_core::db::DatabaseError;

match create_user(&db, None, settings, metadata).await {
    Ok(user) => println!("Created user {}", user.id),
    Err(DatabaseError::ConnectionFailed { .. }) => {
        eprintln!("Database connection failed");
    }
    Err(DatabaseError::QueryFailed(e)) => {
        eprintln!("Query failed: {}", e);
    }
    Err(e) => eprintln!("Database error: {}", e),
}
```

## Best Practices

1. **Always use typed IDs**: Never use raw strings for entity IDs
2. **Handle None cases**: Many get operations return `Option<T>`
3. **Use transactions for multi-step operations** (when available)
4. **Keep embeddings consistent**: Use the same model throughout
5. **Index frequently searched fields**: Use appropriate indexes for performance

## SurrealDB Query Patterns

### Full-Text Search

Pattern uses SurrealDB's `@@` operator for full-text search with BM25 ranking:

```sql
-- Search archival memories
SELECT * FROM mem
WHERE (<-agent_memories<-agent:⟨agent_uuid⟩..)
AND memory_type = 'archival'
AND value @@ $search_term
LIMIT $limit;

-- Search messages by content
SELECT * FROM message
WHERE (<-agent_messages<-agent:⟨agent_uuid⟩..)
AND content @@ $search_query
AND created_at >= $start_time
ORDER BY position DESC
LIMIT $limit;
```

### Graph Traversal

Use SurrealDB's graph notation for relationship queries:

```sql
-- Get all memories for an agent (through edge entity)
SELECT * FROM mem
WHERE (<-agent_memories<-agent:⟨agent_uuid⟩..);

-- Get agent's conversations
SELECT * FROM conversation
WHERE (<-agent_conversations<-agent:⟨agent_uuid⟩..);
```

### Pre-Computed Table Views

SurrealDB's pre-computed views provide event-based, incrementally updating materialized views:

```sql
-- Agent activity statistics (auto-updates on message insert)
DEFINE TABLE agent_activity TYPE NORMAL AS
SELECT
    count() AS message_count,
    time::max(created_at) AS last_active,
    ->agent.id AS agent_id,
    ->agent.name AS agent_name,
    array::distinct(->conversation.id) AS active_conversations
FROM message
GROUP BY agent_id, agent_name;

-- Memory usage analytics (auto-updates on memory access)
DEFINE TABLE memory_usage TYPE NORMAL AS
SELECT
    count() AS access_count,
    time::max(accessed_at) AS last_accessed,
    ->agent.id AS agent_id,
    ->agent.name AS agent_name,
    label,
    memory_type
FROM mem
WHERE accessed_at IS NOT NULL
GROUP BY agent_id, agent_name, label, memory_type;

-- Tool usage patterns (auto-updates on tool call)
DEFINE TABLE tool_usage TYPE NORMAL AS
SELECT
    count() AS call_count,
    ->agent.id AS agent_id,
    ->agent.name AS agent_name,
    tool_name,
    array::group(RETURN {
        status: status,
        count: count()
    }) AS status_breakdown
FROM tool_call
GROUP BY agent_id, agent_name, tool_name;

-- Conversation summary view
DEFINE TABLE conversation_summary TYPE NORMAL AS
SELECT
    count() AS message_count,
    time::min(created_at) AS started_at,
    time::max(created_at) AS last_message_at,
    ->conversation.id AS conversation_id,
    ->agent.id AS agent_id,
    ->agent.name AS agent_name,
    array::group(RETURN {
        role: role,
        count: count()
    }) AS message_breakdown
FROM message
GROUP BY conversation_id, agent_id, agent_name;
```

### Query Examples

```rust
// Use pre-computed views for fast statistics
let agent_stats: Vec<Value> = db
    .query("SELECT * FROM agent_activity WHERE agent_id = $agent_id")
    .bind(("agent_id", agent_id))
    .await?
    .take(0)?;

// Search with filters
let messages: Vec<Message> = db
    .query(r#"
        SELECT * FROM message
        WHERE agent_id = $agent_id
        AND content @@ $query
        AND role = $role
        AND created_at BETWEEN $start AND $end
        ORDER BY position DESC
        LIMIT $limit
    "#)
    .bind(("agent_id", agent_id))
    .bind(("query", search_query))
    .bind(("role", "assistant"))
    .bind(("start", start_time))
    .bind(("end", end_time))
    .bind(("limit", 50))
    .await?
    .take(0)?;
```

### Important Notes

1. **Pre-computed views update automatically** - No triggers needed
2. **Views only trigger on FROM table changes** - Deleting an agent won't update agent_activity
3. **Initial view creation can be slow** - Index appropriately
4. **Use parameter binding** - Never concatenate SQL strings
5. **Graph notation `<-relation<-` is powerful** - Traverses relationships efficiently
```
