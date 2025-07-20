# CLAUDE.md - Pattern Core

This crate provides the core agent framework, memory management, and tool execution system that powers Pattern's multi-agent cognitive support system. Inspired by MemGPT's architecture for building stateful agents on stateless LLMs.

## Modular Database Entity System ✅ COMPLETED (2025-01-10)

### Overview
Successfully implemented a proc macro-based system for extensible database entities that:
- Keeps ADHD-specific types in pattern-nd
- Uses compile-time proc macros instead of runtime registration
- Properly uses SurrealDB's graph relationships with RELATE
- Provides automatic relation management with convenience methods

### Implementation
The entity system is built around the `#[derive(Entity)]` proc macro which generates all necessary boilerplate. See [Entity System Architecture](../../docs/architecture/ENTITY_SYSTEM.md) for comprehensive documentation.

Example usage:
```rust
#[derive(Debug, Clone, Entity, Serialize, Deserialize)]
#[entity(entity_type = "user")]
pub struct User {
    pub id: UserId,
    pub username: String,
    
    // Skip fields - not persisted
    #[entity(skip)]
    pub runtime_state: UserState,
    
    // ID relations - just store/load IDs
    #[entity(relation = "owns")]
    pub owned_agent_ids: Vec<AgentId>,
    
    // Full entity relations
    #[entity(relation = "created")]
    pub created_tasks: Vec<Task>,
}

// Usage with automatic relation management
let stored = user.store_with_relations(&db).await?;
let loaded = User::load_with_relations(&db, user_id).await?;
```

### Key Features

1. **Proc Macro Approach**
   - `#[derive(Entity)]` generates storage types, conversions, and methods
   - Field attributes: `skip`, `relation`, `db_type`, `crate_path`
   - Automatic schema generation with proper SurrealDB types

2. **SurrealDB Graph Relationships**
   - No foreign keys - all relationships use RELATE
   - Bidirectional traversal support
   - Relations ordered by ID for consistent results
   - Edge tables can store relationship metadata

3. **Automatic Type Conversions**
   - `DateTime<Utc>` ↔ `surrealdb::Datetime`
   - `UserId/AgentId/etc` ↔ `surrealdb::RecordId`
   - `serde_json::Value` → FLEXIBLE object
   - Custom conversions via `db_type` attribute
   - Enums serialize as strings (with custom Serialize/Deserialize)

4. **Convenience Methods**
   - `store_with_relations()` - upserts entity and all relations
   - `load_with_relations()` - loads entity with all relations
   - Manual control via `store_relations()` and `load_relations()`

### Benefits Achieved
- ✅ Zero boilerplate for entity definitions
- ✅ Type-safe relation management
- ✅ Optimized SurrealDB graph traversals (faster than SQL joins!)
- ✅ Flexible relation types (ID-only or full entities)
- ✅ Domain separation (ADHD types in pattern-nd)

### Important Implementation Notes

1. **Enum Serialization**: Enums like `BaseAgentType` must implement custom serialization to work with SurrealDB's string field type:
   ```rust
   impl Serialize for BaseAgentType {
       fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
       where S: serde::Serializer,
       {
           match self {
               BaseAgentType::Assistant => serializer.serialize_str("assistant"),
               BaseAgentType::Custom(s) => serializer.serialize_str(s),
           }
       }
   }
   ```

2. **Relation Ordering**: Relations are automatically ordered by ID to ensure consistent results across queries. The ORDER BY clause is added to all relation queries.

3. **Upsert Behavior**: `store_with_relations()` uses UPSERT internally, so it's safe to call multiple times without creating duplicates.

## Recent Updates (2025-01-10)

### Completed
1. **Entity System with Proc Macros** ✅
   - `#[derive(Entity)]` with full relation support
   - Automatic storage type generation
   - Built-in convenience methods
   - Comprehensive test coverage
   - Fixed enum serialization for SurrealDB compatibility

2. **Relation Management** ✅
   - ID-only and full entity relations
   - Automatic loading/storing with `*_with_relations` methods
   - Ordered results for consistency
   - No traditional joins needed!

3. **Type System Integration** ✅
   - All ID types use consistent format
   - Proper SurrealDB type mappings
   - Custom storage via `db_type` attribute
   - Enum fields serialize as strings

