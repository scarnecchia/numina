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

## Common Patterns

### Initialize User with Agents

```rust
async fn initialize_user(
    db: &impl VectorStore,
    discord_id: &str,
) -> Result<(User, Vec<Agent>)> {
    // Create user
    let user = create_user(
        db,
        None,
        serde_json::json!({}),
        serde_json::json!({ "discord_id": discord_id }),
    ).await?;
    
    // Create agents
    let mut agents = Vec::new();
    
    for (agent_type, name, prompt) in [
        (AgentType::Pattern, "Pattern", "You are Pattern..."),
        (AgentType::Entropy, "Entropy", "You are Entropy..."),
        // ... other agents
    ] {
        let agent = create_agent(
            db,
            user.id,
            agent_type,
            name.to_string(),
            prompt.to_string(),
            serde_json::json!({}),
            AgentState::Ready,
        ).await?;
        agents.push(agent);
    }
    
    Ok((user, agents))
}
```

### Search Agent Context

```rust
async fn search_context(
    db: &impl VectorStore,
    embedder: &impl EmbeddingProvider,
    agent_id: AgentId,
    query: &str,
) -> Result<String> {
    let memories = search_memories(db, embedder, agent_id, query, 5).await?;
    
    let context = memories
        .into_iter()
        .map(|(memory, score)| {
            format!("[{:.2}] {}: {}", score, memory.label, memory.content)
        })
        .collect::<Vec<_>>()
        .join("\n");
    
    Ok(context)
}
```
