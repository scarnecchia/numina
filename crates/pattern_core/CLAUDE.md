# CLAUDE.md - Pattern Core

This crate provides the core agent framework, memory management, and tool execution system that powers Pattern's multi-agent cognitive support system. Inspired by MemGPT's architecture for building stateful agents on stateless LLMs.

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
- Automatic handling of SurrealDB's nested value format via `unwrap_surreal_value`
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