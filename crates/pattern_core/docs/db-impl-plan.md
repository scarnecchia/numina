Pattern Database Backend Implementation Plan

### Phase 1: Foundation (Core Abstractions)

#### 1.1 Database Trait System
```rust
// crates/pattern-core/src/db/mod.rs
trait DatabaseBackend: Send + Sync {
    async fn connect(config: DatabaseConfig) -> Result<Self>;
    async fn execute(&self, query: Query) -> Result<Response>;
    async fn transaction<F, R>(&self, f: F) -> Result<R>
        where F: FnOnce(&Transaction) -> Result<R>;
}

trait VectorStore: DatabaseBackend {
    async fn vector_search(
        &self,
        table: &str,
        vector: &[f32],
        limit: usize,
        filter: Option<Filter>,
    ) -> Result<Vec<SearchResult>>;
}
```

#### 1.2 Embedding Trait System
```rust
// crates/pattern-core/src/embeddings/mod.rs
trait EmbeddingProvider: Send + Sync {
    async fn embed(&self, text: &str) -> Result<Embedding>;
    async fn embed_batch(&self, texts: &[String]) -> Result<Vec<Embedding>>;
    fn model_id(&self) -> &str;
    fn dimensions(&self) -> usize;
}

struct Embedding {
    vector: Vec<f32>,
    model: String,
    dimensions: usize,
}
```

### Phase 2: Initial Implementations

#### 2.1 SurrealDB Embedded Backend
- Use `surrealkv` (pure Rust) initially
- Implement `DatabaseBackend` and `VectorStore` traits
- Connection pooling for concurrent access
- Schema migrations on startup

#### 2.2 Embedding Providers
- **Candle Provider**: BAAI/bge-small-en-v1.5 (384 dims)
- **OpenAI Provider**: text-embedding-3-small (1536 dims)
- **Cohere Provider**: embed-english-v3.0 (1024 dims)
- Model validation and dimension checking

### Phase 3: Schema Design

#### 3.1 Core Tables
```surql
-- Metadata table (singleton)
DEFINE TABLE system_metadata SCHEMAFULL;
DEFINE FIELD embedding_model ON system_metadata TYPE string;
DEFINE FIELD embedding_dimensions ON system_metadata TYPE int;
DEFINE FIELD schema_version ON system_metadata TYPE int;

-- Users (partners)
DEFINE TABLE users SCHEMAFULL;
DEFINE FIELD id ON users TYPE string;
DEFINE FIELD created_at ON users TYPE datetime;
DEFINE FIELD settings ON users TYPE object;

-- Agents
DEFINE TABLE agents SCHEMAFULL;
DEFINE FIELD id ON agents TYPE string;
DEFINE FIELD user_id ON agents TYPE string;
DEFINE FIELD type ON agents TYPE string;
DEFINE FIELD config ON agents TYPE object;
DEFINE FIELD state ON agents TYPE object;
DEFINE FIELD created_at ON agents TYPE datetime;
DEFINE FIELD updated_at ON agents TYPE datetime;
DEFINE INDEX agent_user ON agents COLUMNS user_id;

-- Memory blocks with embeddings
DEFINE TABLE memory_blocks SCHEMAFULL;
DEFINE FIELD id ON memory_blocks TYPE string;
DEFINE FIELD agent_id ON memory_blocks TYPE string;
DEFINE FIELD label ON memory_blocks TYPE string;
DEFINE FIELD content ON memory_blocks TYPE string;
DEFINE FIELD embedding ON memory_blocks TYPE array;
DEFINE FIELD embedding->vector ON memory_blocks TYPE array<float>;
DEFINE FIELD metadata ON memory_blocks TYPE object;
DEFINE FIELD created_at ON memory_blocks TYPE datetime;
DEFINE FIELD updated_at ON memory_blocks TYPE datetime;
DEFINE INDEX memory_agent ON memory_blocks COLUMNS agent_id;
DEFINE INDEX memory_embedding ON memory_blocks FIELDS embedding->vector HNSW DIMENSION 384 DIST COSINE;
```

#### 3.2 Relationships
```surql
-- Agent belongs to user
DEFINE FIELD user ON agents TYPE record(users);

-- Memory belongs to agent
DEFINE FIELD agent ON memory_blocks TYPE record(agents);
```

### Phase 4: Repository Pattern

#### 4.1 User Repository
```rust
struct UserRepository<DB: DatabaseBackend> {
    db: Arc<DB>,
}

impl UserRepository {
    async fn create(&self, user: NewUser) -> Result<User>;
    async fn get(&self, id: &str) -> Result<Option<User>>;
    async fn get_with_agents(&self, id: &str) -> Result<Option<UserWithAgents>>;
}
```