4. **Base Entity Refactor** ✅
   - Updated BaseUser, BaseAgent, BaseTask, BaseEvent, BaseMemoryBlock
   - Added proper relation fields (owns, created, assigned, etc.)
   - Removed foreign key fields in favor of relations
   - All base entities now use the Entity macro

### Documentation TODOs
1. **Tool execution flow**
   - registration pathway
   - dynamic tool conversion
   - registry execution model
   - built-in tools integration

2. **Embedding provider selection guide**
   - when to use which provider
   - dimension selection criteria
   - performance/cost tradeoffs

### Next Steps
1. **Update database operations**
   - Replace manual RELATE queries with entity methods
   - Use `store_with_relations` throughout codebase
   - Remove old relation helper functions
   - Update tests to use new entity system

2. **Migration Guide**
   - Document how to migrate from foreign keys to relations
   - Provide examples of common patterns
   - Create migration scripts for existing data

## Core Principles

- **Type Safety First**: Use generics and strong typing wherever possible (see tool.rs refactor)
- **Memory Efficiency**: Use CompactString for small string optimization, Box instead of Arc when mutability might be needed
- **Concurrency**: DashMap for thread-safe collections, avoid unnecessary locking
- **Pure Rust**: No C dependencies in this crate
- **Error Clarity**: Use miette for rich error diagnostics with helpful context

## Architecture Overview

### Key Components

1. **Agent System** (`agent.rs`)
   - Base `Agent` trait that all agents implement
   - AgentType enum with feature-gated ADHD variants
   - Message/Response types with metadata
   - AgentBuilder for constructing agents

2. **Memory System** (`memory.rs`)
   - MemGPT-style memory blocks with character limits
   - Persistent memory between conversations
   - Simple key-value structure with descriptions

3. **Tool System** (`tool.rs`)
   - Type-safe `AiTool<Input, Output>` trait
   - Dynamic dispatch via `DynamicTool` trait
   - Thread-safe `ToolRegistry` using DashMap
   - MCP-compatible schema generation (no $ref)
   - Built-in tools in `tool/builtin/` subdirectory

4. **Context Building** (`context/`)
   - Stateful agents on stateless protocols
   - Multiple compression strategies
   - genai compatibility layer
   - Agent state management with checkpoints

5. **Model Abstraction** (`model.rs`)
   - Provider-agnostic interface
   - Capability-based model selection
   - Structured types for completions

## Important Implementation Details

### Tool System
- Tools must be `Clone` to work with the registry
- Schema generation uses `schemars` with `inline_subschemas = true` for MCP compatibility
- DynamicToolAdapter provides type erasure while preserving type safety
- Built-in tools (update_memory, send_message) use AgentHandle for efficient access
- Built-in tools are registered in the same ToolRegistry as external tools
- BuiltinTools::builder() allows customization of default implementations

### Memory Management
- Memory blocks are soft-limited by character count
- Each block has a label, value, optional description, and last_modified timestamp
- No versioning in the current implementation (kept simple)
- Memory uses Arc<DashMap> internally for thread-safe concurrent access

### Context Building
- ContextBuilder follows builder pattern for flexibility
- Compression happens automatically when message count exceeds limits
- Support for XML-style tags or plain text formatting based on model
- AgentContext split into:
  - AgentHandle: Cheap, cloneable handle with agent_id and memory
  - MessageHistory: Large message data behind Arc<RwLock<_>>
  - Metadata behind Arc<RwLock<_>> for concurrent updates

### Database Backend (`db/`)
- Pure Rust embedded database using SurrealKV
- Schema migrations with version tracking
- Vector search support with HNSW indexes
- Direct operations via functions in `db::ops`
- Type-safe IDs (UserId, AgentId, etc.) throughout the codebase
  - All IDs use underscore separator format: `prefix_uuid` (e.g., `agent_12345678-...`)
  - MemoryIdType uses "mem" prefix, not "memory"
  - MessageId exception: plain string UUID for API compatibility
- Automatic handling of SurrealDB's nested value format via `unwrap_surreal_value`
- **Graph relationships using RELATE**:
  - No direct foreign keys in entities
  - Edge tables for all relationships
  - Bidirectional traversal support
  - Relationship metadata in edge tables
