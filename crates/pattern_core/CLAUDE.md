# CLAUDE.md - Pattern Core

This crate provides the core agent framework, memory management, and tool execution system that powers Pattern's multi-agent cognitive support system.

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

### Memory Management
- Memory blocks are soft-limited by character count
- Each block has a label, value, optional description, and last_modified timestamp
- No versioning in the current implementation (kept simple)

### Context Building
- ContextBuilder follows builder pattern for flexibility
- Compression happens automatically when message count exceeds limits
- Support for XML-style tags or plain text formatting based on model

### Database Backend (`db/`)
- Trait-based abstraction (`DatabaseBackend`, `VectorStore`) for multiple implementations
- Pure Rust embedded database using SurrealKV (no RocksDB dependency)
- Schema migrations with version tracking
- Vector search support with HNSW indexes
- Direct operations via functions in `db::ops` (no unnecessary repository pattern)
- Type-safe IDs (UserId, AgentId, etc.) throughout the codebase
- Automatic handling of SurrealDB's nested value format via `unwrap_surreal_value`

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
- BERT models (`BAAI/bge-*`) fail with "unsupported dtype F32 for op index-select" error due to Candle's BERT implementation. Use Jina models for local embeddings or cloud providers for production.

### Serialization
### Serialization
- All `Option<T>` fields use `#[serde(skip_serializing_if = "Option::is_none")]`
- Duration fields use custom serialization (as milliseconds)
- Avoid nested structs in types that need MCP-compatible schemas
- SurrealDB responses wrap data in nested structures - use `unwrap_surreal_value` helper
- Custom serde implementations for enums (e.g., AgentType serializes Custom variants with `custom:` prefix)

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

## Performance Considerations

- CompactString inlines strings ≤ 24 bytes
- DashMap shards internally for concurrent access
- Message compression is async but happens during context building
- Token estimation is rough (4 chars ≈ 1 token)

## Future Considerations

- ~~Vector embeddings for semantic memory search~~ ✅ Implemented
- Streaming response support
- Multi-modal message content
- Advanced compression strategies (semantic clustering)
- Remote SurrealDB support
- Complete Ollama embedding provider (currently stub)
- Fix BERT model support in Candle (dtype issues)
- Additional embedding providers (local ONNX, etc.)
- Batch embedding operations
- Cross-agent memory sharing