#### 4.2 Agent Repository
```rust
struct AgentRepository<DB: DatabaseBackend> {
    db: Arc<DB>,
}

impl AgentRepository {
    async fn create(&self, agent: NewAgent) -> Result<Agent>;
    async fn update_state(&self, id: &str, state: Value) -> Result<()>;
    async fn get_by_user(&self, user_id: &str) -> Result<Vec<Agent>>;
}
```

#### 4.3 Memory Repository
```rust
struct MemoryRepository<DB: VectorStore, E: EmbeddingProvider> {
    db: Arc<DB>,
    embeddings: Arc<E>,
}

impl MemoryRepository {
    async fn create(&self, memory: NewMemory) -> Result<Memory>;
    async fn search_semantic(
        &self,
        agent_id: &str,
        query: &str,
        limit: usize,
    ) -> Result<Vec<MemorySearchResult>>;
    async fn update(&self, id: &str, content: &str) -> Result<()>;
}
```

### Phase 5: Service Layer

#### 5.1 Database Service
```rust
pub struct DatabaseService {
    backend: Arc<dyn DatabaseBackend>,
    embeddings: Arc<dyn EmbeddingProvider>,
    users: UserRepository,
    agents: AgentRepository,
    memory: MemoryRepository,
}

impl DatabaseService {
    pub async fn new(config: DatabaseConfig) -> Result<Self>;
    pub async fn initialize_schema(&self) -> Result<()>;
    pub async fn validate_embedding_compatibility(&self) -> Result<()>;
}
```

### Phase 6: Configuration & Migration

#### 6.1 Configuration
```rust
#[derive(Deserialize)]
struct DatabaseConfig {
    #[serde(default = "default_backend")]
    backend: DatabaseBackend,

    #[serde(default)]
    embedded: EmbeddedConfig,

    #[serde(default)]
    remote: Option<RemoteConfig>,
}

#[derive(Deserialize)]
struct EmbeddingConfig {
    #[serde(default = "default_provider")]
    provider: EmbeddingProvider,

    model: Option<String>,
    api_key: Option<String>,

    #[serde(default)]
    cache_embeddings: bool,
}
```

#### 6.2 Migration System
```rust
trait Migration {
    fn version(&self) -> u32;
    fn description(&self) -> &str;
    async fn up(&self, db: &dyn DatabaseBackend) -> Result<()>;
    async fn down(&self, db: &dyn DatabaseBackend) -> Result<()>;
}

struct MigrationRunner {
    migrations: Vec<Box<dyn Migration>>,
}
```

### Phase 7: Error Handling

```rust
#[derive(Error, Debug, Diagnostic)]
pub enum DatabaseError {
    #[error("Connection failed")]
    ConnectionFailed(#[source] Box<dyn std::error::Error + Send + Sync>),

    #[error("Embedding model mismatch: database has {db_model}, config specifies {config_model}")]
    EmbeddingModelMismatch {
        db_model: String,
        config_model: String,
    },

    #[error("Query failed")]
    QueryFailed(#[source] surrealdb::Error),
}
```

### Phase 8: Testing Strategy

1. **Unit Tests**: Mock implementations of traits
2. **Integration Tests**: In-memory SurrealDB with real embeddings
3. **Embedding Tests**: Validate dimensions and similarity scores
4. **Migration Tests**: Up/down migration verification

### Implementation Order

1. **Week 1**:
   - Database traits and embedded SurrealDB implementation
   - Basic schema and migration system
   - Error types with miette

2. **Week 2**:
   - Embedding traits and providers (Candle + OpenAI)
   - Memory repository with vector search
   - Configuration system

3. **Week 3**:
   - User and Agent repositories
   - Service layer integration
   - Comprehensive testing

### Success Criteria

- [ ] Can store and retrieve users, agents, and memory blocks
- [ ] Vector similarity search returns relevant results
- [ ] Embedding provider can be switched via config
- [ ] Schema migrations run automatically
- [ ] All operations are async and thread-safe
- [ ] Comprehensive error messages with suggestions
- [ ] 80%+ test coverage

### Future Enhancements (Post-MVP)

1. Conversation history with embeddings
2. Task management with vector search
3. Tool usage analytics
4. Remote SurrealDB support
5. Additional embedding providers (Ollama, Cohere)
6. Embedding caching layer
7. Batch processing for large imports

---

This plan provides a solid foundation while keeping complexity manageable. We start with the essential features and can layer on more sophisticated functionality once the core is stable.

Ready to start implementing?
