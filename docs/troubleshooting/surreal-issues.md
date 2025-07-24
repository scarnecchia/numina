# SurrealDB Integration Issues

This document provides quick solutions to common SurrealDB integration problems in Pattern.

## Quick Reference

For detailed explanations and examples, see [SurrealDB Integration Gotchas](../dev-logs/2025-01-07-surreal-db-gotchas.md).

## Common Errors and Solutions

### FieldCheck Error: "expected a datetime"

**Error:**
```
FieldCheck { thing: "user:⟨uuid⟩", value: "'2025-01-07T19:20:40Z'", field: Idiom([Field(Ident("created_at"))]), check: "datetime" }
```

**Solution:** Use `surrealdb::Datetime` instead of serializing to JSON:
```rust
// Wrong
serde_json::json!({ "created_at": chrono::Utc::now() })

// Right
struct DbModel {
    created_at: surrealdb::Datetime,
}
```

### IdMismatch Error

**Error:**
```
IdMismatch { value: "agent:⟨uuid⟩" }
```

**Solution:** Pass UUID as string, not UUID type:
```rust
// Wrong
DB.create((AgentIdType::PREFIX, id.uuid()))

// Right
DB.create((AgentIdType::PREFIX, id.uuid().to_string()))
```

### Invalid UUID Format

**Error:**
```
Error(Char { character: '⟨', index: 1 })
```

**Solution:** Strip the `⟨⟩` characters when parsing:
```rust
pub fn strip_brackets(s: &str) -> &str {
    s.strip_prefix('⟨')
        .and_then(|s| s.strip_suffix('⟩'))
        .unwrap_or(s)
}
```

### AlreadyConnected Error in Tests

**Error:**
```
ConnectionFailed(Api(AlreadyConnected))
```

**Solution:** Use the extension trait pattern with isolated test databases:
```rust
// In tests - create isolated instance
let db = any::connect("memory").await.unwrap();
db.use_ns("test").use_db("test").await.unwrap();
```

### InvalidUrl for Embedded Database

**Error:**
```
ConnectionFailed(Api(InvalidUrl("/path/to/db")))
```

**Solution:** Use proper path format:
```rust
// For file-based
format!("surrealkv://{}", path)

// For in-memory
"memory"
```

## Database Schema Best Practices

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

## Testing Strategies

### Extension Trait Pattern (Recommended)

Create an extension trait that works with both global and local instances:
```rust
pub trait SurrealExt<C>
where
    C: surrealdb::Connection,
{
    async fn create_user(&self, /* params */) -> Result<User>;
}

impl<T, C> SurrealExt<C> for T
where
    T: AsRef<surrealdb::Surreal<C>>,
    C: surrealdb::Connection,
{
    // Implementation
}

// In tests
#[tokio::test]
async fn test_operations() {
    let db = any::connect("memory").await.unwrap();
    db.use_ns("test").use_db("test").await.unwrap();
    // Each test gets its own isolated database!
}
```

## See Also

- [Complete SurrealDB Gotchas Guide](../dev-logs/2025-01-07-surreal-db-gotchas.md)
- [Database API Reference](../api/DATABASE_API.md)
- [Database Migration Guide](../guides/DATABASE_MIGRATION.md)
