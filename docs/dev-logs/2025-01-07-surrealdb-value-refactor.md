# SurrealDB Value Handling Refactor

**Date**: 2025-01-07  
**Status**: Planning

## Problem

We built a custom recursive `unwrap_surreal_value()` function to handle SurrealDB's wrapped value format, when SurrealDB already provides `from_value()` and `to_value()` helpers that do this automatically.

## Current Implementation

### What We're Doing Now
1. Get `surrealdb::Value` from query response
2. Convert to `serde_json::Value` using `serde_json::to_value()`
3. Use our custom `unwrap_surreal_value()` to recursively unwrap
4. Use `serde_json::from_value()` to deserialize to our types

### The Issue
- We're doing manual work that SurrealDB's helpers already handle
- Extra conversions: `surrealdb::Value` → `serde_json::Value` → unwrap → deserialize
- Our custom unwrapper is ~50 lines of recursive code that duplicates SurrealDB's functionality

## Proposed Solution

### Option 1: Use SurrealDB Helpers in Database Backend
Modify `EmbeddedDatabase::execute()` to return properly deserialized data:

```rust
// In execute() method
let surreal_value: surrealdb::Value = response.take(0)?;

// Instead of converting to JSON and wrapping, just return the Value
// Let the ops layer handle deserialization
```

**Pros**: 
- Clean separation of concerns
- Database layer just returns raw values

**Cons**: 
- Would need to change the trait to return `surrealdb::Value` instead of `serde_json::Value`
- Breaks our database abstraction (exposes SurrealDB types)

### Option 2: Use SurrealDB Helpers in Ops Layer
Keep the current trait but handle conversion better:

```rust
// In ops functions
let response = db.execute(query, params).await?;

// Convert the JSON back to surrealdb::Value
let surreal_value = surrealdb::value::to_value(&response.data)?;

// Use from_value to deserialize directly
let user: User = surrealdb::value::from_value(surreal_value)?;
```

**Pros**: 
- Maintains database abstraction
- No trait changes needed

**Cons**: 
- Double conversion: Value → JSON → Value
- Inefficient

### Option 3: Hybrid Approach (Recommended)
Add a method to the trait that returns typed data directly:

```rust
trait DatabaseBackend {
    // Existing method
    async fn execute(&self, query: &str, params: Vec<(String, serde_json::Value)>) -> Result<QueryResponse>;
    
    // New method for typed queries
    async fn query_one<T: DeserializeOwned>(&self, query: &str, params: Vec<(String, serde_json::Value)>) -> Result<Option<T>>;
    
    async fn query_many<T: DeserializeOwned>(&self, query: &str, params: Vec<(String, serde_json::Value)>) -> Result<Vec<T>>;
}

// Implementation in EmbeddedDatabase
async fn query_one<T: DeserializeOwned>(&self, query: &str, params: Vec<(String, serde_json::Value)>) -> Result<Option<T>> {
    let mut surrealdb_query = self.client.query(query);
    for (name, value) in params {
        surrealdb_query = surrealdb_query.bind((name, value));
    }
    
    let mut response = surrealdb_query.await?;
    let surreal_value: surrealdb::Value = response.take(0)?;
    
    // Use from_value directly!
    match surrealdb::value::from_value(surreal_value)? {
        Some(val) => Ok(Some(val)),
        None => Ok(None),
    }
}
```

**Pros**: 
- Clean API for typed queries
- Uses SurrealDB's helpers properly
- Maintains abstraction (SurrealDB types don't leak)
- More efficient (no double conversion)

**Cons**: 
- Need to add new methods to trait
- Some duplication between execute() and query_*() methods

## Implementation Plan

1. **Add new methods to DatabaseBackend trait**
   - `query_one<T>()` - returns `Option<T>`
   - `query_many<T>()` - returns `Vec<T>`
   - Keep `execute()` for raw queries that need custom handling

2. **Implement in EmbeddedDatabase**
   - Use `surrealdb::value::from_value()` directly
   - Handle CREATE operations that return single values
   - Handle SELECT operations that return arrays

3. **Update ops functions**
   - Replace `execute()` + `unwrap_surreal_value()` with `query_one()`/`query_many()`
   - Remove manual JSON manipulation

4. **Remove custom unwrapper**
   - Delete `unwrap_surreal_value()` function
   - Remove related tests
   - Update documentation

## Benefits

- **Simpler code**: Remove 50+ lines of custom unwrapping logic
- **Better performance**: No double conversion through JSON
- **Type safety**: Compile-time guarantees for query results
- **Cleaner API**: `query_one()` clearly indicates expecting single result

## Migration Example

### Before:
```rust
pub async fn get_user<DB: VectorStore>(db: &DB, id: UserId) -> Result<Option<User>> {
    let response = db.execute(&format!("SELECT * FROM users:`{}` LIMIT 1", id.to_string()), vec![]).await?;
    
    if let Some(data) = response.data.as_array().and_then(|arr| arr.first()) {
        let unwrapped = unwrap_surreal_value(data);
        Ok(Some(serde_json::from_value(unwrapped)?))
    } else {
        Ok(None)
    }
}
```

### After:
```rust
pub async fn get_user<DB: VectorStore>(db: &DB, id: UserId) -> Result<Option<User>> {
    db.query_one(&format!("SELECT * FROM users:`{}` LIMIT 1", id.to_string()), vec![]).await
}
```

Much cleaner! 🎯