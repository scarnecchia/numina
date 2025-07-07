# Pattern Core Database Backend

This document describes the database backend implementation for Pattern Core, including the data model, embedding support, and migration system.

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

### Core Tables

#### System Metadata
```sql
DEFINE TABLE system_metadata SCHEMAFULL;
DEFINE FIELD embedding_model ON system_metadata TYPE string;
DEFINE FIELD embedding_dimensions ON system_metadata TYPE int;
DEFINE FIELD schema_version ON system_metadata TYPE int;
```

#### Users (Partners)
```sql
DEFINE TABLE users SCHEMAFULL;
DEFINE FIELD id ON users TYPE string;
DEFINE FIELD settings ON users TYPE object;
DEFINE FIELD metadata ON users TYPE object;
```

#### Agents
```sql
DEFINE TABLE agents SCHEMAFULL;
DEFINE FIELD id ON agents TYPE string;
DEFINE FIELD user_id ON agents TYPE string;
DEFINE FIELD agent_type ON agents TYPE string;
DEFINE FIELD name ON agents TYPE string;
DEFINE FIELD system_prompt ON agents TYPE string;
DEFINE FIELD config ON agents TYPE object;
DEFINE FIELD state ON agents TYPE object;
DEFINE FIELD is_active ON agents TYPE bool DEFAULT true;
```

#### Memory Blocks (with embeddings)
```sql
DEFINE TABLE memory_blocks SCHEMAFULL;
DEFINE FIELD id ON memory_blocks TYPE string;
DEFINE FIELD agent_id ON memory_blocks TYPE string;
DEFINE FIELD label ON memory_blocks TYPE string;
DEFINE FIELD content ON memory_blocks TYPE string;
DEFINE FIELD embedding ON memory_blocks TYPE array<float>;
DEFINE FIELD embedding_model ON memory_blocks TYPE string;
DEFINE INDEX memory_embedding ON memory_blocks FIELDS embedding 
  HNSW DIMENSION 384 DIST COSINE;
```

#### Additional Tables
- **conversations**: Message threads between users and agents
- **messages**: Individual messages with optional embeddings
- **tool_calls**: Audit trail of tool usage
- **tasks**: ADHD-aware task management with embeddings

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

### Vector Search

```rust
// Semantic memory search using direct operations
let results = db::ops::search_memories(
    &db, 
    &embeddings,
    agent_id, 
    "query text", 
    10
).await?;

// Results include similarity scores
for (memory, score) in results {
    println!("{}: {}", memory.label, score);
}
```

## Direct Operations

All database operations are exposed as simple functions in `db::ops`:

### User Operations
- `create_user()` - Create a new user
- `get_user()` - Get user by ID
- `get_user_with_agents()` - Get user and their agents in one query

### Agent Operations
- `create_agent()` - Create a new agent
- `get_user_agents()` - Get all agents for a user
- `update_agent_state()` - Update agent state

### Memory Operations
- `create_memory()` - Create memory block with automatic embedding
- `search_memories()` - Semantic search with embeddings
- `get_memory_by_label()` - Direct label lookup

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