# Database Migration Guide

This guide covers the recent migration from SQLite to SurrealDB and the implementation of type-safe database operations.

## Overview

As of 2025-07-06, Pattern has migrated from SQLite with a custom KV cache to SurrealDB embedded with built-in vector search capabilities. This migration brings several benefits:

- **Type Safety**: Custom ID types (UserId, AgentId, etc.) throughout the codebase
- **Vector Search**: Native support for similarity search without additional dependencies
- **Pure Rust**: No C dependencies (using SurrealKV instead of RocksDB)
- **Schema Flexibility**: Better support for nested and complex data structures

## Key Changes

### 1. ID Types

All database IDs now use strongly-typed wrappers:

```rust
// Before
let user_id: String = "user-123".to_string();

// After
let user_id: UserId = UserId::generate();
```

### 2. Database Operations

Database operations have been simplified and made type-safe:

```rust
// Create a user
let user = create_user(
    &db,
    None, // Optional ID, will generate if None
    serde_json::json!({"theme": "dark"}),
    serde_json::json!({"source": "discord"}),
).await?;

// Get a user by ID (type-safe)
let user = get_user(&db, user_id).await?;

// Create an agent
let agent = create_agent(
    &db,
    user.id,
    AgentType::Pattern,
    "Pattern".to_string(),
    "You are Pattern, the ADHD support orchestrator".to_string(),
    serde_json::json!({}),
    AgentState::Ready,
).await?;
```

### 3. SurrealDB Value Format

SurrealDB wraps values in a nested format. We handle this automatically:

```rust
// SurrealDB returns:
{
  "Object": {
    "created_at": {
      "Datetime": "2025-07-06T12:00:00Z"
    },
    "id": {
      "Thing": {
        "tb": "users",
        "id": {
          "String": "user-123"
        }
      }
    }
  }
}

// Our unwrap_surreal_value helper converts to:
{
  "created_at": "2025-07-06T12:00:00Z",
  "id": "user-123"
}
```

### 4. Vector Search

Embeddings are stored directly in the database with vector indexes:

```rust
// Create memory with embedding
let memory = create_memory(
    &db,
    Some(&embedder), // Optional embedding provider
    agent_id,
    "context".to_string(),
    "Important context about the user".to_string(),
    None,
    serde_json::json!({}),
).await?;

// Search memories by similarity
let results = search_memories(
    &db,
    &embedder,
    agent_id,
    "user preferences",
    10, // limit
).await?;
```

### 5. Schema Definition

Database schemas are defined in code:

```rust
// From schema.rs
pub fn agents() -> TableDefinition {
    TableDefinition {
        name: "agents".to_string(),
        schema: r#"
            DEFINE TABLE agents SCHEMAFULL;
            DEFINE FIELD id ON agents TYPE string;
            DEFINE FIELD user_id ON agents TYPE record;
            DEFINE FIELD agent_type ON agents TYPE string;
            DEFINE FIELD name ON agents TYPE string;
            DEFINE FIELD system_prompt ON agents TYPE string;
            DEFINE FIELD config ON agents TYPE object;
            DEFINE FIELD state ON agents TYPE any;
            DEFINE FIELD created_at ON agents TYPE datetime;
            DEFINE FIELD updated_at ON agents TYPE datetime;
            DEFINE FIELD is_active ON agents TYPE bool DEFAULT true;
        "#.to_string(),
        indexes: vec![
            "DEFINE INDEX agents_user ON agents FIELDS user_id".to_string(),
            "DEFINE INDEX agents_type ON agents FIELDS agent_type".to_string(),
        ],
    }
}
```

## Migration Process

### 1. Running Migrations

Migrations are automatically run when the database is initialized:

```rust
// In your initialization code
let db = EmbeddedDatabase::connect(config).await?;
MigrationRunner::run_migrations(&db).await?;
```

### 2. Data Migration from SQLite

If you have existing SQLite data, you'll need to migrate it manually. Here's a basic approach:

```rust
// 1. Open old SQLite database
let old_db = sqlx::SqlitePool::connect("sqlite:pattern.db").await?;

// 2. Read old data
let old_users = sqlx::query!("SELECT * FROM users")
    .fetch_all(&old_db)
    .await?;

// 3. Convert and insert into SurrealDB
for old_user in old_users {
    create_user(
        &new_db,
        Some(UserId::parse(&old_user.id)?),
        old_user.settings,
        old_user.metadata,
    ).await?;
}
```

### 3. Embedding Migration

If you had embeddings stored separately, they now go directly in the database:

```rust
// Re-generate embeddings for existing content
for memory in old_memories {
    create_memory(
        &db,
        Some(&embedder), // This will generate the embedding
        agent_id,
        memory.label,
        memory.content,
        memory.description,
        memory.metadata,
    ).await?;
}
```

## Troubleshooting

### Common Issues

1. **"missing field `id`" errors**: Make sure you're using the `unwrap_surreal_value` helper or handling the wrapped format manually.

2. **Vector dimension mismatch**: Ensure all embeddings use the same model/dimensions. The default is 384 dimensions.

3. **ID format errors**: Use the typed ID helpers (UserId::generate(), etc.) instead of raw strings.

### Debugging Tips

1. **Print raw responses**: Add debug prints to see the actual SurrealDB response format:
   ```rust
   eprintln!("Raw response: {:?}", response.data);
   ```

2. **Check schema**: Verify your schema matches the data you're trying to insert:
   ```rust
   db.execute("INFO FOR TABLE agents", vec![]).await?;
   ```

3. **Test queries**: Use the SurrealDB REPL to test queries:
   ```bash
   surreal sql --conn memory
   > USE NS pattern DB main;
   > SELECT * FROM users;
   ```

## Best Practices

1. **Always use typed IDs**: This prevents mixing up different entity IDs
2. **Handle errors properly**: Don't unwrap in production code
3. **Use transactions for multi-step operations**: Even though our current implementation doesn't fully support them
4. **Keep embeddings consistent**: Always use the same embedding model for a given database

## Future Improvements

- Full transaction support in embedded mode
- Batch operations for better performance
- Remote SurrealDB support for scaling
- Automatic embedding regeneration when changing models