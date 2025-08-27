# Entity System Architecture

Pattern uses a sophisticated macro-based entity system that provides compile-time safety, automatic type conversions, and seamless integration with SurrealDB's graph database features.

## Overview

The entity system is built around the `#[derive(Entity)]` proc macro which generates all the boilerplate code needed for database entities, including:

- Storage model structs with SurrealDB-compatible types
- Automatic conversions between domain types and storage types
- Schema generation with proper field definitions
- Relation management using SurrealDB's RELATE queries
- Convenient methods for storing and loading entities with their relations

## Basic Usage

### Simple Entity

```rust
use pattern_macros::Entity;
use pattern_core::id::UserId;
use chrono::{DateTime, Utc};
use serde::{Serialize, Deserialize};

#[derive(Debug, Clone, Entity, Serialize, Deserialize)]
#[entity(entity_type = "user")]
pub struct User {
    pub id: UserId,
    pub username: String,
    pub email: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}
```

This generates:
- `UserDbModel` struct with SurrealDB types
- `DbEntity` trait implementation
- Schema definition for the `user` table

### Using the Entity

```rust
// Create a new user
let user = User {
    id: UserId::generate(),
    username: "alice".to_string(),
    email: Some("alice@example.com".to_string()),
    created_at: Utc::now(),
    updated_at: Utc::now(),
};

// Store to database
let stored = user.store_with_relations(&db).await?;

// Load from database
let loaded = User::load_with_relations(&db, user_id).await?
    .expect("User not found");
```

## Field Attributes

### Skip Fields

Fields marked with `#[entity(skip)]` are not persisted to the database:

```rust
#[derive(Debug, Clone, Entity, Serialize, Deserialize)]
#[entity(entity_type = "agent")]
pub struct Agent {
    pub id: AgentId,
    pub name: String,
    
    #[entity(skip)]
    pub runtime_state: AgentState,  // Computed at runtime, not stored
}
```

### Custom Storage Types

Use `#[entity(db_type = "Type")]` to specify custom storage conversions:

```rust
#[derive(Debug, Clone, Entity, Serialize, Deserialize)]
#[entity(entity_type = "task")]
pub struct Task {
    pub id: TaskId,
    pub title: String,
    
    // Store as flexible JSON object
    pub metadata: serde_json::Value,
    
    // Store Vec<String> as comma-separated string
    #[entity(db_type = "String")]
    pub tags: Vec<String>,
}
```

### Relations

Fields marked with `#[entity(relation = "edge_name")]` are stored as graph relations:

```rust
#[derive(Debug, Clone, Entity, Serialize, Deserialize)]
#[entity(entity_type = "user")]
pub struct User {
    pub id: UserId,
    pub username: String,
    
    // ID-only relations (just store/load IDs)
    #[entity(relation = "owns")]
    pub owned_agent_ids: Vec<AgentId>,
    
    // Full entity relations (store/load complete entities)
    #[entity(relation = "created")]
    pub created_tasks: Vec<Task>,
}
```

## Relation Management

### How Relations Work

Instead of foreign keys, the entity system uses SurrealDB's RELATE to create edge tables:

```
user:123 -> owns -> agent:456
user:123 -> created -> task:789
```

This creates bidirectional relationships that can be traversed efficiently.

### ID vs Entity Relations

To avoid circular dependencies, you can choose between:

1. **ID Relations**: Store only the ID, load only the ID
   ```rust
   #[entity(relation = "owns")]
   pub owned_agent_ids: Vec<AgentId>,
   ```

2. **Entity Relations**: Store and load full entities
   ```rust
   #[entity(relation = "created")]
   pub created_tasks: Vec<Task>,
   ```

### Manual Relation Management

```rust
// Store relations manually
user.store_relations(&db).await?;

// Load relations manually
user.load_relations(&db).await?;
```

### Automatic Relation Management

The convenience methods handle relations automatically:

```rust
// Stores entity AND all relations
let stored = user.store_with_relations(&db).await?;

// Loads entity AND all relations
let loaded = User::load_with_relations(&db, user_id).await?;
```

## Generated Methods

The `#[derive(Entity)]` macro generates the following methods:

### Instance Methods

```rust
impl User {
    // Store all relation fields to the database
    pub async fn store_relations(&self, db: &Surreal<Db>) -> Result<(), DatabaseError>;
    
    // Load all relation fields from the database
    pub async fn load_relations(&mut self, db: &Surreal<Db>) -> Result<(), DatabaseError>;
    
    // Store entity with all relations (upserts if exists)
    pub async fn store_with_relations(&self, db: &Surreal<Db>) -> Result<Self, DatabaseError>;
}

## ID Handling and Record Keys

Pattern’s ID types implement a Display that is human-friendly (e.g., `agent:abcd…`, `group:ef01…`). This is great for logs and CLI output, but must not be used when persisting or constructing record keys for the database.

- Use `id.to_key()` when you need the raw record key (just the UUID-like portion) for persistence or composing IDs stored in other entities.
- Use `RecordId::from(&id)` when binding IDs in SurrealDB queries (`.bind(("agent_id", RecordId::from(&agent_id)))`).
- Avoid `id.to_string()` for any database-bound value. Display includes the table prefix, which can break UUID validations or schema expectations.

Common pitfalls and fixes:
- Tracker or cursor IDs derived from other IDs should use `.to_key()` rather than `.to_string()`.
- When storing string keys (e.g., `MemoryId`, cursor keys), prefer the raw key: `let key = some_id.to_key();`.

Caveat (rare acceptable `to_string()`):
- Some IDs are intentionally not table-prefixed and their Display equals the raw key (for example `Did`, and `MessageId` where the key is an arbitrary string). In those specific cases, `to_string()` may be acceptable if and only if you expect the raw key string. When in doubt, prefer `.to_key()` or `RecordId::from(&id)`.

Quick checklist:
- Persistence: `to_key()` or `RecordId::from(&id)`
- Display/logging: `to_string()`
- Never concatenate Display-form IDs into other record keys

Code review checklist:
- Are any IDs converted with `.to_string()` and then used in DB paths, record keys, or binds? If yes, replace with `.to_key()` or `RecordId::from(&id)` unless it’s one of the rare exceptions above.
- Are tuple APIs used correctly? Always pass `(table, id.to_key())` or `(table, id.to_record_id())`, not `(table, id.to_string())`.
```

