# Pattern Core Database Backend

This document describes the database backend implementation for Pattern Core, including the data model, embedding support, and migration system.

**Status**: Core functionality implemented. Embeddings and vector search still need to be completed.

## Architecture Overview

The database backend is designed with flexibility in mind, supporting both embedded and remote databases through a trait-based abstraction layer.

### Key Components

1. **Database Traits** (`db/mod.rs`)
   - `DatabaseBackend`: Core database operations
   - `DatabaseOperations`: Generic operations (transactions)
   - `VectorStore`: Vector search capabilities
   - `Transaction`: Transaction operations

2. **Embedded Implementation** (`db/embedded.rs`)
   - Uses SurrealDB with SurrealKv (pure Rust backend)
   - Supports vector indexes via HNSW
   - In-process database for single-node deployments

3. **Schema Management** (`db/schema.rs`)
   - Strongly-typed table definitions
   - Model structs with serde serialization
   - Vector embedding support built-in

4. **Direct Operations** (`db/ops.rs`)
   - Simple function-based data access
   - Type-safe operations without abstraction overhead
   - No unnecessary repository pattern

5. **Migration System** (`db/migration.rs`)
   - Simple version-based migrations
   - Automatic schema updates on startup
   - Rollback support for development

## Data Model

The database uses the Entity macro system to define tables. Core entities are defined using `#[derive(Entity)]` which automatically generates the schema.

### Core Tables

#### System Metadata
```sql
DEFINE TABLE system_metadata SCHEMALESS;
DEFINE FIELD embedding_model ON system_metadata TYPE string;
DEFINE FIELD embedding_dimensions ON system_metadata TYPE int;
DEFINE FIELD schema_version ON system_metadata TYPE int;
DEFINE FIELD created_at ON system_metadata TYPE datetime;
DEFINE FIELD updated_at ON system_metadata TYPE datetime;
```

#### Primary Entities (via Entity macro)

**Users**
- Generated from `User` struct in `users` module
- Fields: id, username, email, settings, metadata, created_at, updated_at
- Relations: owns agents, owns constellations

**Agents** 
- Generated from `AgentRecord` struct
- Fields: id, name, agent_type, state, model config, context settings, statistics
- Relations: owned by user, has memories, has messages, assigned tasks

**Memory Blocks**
- Generated from `MemoryBlock` struct  
- Fields: id, owner_id, label, value, description, memory_type, permission, embedding
- Memory types: Core (always in context), Working (swappable), Archival (searchable)
- Permissions: ReadOnly, Partner, Human, Append, ReadWrite, Admin

**Messages**
- Generated from `Message` struct
- Fields: id, role, content, metadata, embedding, created_at
- Content types: Text, Parts (multimodal), ToolCalls, ToolResponses
- Snowflake ID positioning for distributed ordering

**Tasks**
- Generated from `BaseTask` struct
- Fields: id, title, description, status, priority, due_date, creator_id
- Relations: parent/subtasks, assigned agent

**Tool Calls**
- Generated from `ToolCall` struct
- Fields: id, agent_id, tool_name, parameters, result, error, duration_ms
- Audit trail for all tool executions

## Embedding Support

The database backend integrates with the embeddings module to provide semantic search capabilities.

### Embedding Providers

1. **Candle (Local)** ✅
   - Pure Rust implementation
   - Supports BERT-based models
   - Default: BAAI/bge-small-en-v1.5 (384 dims)

2. **Cloud Providers** ✅
   - OpenAI: text-embedding-3-small/large
   - Cohere: embed-english-v3.0
   - API key configuration required

3. **Ollama** 🚧
   - Stub implementation only
   - Planned for future release

### Embedding Storage

- Vectors stored directly in SurrealDB
- HNSW indexes for fast similarity search
- Cosine distance metric for text embeddings
- Automatic re-embedding on content updates

### Vector Search (TODO)

```rust
// Planned: Semantic memory search
// Currently embeddings are stored but search is not implemented
let results = db::ops::search_archival_memories(
    &db,
    agent_id,
    "query text",
    10
).await?;
```

## Direct Operations

All database operations are exposed as simple functions in `db::ops`:

### User Operations
- `create_user()` - Create a new user with constellation
- `get_user_by_id()` - Get user by ID
- `get_user_by_username()` - Get user by username

### Agent Operations  
- `create_agent_for_user()` - Create agent in user's constellation
- `get_agent_with_relations()` - Get agent with memories and messages
- `persist_agent()` - Save full agent state
- `persist_agent_message()` - Save message with relation type

### Memory Operations
- `create_memory_block()` - Create memory with permissions
- `get_agent_memories()` - Get all memories for an agent
- `search_archival_memories()` - Full-text search (BM25)
- `insert_archival_memory()` - Add to archival storage
- `delete_archival_memory()` - Remove from archival

### Group Operations
- `create_group_for_user()` - Create agent group
- `add_agent_to_group()` - Add member with role
- `get_group_members()` - List group agents
- `list_groups_for_user()` - User's groups

This approach keeps the code simple and direct without unnecessary abstraction layers.

## Configuration

### Database Configuration
```toml
[database]
type = "embedded"
path = "./pattern.db"
strict_mode = false

# Or for remote (when implemented)
# type = "remote"
# url = "ws://localhost:8000"
# namespace = "pattern"
# database = "main"
```

### Embedding Configuration
```toml
[embeddings]
provider = "candle"
model = "BAAI/bge-small-en-v1.5"

# Or for cloud
# provider = "openai"
# model = "text-embedding-3-small"
# api_key = "${OPENAI_API_KEY}"
```

## Migration System

The migration system ensures database schema consistency across versions:

1. **Automatic Migration**: Runs on startup
2. **Version Tracking**: Stored in system_metadata
3. **Idempotent**: Safe to run multiple times
4. **Rollback Support**: For development/testing

### Adding a Migration

1. Add a new migration function in `migration.rs`
2. Increment the version check in `run_migrations()`
3. Implement the schema changes
4. Test with both fresh and existing databases

## Feature Flags

The database backend supports multiple configurations via feature flags:

- `surreal-embedded`: Embedded SurrealDB (default) ✅
- `surreal-remote`: Remote SurrealDB connection 🚧 Not implemented
- `embed-candle`: Local Candle embeddings (default) ✅
- `embed-cloud`: Cloud embedding providers (default) ✅
- `embed-ollama`: Ollama embedding support 🚧 Stub only

## Testing

The database module includes comprehensive tests:

1. **Unit Tests**: Direct operation functions
2. **Integration Tests**: Full database workflows
3. **Migration Tests**: Schema update verification
4. **Embedding Tests**: Vector operations

Run tests with:
```bash
cargo test --features surreal-embedded,embed-candle
```

## Performance Considerations

1. **Connection Pooling**: Single connection for embedded DB
2. **Vector Indexes**: HNSW for approximate nearest neighbor
3. **Batch Operations**: Supported for embeddings
4. **Caching**: Consider caching frequent queries in production

## Future Enhancements

1. **Remote Database Support**: For distributed deployments
2. **Complete Ollama Provider**: Full implementation of stub
3. **Additional Embedding Providers**: Local ONNX models
4. **Vector Index Tuning**: Configurable HNSW parameters
5. **Query Optimization**: Prepared statements, query plans
6. **Backup/Restore**: Automated database backups