# CLAUDE.md - Pattern Core

Core agent framework, memory management, and coordination system for Pattern's multi-agent ADHD support. Inspired by MemGPT architecture for stateful agents.

## Current Status (2025-07-21)

### Entity System ✅ COMPLETE
- Proc macro-based entities with `#[derive(Entity)]`
- SurrealDB graph relationships using RELATE
- Message-agent relationships with Snowflake IDs
- Zero-boilerplate relation management

### DatabaseAgent ✅ COMPLETE
- Non-blocking persistence with optimistic updates
- Full state recovery from AgentRecord
- Memory sharing via edge entities
- Message compression support ready

### Agent Groups ✅ COMPLETE
- Dynamic, round-robin, sleeptime, pipeline managers
- Selector registry with load balancing, random, etc.
- All tests passing (92/92)

## Critical Implementation Notes

### Entity System Sacred Patterns ⚠️

1. **Edge Entity Pattern - DO NOT CHANGE**:
   ```rust
   // In the macro (pattern_macros/src/lib.rs):
   // YES THIS LOOKS WEIRD AND REDUNDANT. DO NOT CHANGE, IT BREAKS THE MACRO!!!!
   if let (Some(relation_name), Some(edge_entity)) =
       (&field_opts.relation, &field_opts.edge_entity)
   {
       // Edge entity handling...
   }
   ```

   But users should ONLY specify `edge_entity` attribute:
   ```rust
   // Correct usage:
   #[entity(edge_entity = "agent_memories")]
   pub memories: Vec<(TestMemory, AgentMemoryRelation)>,

   // WRONG - don't specify both:
   // #[entity(relation = "agent_memories", edge_entity = "AgentMemoryRelation")]
   ```

2. **Enum Serialization for SurrealDB**:
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

3. **Complex Types with `db_type = "object"`**:
   ```rust
   #[entity(db_type = "object")]
   pub compression_strategy: CompressionStrategy,
   ```
   Generates `FLEXIBLE TYPE object` for enums with associated data.

### Database Patterns (CRITICAL)

1. **SurrealDB Response Handling**:
   - **NEVER** try to get `Vec<serde_json::Value>` from a SurrealDB response - it "literally NEVER works"
   - **ALWAYS** print the raw response before attempting `.take()` when debugging
   - Response `.take()` can return nested vectors, DELETE queries need special handling
   - Use `unwrap_surreal_value` helper for nested value extraction

2. **ID Format Consistency**:
   - All IDs use underscore separator: `prefix_uuid` (e.g., `agent_12345678-...`)
   - MemoryIdType uses "mem" prefix, not "memory"
   - MessageId exception: preserves full string for API compatibility

3. **Parameterized Queries**:
   - **ALWAYS** use parameter binding to prevent SQL injection
   - Example: `query("SELECT * FROM user WHERE id = $id").bind(("id", user_id))`

### Technical Decisions

1. **Free Functions over Extension Traits**:
   - All database operations in `db::ops` as free functions
   - No `use SomeExt` imports needed
   - Cleaner, more discoverable API

2. **Optimistic Updates Pattern**:
   - UI responds immediately
   - Database syncs in background via `tokio::spawn`
   - Clone minimal data for background tasks

3. **Snowflake IDs for Message Ordering**:
   - Using ferroid crate for distributed monotonic IDs
   - Stored as String in `position` field
   - Guarantees order even with rapid message creation

## Tool System Refactoring (In Progress)

Following Letta/MemGPT patterns, tools are being refactored into domain-based multi-operation tools:

### Completed Tools
1. **manage_core_memory** ✅ - Operations on core memory blocks
   - `append` - Add content to existing memory
   - `replace` - Replace specific content within memory

### Completed Tools
2. **manage_archival_memory** ✅ - Long-term storage operations
   - `insert` - Add new memories to archival storage (creates MemoryBlock with MemoryType::Archival)
   - `search` - Search through archived memories (uses DB when available, falls back to DashMap)
   - `delete` - Remove archived memories
   - `read` - Read specific archival memory by label

   **Implementation Notes**:
   - Archival memories are stored as regular MemoryBlocks with `MemoryType::Archival`
   - AgentHandle now has private DB connection with controlled access methods
   - DB methods: `search_archival_memories`, `insert_archival_memory`, `delete_archival_memory`, `count_archival_memories`
   - Graceful fallback: tries DB first, falls back to in-memory if unavailable
   - Full-text search working with SurrealDB's @@ operator and BM25 analyzer
   - Uses computed field `archival_memories` on agent table for efficient access
   - Search uses: `SELECT archival_memories FROM agent WHERE id = $agent_id AND archival_memories.*.value @@ $search_term`
   - List uses: `SELECT *, ->agent_memories->mem AS memories FROM $agent_id FETCH memories`
   - Successfully tested with CLI debug tools