- **Macro-based entity system**:
  - `define_entity!` macro for minimal boilerplate
  - Compile-time field validation
  - Automatic schema generation
- **Important**: See [`db/SURREALDB_PATTERNS.md`](./src/db/SURREALDB_PATTERNS.md) for query patterns and response handling

### Embeddings (`embeddings/`)
- Provider trait for multiple backends
- Feature-gated implementations:
  - ✅ Candle (local) - Pure Rust embeddings with Jina models
  - ✅ OpenAI - text-embedding-3-small/large
  - ✅ Cohere - embed-english-v3.0
  - 🚧 Ollama - Stub implementation only
- Automatic embedding generation for memory blocks
- Cosine similarity for semantic search
- Model validation and dimension checking
- Supported Candle models:
  - `jinaai/jina-embeddings-v2-small-en` (512 dims) ✅
  - `jinaai/jina-embeddings-v2-base-en` (768 dims) ✅

**Known Issues:**
- BERT models (`BAAI/bge-*`) fail with dtype errors in Candle. Use Jina models for local embeddings.

### Serialization
- All `Option<T>` fields use `#[serde(skip_serializing_if = "Option::is_none")]`
- Duration fields use custom serialization (as milliseconds)
- Avoid nested structs in types that need MCP-compatible schemas
- SurrealDB responses wrap data in nested structures - use `unwrap_surreal_value` helper
- **SurrealDB Gotchas**: Response `.take()` can return nested vectors, DELETE queries need special handling - see patterns doc

## Common Patterns

### Creating a Tool
```rust
#[derive(Debug, Clone)]
struct MyTool;

#[async_trait]
impl AiTool for MyTool {
    type Input = MyInput;   // Must impl JsonSchema + Deserialize + Serialize
    type Output = MyOutput; // Must impl JsonSchema + Serialize

    fn name(&self) -> &str { "my_tool" }
    fn description(&self) -> &str { "Does something useful" }

    async fn execute(&self, params: Self::Input) -> Result<Self::Output> {
        // Implementation
    }
}
```

### Built-in Tools Pattern
```rust
// Creating built-in tools with AgentHandle
let handle = AgentHandle {
    agent_id: agent_id.clone(),
    memory: memory.clone(),
};

// Register default built-in tools
let builtin = BuiltinTools::default_for_agent(handle);
builtin.register_all(&registry);

// Or customize built-in tools
let builtin = BuiltinTools::builder()
    .with_memory_tool(CustomMemoryTool::new())
    .build_for_agent(handle);
```

### Error Handling
```rust
// Use specific error variants with context
return Err(CoreError::tool_not_found(name, available_tools));

// For complex errors, use the builder methods
return Err(CoreError::memory_not_found(
    &agent_id,
    &block_name,
    available_blocks
));
```

## Testing Guidelines

- Test actual behavior, not mocks
- All error paths should be tested
- Use `tokio_test::block_on` for async tests
- Prefer integration-style tests over unit tests
- Keep slow tests (e.g., Candle embeddings) in integration tests, not unit tests
- Unit test suite should run in <1 second
- Use MockModelProvider and MockEmbeddingProvider for testing agent functionality

## Performance Considerations

- CompactString inlines strings ≤ 24 bytes
- DashMap shards internally for concurrent access
- Message compression is async but happens during context building
- Token estimation is rough (4 chars ≈ 1 token)
- Memory uses Arc<DashMap> for efficient cloning and thread-safe access
- AgentHandle provides cheap cloning for built-in tools
- MessageHistory behind RwLock to avoid cloning large message vectors

## Future Considerations

- Streaming response support
- Multi-modal message content
- Advanced compression strategies (semantic clustering)
- Remote SurrealDB support
- Complete Ollama embedding provider (currently stub)
- Fix BERT model support in Candle (dtype issues)
- Additional embedding providers (local ONNX, etc.)
- Batch embedding operations
- ~~Cross-agent memory sharing~~ ✅ Implemented
- Agent groups (dynamic, supervisor, sleeptime managers)
- Modular entity system with macro-based definitions
- Graph-based relationships using SurrealDB RELATE
