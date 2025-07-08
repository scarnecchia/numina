# SurrealDB Query Patterns

This document covers common patterns and gotchas when working with SurrealDB in the Pattern project.

## Response Handling with `.take()`

The `surrealdb::Response::take()` method is used to extract results from queries, but it has some important behaviors to understand:

### Basic Usage

```rust
let mut response = db.query("SELECT * FROM user").await?;
let users: Vec<User> = response.take(0)?; // Take from first query
```

### Multiple Queries

When you send multiple queries, each has its own index:

```rust
let mut response = db
    .query("SELECT * FROM user WHERE name = 'John'")  // Index 0
    .query("SELECT * FROM task WHERE status = 'pending'") // Index 1
    .await?;

let johns: Vec<User> = response.take(0)?;
let tasks: Vec<Task> = response.take(1)?;
```

### Nested Results (Common Gotcha!)

When queries return nested structures, you might get a `Vec<Vec<T>>` instead of `Vec<T>`:

```rust
// This DELETE query returns [[{ affected: 1 }]]
let delete_query = "DELETE user:123";
let mut response = db.query(delete_query).await?;

// WRONG: This will fail with deserialization error
let result: Vec<serde_json::Value> = response.take(0)?;

// CORRECT: Handle the nested structure
let result: Vec<Vec<serde_json::Value>> = response.take(0)?;
let affected = result[0][0]["affected"].as_u64().unwrap_or(0);
```

### Field Extraction

You can extract specific fields using tuple syntax:

```rust
let mut response = db.query("SELECT id, name, email FROM user").await?;

// Extract just the IDs
let ids: Vec<String> = response.take("id")?;

// Or from a specific query index
let names: Vec<String> = response.take((0, "name"))?;
```

## DELETE Operations

### Using the Database Delete Method

The simplest way to delete records is using the database's delete method:

```rust
// Delete a specific record using RecordId::from() for proper formatting
db.delete(RecordId::from(user_id)).await?;

// Or with table prefix
db.delete(("user", user_id.uuid().to_string())).await?;
```

### Delete with Raw Queries

For conditional deletes or complex operations:

```rust
// Delete records matching a condition
db.query("DELETE task WHERE status = 'completed' AND created_at < time::now() - 30d")
    .await?;

// Delete specific record with query
let query = format!("DELETE {}", RecordId::from(user_id));
db.query(&query).await?;
```

### Key Pattern: Always Use RecordId::from()

When deleting specific records, always use `RecordId::from()` on the ID field:

```rust
// CORRECT: RecordId::from() handles formatting
db.delete(RecordId::from(agent.id)).await?;

// AVOID: Manual formatting is error-prone
let query = format!("DELETE agent:{}", agent.id.uuid().to_string());
```

## Live Query Patterns

### Watching Specific Records

```rust
// Watch a single record
let stream = db
    .select((UserIdType::PREFIX, user_id.uuid().to_string()))
    .live()
    .await?;
```

### Watching with Conditions

For efficient live queries on collections with conditions, consider adding array fields to enable direct filtering:

```rust
// Instead of watching a junction table:
// LIVE SELECT out.* FROM agent_memories WHERE in = $agent_id

// Add an agents array to memory blocks:
// DEFINE FIELD agents ON mem TYPE array<record<agent>> DEFAULT [];

// Then watch directly (note: no parameters in WHERE clause!)
let query = format!("LIVE SELECT * FROM mem WHERE {} IN agents", RecordId::from(agent_id));
```

### Parameter Limitations in LIVE SELECT

**Important**: You cannot use parameters in the WHERE clause of LIVE SELECT queries:

```rust
// WRONG: Parameters not allowed in WHERE clause
let query = "LIVE SELECT * FROM person WHERE age > $min_age";
db.query(query).bind(("min_age", 18)).await?; // This won't work!

// CORRECT: Build the query string with format!
let min_age = 18;
let query = format!("LIVE SELECT * FROM person WHERE age > {}", min_age);
db.query(&query).await?;

// OK: Parameters allowed for table reference
let query = "LIVE SELECT * FROM $table WHERE age > 18";
db.query(query).bind(("table", "person")).await?;
```