3. **search_conversations** ✅ - Query message history
   - Filters: time range, participant, content search, role
   - Returns relevant messages with optional context
   - Uses database full-text search when available
   - Graceful fallback if no DB connection
   - CLI debug tool implemented for testing

4. **interact_with_files** - File system operations
   - `read` - Read file contents
   - `search` - Search within files
   - `edit` (optional) - Modify files

5. **swap_memory** - Memory management between core and archival
   ```rust
   pub struct SwapMemoryInput {
       pub swap_out: Vec<String>,  // Labels to move from core to archival
       pub swap_in: Vec<String>,   // Labels to move from archival to core
       pub reason: String,         // Why the swap is needed
   }
   ```

### Implementation Notes
- Each tool has a single entry point with operation selection
- Tool usage rules are bundled with tools (via `usage_rule()` trait method)
- ToolRegistry automatically provides rules to context builder
- Operations use enums for type safety

## Next Priorities
1. ~~Implement search_conversations tool with database queries~~ ✅
2. Implement swap_memory tool for memory management
3. Add vector search for archival memory using embeddings
4. Implement message compression with archival when context window fills


## Eventually
 - Add provider capability flags to ModelProvider trait for multi-modal support
   - Currently using lowest-common-denominator approach (converting Parts to Text)
   - Some providers support multi-modal assistant responses, others don't
   - Would allow smarter content conversion based on provider capabilities

## Core Principles

- **Type Safety First**: Use generics and strong typing wherever possible
- **Memory Efficiency**: CompactString for small strings, Arc<DashMap> for thread-safe collections
- **Pure Rust**: No C dependencies in this crate
- **Error Clarity**: Use miette for rich error diagnostics

## Architecture Overview

### Key Components

1. **Agent System** (`agent/`, `context/`)
   - Base `Agent` trait with memory and tool access
   - DatabaseAgent with SurrealDB persistence
   - AgentType enum with feature-gated ADHD variants

2. **Memory System** (`memory.rs`)
   - MemGPT-style memory blocks with character limits
   - Arc<DashMap> for thread-safe concurrent access
   - Persistent memory between conversations

3. **Tool System** (`tool/`)
   - Type-safe `AiTool<Input, Output>` trait
   - Dynamic dispatch via `DynamicTool` trait
   - Thread-safe `ToolRegistry` using DashMap
   - Built-in tools (update_memory, send_message)

4. **Coordination** (`coordination/`)
   - Agent groups with dynamic/round-robin/sleeptime patterns
   - Selector registry (load balancing, random, capability-based)
   - Message routing and response aggregation

5. **Database** (`db/`)
   - SurrealKV embedded database with vector search
   - Entity system with `#[derive(Entity)]` macro
   - RELATE-based relationships (no foreign keys)

### Common Patterns

#### Creating a Tool
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

#### Entity Definition
```rust
#[derive(Debug, Clone, Entity, Serialize, Deserialize)]
#[entity(entity_type = "user")]
pub struct User {
    pub id: UserId,
    pub username: String,

    // Relations use RELATE, not foreign keys
    #[entity(relation = "owns")]
    pub owned_agent_ids: Vec<AgentId>,
}
```

#### Error Handling
```rust
// Use specific error variants with context
return Err(CoreError::tool_not_found(name, available_tools));
return Err(CoreError::memory_not_found(&agent_id, &block_name, available_blocks));
```

## Testing Guidelines
- Test actual behavior, not mocks
- Use MockModelProvider and MockEmbeddingProvider for testing
- Keep unit tests fast (<1 second), slow tests in integration
- All error paths must be tested with proper assertions

## Performance Notes
- CompactString inlines strings ≤ 24 bytes
- DashMap shards internally for concurrent access
- AgentHandle provides cheap cloning for built-in tools
- Database operations are non-blocking with optimistic updates

## Embedding Providers (`embeddings/`)
- **Candle (local)**: Pure Rust with Jina models (512/768 dims) - Use for offline/privacy
- **OpenAI**: text-embedding-3-small/large - Use for production quality
- **Cohere**: embed-english-v3.0 - Alternative cloud provider
- **Ollama**: Stub only - TODO: Complete implementation

**Known Issues**: BERT models fail in Candle (dtype errors), use Jina models instead.
