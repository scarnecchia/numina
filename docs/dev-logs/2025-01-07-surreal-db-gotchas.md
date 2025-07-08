# SurrealDB Integration Gotchas - 2025-01-07

This document captures the various quirks and gotchas we encountered while integrating SurrealDB into Pattern. Future developers, you're welcome.

## Record ID Handling

### The UUID Format Issue

SurrealDB expects record IDs in a specific format: `table:identifier`. When creating records with `.create()`, you need to pass just the identifier part, not a full UUID object.

**Wrong:**
```rust
DB.create((UserIdType::PREFIX, id.uuid()))  // Passes a UUID type
```

**Right:**
```rust
DB.create((UserIdType::PREFIX, id.uuid().to_string()))  // Passes a string
```

### The `⟨⟩` Character Problem

When SurrealDB stores certain identifiers (especially UUIDs or complex strings), it wraps them in special Unicode characters `⟨` and `⟩` (NOT regular angle brackets `<>`). These need to be stripped when parsing IDs back from the database.

```rust
// Helper function to strip the brackets
pub fn strip_brackets(s: &str) -> &str {
    s.strip_prefix('⟨')
        .and_then(|s| s.strip_suffix('⟩'))
        .unwrap_or(s)
}

// Usage when parsing record IDs
let uuid_str = strip_brackets(&db_agent.id.key().to_string());
let agent_id = AgentId::parse(&format!("agent-{}", uuid_str))?;
```

### RecordId vs Custom ID Types

We use custom ID types (`UserId`, `AgentId`, etc.) in our domain model, but SurrealDB uses `RecordId`. This requires conversion at the database boundary:

1. **Domain to DB**: Convert custom IDs to RecordId using `.into()`
2. **DB to Domain**: Parse RecordId back to custom types using the tuple pattern

## Datetime Serialization

### The Type Mismatch Issue

SurrealDB has strict type checking for datetime fields. When you define a field as `TYPE datetime` in the schema, you CANNOT pass a string representation - it must be a `surrealdb::Datetime` type.

**Schema Definition:**
```sql
DEFINE FIELD created_at ON user TYPE datetime;
```

**Wrong - Using JSON serialization:**
```rust
// This serializes datetime to a string, causing a FieldCheck error
let data = serde_json::json!({
    "created_at": chrono::Utc::now()  // Becomes "2025-01-07T19:20:40.270159777Z"
});
```

**Right - Using proper types:**
```rust
// Option 1: Use surrealdb::Datetime in your structs
#[derive(Serialize)]
struct DbUser {
    created_at: surrealdb::Datetime,
}

// Option 2: Convert when creating
let db_user = DbUser {
    created_at: chrono::Utc::now().into(),  // Converts to surrealdb::Datetime
};
```

### Updates with Datetimes

When updating records with datetime fields, you can't use `serde_json::json!` because it will serialize to strings:

```rust
// Wrong
.merge(serde_json::json!({
    "updated_at": chrono::Utc::now()  // Serializes to string!
}))

// Right - use a struct
#[derive(Serialize)]
struct AgentUpdate {
    state: AgentState,
    updated_at: surrealdb::Datetime,
}

let update = AgentUpdate {
    state,
    updated_at: chrono::Utc::now().into(),
};
DB.update(id).merge(update).await
```

## Database Wrapper Types

To handle the impedance mismatch between our domain types and SurrealDB's expectations, we created wrapper types:

```rust
// Domain type (what the rest of the app uses)
pub struct User {
    pub id: UserId,  // Our custom ID type
    pub created_at: chrono::DateTime<chrono::Utc>,
}

// Database type (what SurrealDB expects)
pub struct DbUser {
    pub id: surrealdb::RecordId
    pub created_at: surrealdb::Datetime,
}

// Conversions
impl From<User> for DbUser { ... }
impl (Try)From<DbUser> for User { ... }  // Tuple pattern for ID
```

Key points:
- DB types have specific requirements for any field named "id"
- DB types use `surrealdb::Datetime` instead of `chrono::DateTime`

## Embedded Database Path Format

SurrealDB's embedded mode expects specific path formats:

```rust
// For in-memory database
"memory"

// For file-based database
format!("surrealkv://{}", path)  // Must have surrealkv:// prefix!
```

## Schema Definitions

### DO NOT Define ID Fields as anything other than 'record' type

```sql
-- Wrong
DEFINE FIELD id ON user TYPE uuid;

-- Right (let SurrealDB manage IDs)
DEFINE FIELD created_at ON user TYPE datetime;

-- Right (use the right type if you must be explicit)
DEFINE FIELD id ON user TYPE record;
```

### Use Proper Field Types

```sql
DEFINE FIELD created_at ON table TYPE datetime;  -- Not string!
    -- And also not chrono::DateTime either! fmt string for parsing is "d'%FT%T%.6fZ'"
DEFINE FIELD metadata ON table TYPE object;      -- For JSON data
DEFINE FIELD user_id ON table TYPE record<user>; -- For relations
```