When building queries with `format!()`, always sanitize user input to prevent injection attacks.

### Processing Live Query Notifications

```rust
use futures::StreamExt;
use surrealdb::{Notification, Action};

while let Some(notification) = stream.next().await {
    match notification {
        Ok(Notification { action, data, .. }) => {
            match action {
                Action::Create => {
                    // New record created
                },
                Action::Update => {
                    // Record updated
                },
                Action::Delete => {
                    // Record deleted
                },
                _ => {
                    // Action is non-exhaustive, handle unknown variants
                }
            }
        }
        Err(e) => {
            tracing::error!("Live query error: {}", e);
        }
    }
}
```

## Transaction Patterns

### Ensuring Atomic Operations

When you need multiple operations to succeed or fail together:

```rust
let query = r#"
    BEGIN TRANSACTION;
    
    -- Create the memory block
    CREATE mem:$mem_id SET
        owner_id = $owner_id,
        label = $label,
        content = $content;
    
    -- Attach to agent with access level
    RELATE $agent_id->agent_memories->$mem_id
        SET access_level = 'write';
    
    -- Update the agents array
    UPDATE $mem_id SET agents += $agent_id;
    
    COMMIT TRANSACTION;
"#;

db.query(query)
    .bind(("mem_id", RecordId::from(memory_id)))
    .bind(("owner_id", RecordId::from(owner_id)))
    .bind(("agent_id", RecordId::from(agent_id)))
    .bind(("label", label))
    .bind(("content", content))
    .await?;
```

## Common Gotchas

### 1. UUID Format in Record IDs

Always use `RecordId::from()` to handle ID formatting:

```rust
// BEST: Use the delete method with RecordId
db.delete(RecordId::from(memory_id)).await?;

// GOOD: RecordId handles formatting in queries
let query = format!("DELETE {}", RecordId::from(memory_id));

// AVOID: Manual UUID formatting is error-prone
let query = format!("DELETE mem:{}", memory_id.uuid().to_string().replace('-', "_"));
```

### 2. RELATE Queries Don't Update Existing Records

The `RELATE` query creates junction records but doesn't update the related records themselves. If you need to maintain denormalized data (like an agents array), update it separately:

```rust
// Create the relationship
db.query("RELATE $agent->agent_memories->$memory SET access_level = 'read'")
    .bind(("agent", RecordId::from(agent_id)))
    .bind(("memory", RecordId::from(memory_id)))
    .await?;

// Also update the denormalized array
db.query("UPDATE $memory SET agents += $agent WHERE $agent NOT IN agents")
    .bind(("memory", RecordId::from(memory_id)))
    .bind(("agent", RecordId::from(agent_id)))
    .await?;
```

### 3. Empty Results vs Errors

SurrealDB returns empty results for queries that match nothing, not errors:

```rust
let mut response = db.query("SELECT * FROM user:nonexistent").await?;
let user: Option<User> = response.take(0)?; // This is None, not an error
```

### 4. Live Queries and Junction Tables

Live queries on junction tables don't fire when the related records change, only when the junction itself changes. For real-time updates on related data, watch the actual data tables with appropriate filters.

## Best Practices

1. **Use RecordId::from()**: Always convert typed IDs with `RecordId::from()` for database operations
2. **Prefer Database Methods**: Use `db.delete()`, `db.select()`, etc. over raw queries when possible
3. **LIVE SELECT Parameters**: Remember parameters only work for table names, not WHERE clauses
4. **Sanitize Format Strings**: When using `format!()` for LIVE queries, sanitize user input
5. **Handle Nested Responses**: Be aware that some queries return nested structures
6. **Use Transactions**: For multi-step operations that must be atomic
7. **Index Array Fields**: When using arrays for filtering (like `agents` on memory blocks)
8. **Test Live Queries**: Always test that your live queries fire on the expected changes