# SurrealDB Types Direct Usage Refactor

**Date**: 2025-01-07  
**Status**: In Progress

## Decision

We're committed to using SurrealDB, so let's stop over-abstracting and use their types directly. This will:
1. Simplify our code significantly
2. Avoid double-wrapping issues
3. Make better use of SurrealDB's built-in helpers
4. Reduce conversion overhead

## Current Issues

1. **Double-wrapping**: Our `execute()` method converts `surrealdb::Value` to JSON, causing nested Array wrappers
2. **Type conversions**: We're doing `Value → JSON → unwrap → deserialize` when we could just use `from_value`
3. **Over-abstraction**: Trying to hide SurrealDB types when all backends use them anyway

## Refactor Plan

### 1. Change DatabaseBackend trait to use SurrealDB types

```rust
use surrealdb::Value;

#[async_trait]
pub trait DatabaseBackend: Send + Sync {
    /// Execute a query and return SurrealDB Value directly
    async fn query(&self, query: &str, params: impl IntoIterator<Item = (String, Value)>) -> Result<Value>;
    
    /// Execute a query expecting a single result
    async fn query_one<T: DeserializeOwned>(&self, query: &str, params: impl IntoIterator<Item = (String, Value)>) -> Result<Option<T>>;
    
    /// Execute a query expecting multiple results
    async fn query_many<T: DeserializeOwned>(&self, query: &str, params: impl IntoIterator<Item = (String, Value)>) -> Result<Vec<T>>;
}
```

### 2. Simplify EmbeddedDatabase implementation

```rust
impl DatabaseBackend for EmbeddedDatabase {
    async fn query(&self, query: &str, params: impl IntoIterator<Item = (String, Value)>) -> Result<Value> {
        let mut q = self.client.query(query);
        for (name, value) in params {
            q = q.bind((name, value));
        }
        
        let mut response = q.await?;
        Ok(response.take(0)?)
    }
    
    async fn query_one<T: DeserializeOwned>(&self, query: &str, params: impl IntoIterator<Item = (String, Value)>) -> Result<Option<T>> {
        let value = self.query(query, params).await?;
        
        // For SELECT queries, SurrealDB returns Vec<T>
        if let Ok(mut results) = surrealdb::value::from_value::<Vec<T>>(value.clone()) {
            Ok(results.pop())
        } else {
            // For CREATE/UPDATE, might return single T
            Ok(surrealdb::value::from_value(value).ok())
        }
    }
}
```

### 3. Update Query builder to use SurrealDB types

```rust
pub struct Query {
    query: String,
    params: Vec<(String, surrealdb::Value)>,
}

impl Query {
    pub fn bind<T: Into<surrealdb::Value>>(mut self, name: &str, value: T) -> Self {
        self.params.push((name.to_string(), value.into()));
        self
    }
}
```

### 4. Fix parameter binding in query builders

Instead of binding JSON values, bind SurrealDB values directly:

```rust
impl AgentQueries {
    pub fn create(id: AgentId, user_id: UserId, agent_type: AgentType, name: String, system_prompt: String) -> Query {
        Query::new(format!(
            r#"CREATE agents:`{}` CONTENT {{
                user_id: users:`{}`,
                agent_type: $agent_type,
                name: $name,
                system_prompt: $system_prompt,
                ...
            }}"#,
            id, user_id
        ))
        .bind("agent_type", agent_type)  // Will use Into<Value>
        .bind("name", name)
        .bind("system_prompt", system_prompt)
    }
}
```

### 5. Implement proper serde for our ID types

```rust
impl Serialize for UserId {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        // Serialize as a SurrealDB Thing
        Thing {
            tb: "users".to_string(),
            id: Id::String(self.to_string()),
        }.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for UserId {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let thing = Thing::deserialize(deserializer)?;
        // Parse the ID from the Thing
        match thing.id {
            Id::String(s) => Ok(UserId::from_string(s)),
            _ => Err(D::Error::custom("expected string ID")),
        }
    }
}
```

## Benefits

1. **Simpler code**: Remove custom unwrapping logic
2. **Better performance**: No unnecessary conversions
3. **Type safety**: SurrealDB's from_value handles all the conversions
4. **Less abstraction**: We're using SurrealDB anyway, embrace it

## Migration Steps

1. ✅ Update DatabaseBackend trait to use surrealdb::Value
2. ✅ Simplify EmbeddedDatabase implementation
3. ✅ Update Query builder to use surrealdb::Value
4. ✅ Implement proper serde for ID types
5. ✅ Remove unwrap_surreal_value and related helpers
6. ✅ Update all ops to use new simplified API