## Testing Gotchas

### Global Static DB Instance

Using a global static DB instance (as recommended in SurrealDB docs) causes issues with concurrent tests:

1. **AlreadyConnected errors**: Multiple tests trying to connect
2. **"sending into a closed channel"**: Connection closing between tests
3. **Data bleeding between tests**: Shared namespace issues

**Solutions:**

1. ~~**Single test approach**: Combine related tests into one test function~~ (old workaround)
2. ~~**Serial execution**: Use `serial_test` crate (didn't work well for us)~~ (old workaround)
3. ~~**Test isolation**: Use unique namespaces per test (causes data visibility issues)~~ (old workaround)
4. **Extension trait pattern**: The proper solution! See below.

### The Extension Trait Solution (Recommended)

Create an extension trait that works with both global and local DB instances:

```rust
use surrealdb::engine::any;

// Extension trait for database operations
pub trait SurrealExt<C>
where
    C: surrealdb::Connection,
{
    async fn create_user(&self, /* params */) -> Result<User>;
    // ... other operations
}

// Implementation that works with any Surreal instance
// TODO: Consider simplifying to impl directly on Surreal<C> instead of AsRef
// Since Surreal is cheaply cloneable and handles connection pooling internally
impl<T, C> SurrealExt<C> for T
where
    T: AsRef<surrealdb::Surreal<C>>,
    C: surrealdb::Connection,
{
    async fn create_user(&self, /* params */) -> Result<User> {
        let db = self.as_ref();
        // ... implementation using db
    }
}

// In production code - use global static
use crate::db::DB;
let user = DB.create_user(/* params */).await?;

// In tests - create isolated instance
#[tokio::test]
async fn test_user_operations() {
    let db = any::connect("memory").await.unwrap();
    db.use_ns("test").use_db("test").await.unwrap();
    
    let user = db.create_user(/* params */).await.unwrap();
    // ... test assertions
}
```

**Key insight**: Use `any::connect("memory")` for reliable test database connections without type issues.

## Query Result Handling

### The `take()` Method

SurrealDB query results need to be extracted with `.take(index)`:

```rust
let db_agents: Vec<DbAgent> = DB
    .query(&query)
    .await?
    .take(0)?;  // Takes the first query result
```

### Vector Search Scores

When doing vector similarity searches, extracting scores alongside results is tricky. For now, we return dummy scores:

```rust
// TODO: Figure out how to extract similarity scores from SurrealDB
Ok(results.into_iter().map(|db| (db.into(), 1.0)).collect())
```

## Best Practices

1. **Always use `.to_string()` on UUIDs** when passing to SurrealDB
2. **Use wrapper types** to separate domain and DB concerns
3. **Never use `serde_json::json!`** for updates with datetime fields
4. **Strip `⟨⟩` characters** when parsing IDs from the database
5. **Don't define ID fields** in schema - let SurrealDB manage them
6. **Use proper structs** for database operations, not JSON objects
7. **Test database operations together** to avoid connection issues

## Example: Complete CRUD Pattern

```rust
// Create
let db_user = DbUser {
    created_at: chrono::Utc::now().into(),
    // ... other fields
};
let created: Option<DbUser> = DB
    .create((UserIdType::PREFIX, id.uuid().to_string()))
    .content(db_user)
    .await?;

// Read
let db_user: Option<DbUser> = DB
    .select((UserIdType::PREFIX, id.uuid().to_string()))
    .await?;

// Update (with datetime)
#[derive(Serialize)]
struct UserUpdate {
    updated_at: surrealdb::Datetime,
    settings: serde_json::Value,
}
let update = UserUpdate {
    updated_at: chrono::Utc::now().into(),
    settings: new_settings,
};
DB.update((UserIdType::PREFIX, id.uuid().to_string()))
    .merge(update)
    .await?;

// Delete
DB.delete((UserIdType::PREFIX, id.uuid().to_string()))
    .await?;
```

## Test Isolation Best Practices

1. **Use extension traits** to make database operations work with both global and local instances
2. **Create isolated test databases** with `any::connect("memory")`
3. **Each test gets its own DB instance** - no more connection conflicts!
4. **Tests can run in parallel** - much faster test suites

```rust
// Test helper function
async fn test_db() -> Surreal<Any> {
    let db = any::connect("memory").await.unwrap();
    db.use_ns("test").use_db("test").await.unwrap();
    // Run migrations if needed
    MigrationRunner::run(&db).await.unwrap();
    db
}

#[tokio::test]
async fn test_concurrent_operations() {
    let db = test_db().await;
    // This test has its own isolated database!
}
```

Remember: SurrealDB is powerful but has its own way of doing things. When in doubt, use proper types and avoid JSON serialization for complex types.