### Associated Functions

```rust
impl User {
    // Load entity with all relations
    pub async fn load_with_relations(
        db: &Surreal<Db>, 
        id: Id<UserId>
    ) -> Result<Option<Self>, DatabaseError>;
}
```

### DbEntity Trait Implementation

```rust
impl DbEntity for User {
    type DbModel = UserDbModel;
    type Domain = Self;
    type Id = UserId;
    
    fn to_db_model(&self) -> Self::DbModel;
    fn from_db_model(db_model: Self::DbModel) -> Result<Self, EntityError>;
    fn table_name() -> &'static str;
    fn id(&self) -> Id<Self::Id>;
    fn schema() -> TableDefinition;
}
```

## Schema Generation

The entity system automatically generates SurrealDB schema definitions:

```sql
DEFINE TABLE OVERWRITE user SCHEMALESS;
DEFINE FIELD id ON TABLE user TYPE record;
DEFINE FIELD username ON TABLE user TYPE string;
DEFINE FIELD email ON TABLE user TYPE option<string>;
DEFINE FIELD created_at ON TABLE user TYPE datetime;
DEFINE FIELD updated_at ON TABLE user TYPE datetime;
DEFINE TABLE OVERWRITE owns SCHEMALESS;
DEFINE TABLE OVERWRITE created SCHEMALESS;
```

## Type Conversions

The system automatically handles conversions between Rust types and SurrealDB types:

| Rust Type | SurrealDB Storage Type |
|-----------|------------------------|
| `DateTime<Utc>` | `surrealdb::Datetime` |
| `UserId`, `AgentId`, etc. | `surrealdb::RecordId` |
| `Option<T>` | Nullable field |
| `Vec<T>` | Array field |
| `serde_json::Value` | FLEXIBLE object field |
| `Vec<String>` with `db_type = "String"` | Comma-separated string |
| Enums | String field (requires custom serialization) |

### Enum Serialization

Enums need custom serialization to work with SurrealDB's string field type:

```rust
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum BaseAgentType {
    Assistant,
    Custom(String),
}

impl Serialize for BaseAgentType {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        match self {
            BaseAgentType::Assistant => serializer.serialize_str("assistant"),
            BaseAgentType::Custom(s) => serializer.serialize_str(s),
        }
    }
}

impl<'de> Deserialize<'de> for BaseAgentType {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        match s.as_str() {
            "assistant" => Ok(BaseAgentType::Assistant),
            custom => Ok(BaseAgentType::Custom(custom.to_string())),
        }
    }
}
```

This ensures enums are stored as simple strings in the database, matching the schema's `TYPE string` declaration.

## Advanced Usage

### External Crates

When using the macro outside of pattern-core:

```rust
#[derive(Debug, Clone, Entity, Serialize, Deserialize)]
#[entity(entity_type = "user", crate_path = "::pattern_core")]
pub struct User {
    // ...
}
```

### Complex Relations

For many-to-many relationships with metadata:

```rust
// Edge table can store additional data
user:123 -> assigned{priority: "high", deadline: "2024-01-15"} -> task:789
```

### Ordering Relations

Relations are automatically ordered by ID to ensure consistent results:

```sql
SELECT id, ->owns->agent AS related_entities FROM $parent ORDER BY id ASC
```

## Best Practices

1. **Use ID relations to avoid circular dependencies**
   - If User has Tasks and Task has User, make one use IDs

2. **Leverage skip fields for runtime state**
   - Computed fields, caches, and temporary state shouldn't be stored

3. **Use appropriate storage types**
   - `serde_json::Value` for flexible data
   - Custom `db_type` for optimized storage

4. **Prefer convenience methods**
   - `store_with_relations()` and `load_with_relations()` handle everything

5. **Design relations thoughtfully**
   - Consider query patterns when choosing between ID and entity relations
   - Use edge metadata when relationships have properties

## Migration from Old System

If migrating from a foreign key-based system:

1. Replace foreign key fields with relation fields:
   ```rust
   // Old
   pub owner_id: UserId,
   
   // New
   #[entity(relation = "owned_by")]
   pub owner: User,
   ```

2. Update queries to use RELATE:
   ```rust
   // Old
   task.owner_id = user.id;
   
   // New (handled automatically by store_with_relations)
   task.owner = user;
   ```

3. Create migration queries to establish relations:
   ```sql
   -- Migrate foreign keys to relations
   LET $tasks = SELECT * FROM task WHERE owner_id;
   FOR $task IN $tasks {
       RELATE $task.owner_id->owns->$task.id;
   };
   ```

## Performance Considerations

- **ID relations are cheap**: Only store/load record IDs
- **Entity relations are fast**: SurrealDB's architecture fetches related entities efficiently without traditional joins
- **Graph traversal is optimized**: Following relations is faster than SQL joins due to direct record linking
- **Use batch operations**: Store multiple relations in one transaction
- **Consider lazy loading**: Load relations only when needed for very large datasets
- **Built-in indexing**: SurrealDB automatically optimizes relation traversals
