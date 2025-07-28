# CLAUDE.md - Pattern Core

Core agent framework, memory management, and coordination system for Pattern's multi-agent ADHD support. Inspired by MemGPT architecture for stateful agents.

## Current Status (2025-07-28)

### Data Sources ✅ COMPLETE (2025-07-28)
- **DataSource Trait**: Generic abstraction supporting pull/push patterns with cursor management
- **FileDataSource**: Concrete implementation with ephemeral and indexed modes
- **DataIngestionCoordinator**: Manages sources and routes data to agents via templates
- **Prompt Templates**: Jinja2-style templates using minijinja for agent notifications
- **DataSourceTool**: Agent-accessible operations for data source management
- **Embedding Integration**: Reuses agent's embedding provider for indexed sources
- **Type Erasure**: Maintains concrete types while providing Value-based interface
- **See**: `docs/data-sources.md` for detailed implementation guide

### Agent Groups ✅ COMPLETE (2025-07-24) - NEEDS USER TESTING
- **Configuration**: `GroupConfig` and `GroupMemberConfig` in config system
- **Database Operations**: Full CRUD operations with constellation relationships
- **Coordination Patterns**: All 6 patterns working (Dynamic, RoundRobin, Pipeline, Supervisor, Voting, Sleeptime)
- **CLI Integration**: group list/create/add-member/status commands
- **Group Chat**: `pattern-cli chat --group <name>` routes through coordination patterns
- **Type-erased Agents**: Groups work with `Arc<dyn Agent>` for flexibility and shared ownership
- **Trait Refactoring**: `GroupManager` trait made dyn-compatible with `async_trait`
- **⚠️ NEEDS USER TESTING**: Overall integrity and edge cases need validation
- **Design Note**: Using `Arc<dyn Agent>` is intentional - agents can belong to multiple groups (overlap allowed)

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

2. **Parameterized Queries**:
   - **ALWAYS** use parameter binding to prevent SQL injection
   - Example: `query("SELECT * FROM user WHERE id = $id").bind(("id", user_id))`

### Technical Decisions

1. **Snowflake IDs for Message Ordering**:
   - Using ferroid crate for distributed monotonic IDs
   - Stored as String in `position` field
   - Guarantees order even with rapid message creation

2. **Concurrent Memory Access**:
   - Use `alter_block` for atomic updates to avoid deadlocks
   - Never hold refs across async boundaries
   - Extract data and drop locks immediately

## Tool System ✅ COMPLETE

Following Letta/MemGPT patterns, all tools have been refactored into domain-based multi-operation tools:

### Completed Tools
1. **context** ✅ - Operations on context blocks
   - `append` - Add content to existing memory (always uses \n separator)
   - `replace` - Replace specific content within memory
   - `archive` - Move a core memory block to archival storage
   - `load_from_archival` - Load an archival memory block into core
   - `swap` - Atomic operation to archive one block and load another

2. **recall** ✅ - Long-term storage operations
   - `insert` - Add new memories to archival storage
   - `append` - Add content to existing archival memory (always uses \n separator)
   - `read` - Read specific archival memory by label
   - `delete` - Remove archived memories

   **Implementation Notes**:
   - Archival memories are stored as regular MemoryBlocks with `MemoryType::Archival`
   - AgentHandle now has private DB connection with controlled access methods
   - DB methods: `search_archival_memories`, `insert_archival_memory`, `delete_archival_memory`, `count_archival_memories`
   - Graceful fallback: tries DB first, falls back to in-memory if unavailable
   - Full-text search working with SurrealDB's @@ operator and BM25 analyzer

3. **search** ✅ - Unified search across domains
   - `domain` - Where to search: "archival_memory", "conversations", "all"
   - `query` - Search text
   - `limit` - Maximum results
   - Domain-specific filters (role for conversations, time ranges, etc.)

   **Implementation Notes**:
   - Single interface for all search operations
   - Extensible to add new domains (files, tasks, etc.)
   - Uses appropriate indexes/methods per domain

### Remaining core tools

5. **interact_with_files** - File system operations
   - `read` - Read file contents
   - `search` - Search within files
   - `edit` (optional) - Modify files

### Implementation Notes
- Each tool has a single entry point with operation selection
- Tool usage rules are bundled with tools (via `usage_rule()` trait method)
- ToolRegistry automatically provides rules to context builder
- Operations use enums for type safety

### Memory Context Management
The context builder includes archival memory labels to enable intelligent memory management:
- All archival labels included if under 50 entries
- Above 50, shows most recently accessed + most frequently accessed
- Labels grouped by prefix for organization (e.g., all "meeting_notes_*" together)
- Format: `label: description` to provide semantic hints

This allows agents to make informed decisions about memory swapping without needing to search first.

## Message System Implementation TODOS

#### Phase 4: Additional Endpoints (MEDIUM PRIORITY)
**STATUS: Only CliEndpoint implemented so far**

1. **GroupEndpoint** (TODO)
   - Query group members from database
   - Apply coordination pattern for routing
   - Handle offline members appropriately

2. **DiscordEndpoint** (TODO - when Discord integration ready)
   - Use Discord bot to send messages
   - Map channel IDs appropriately
   - Handle rate limits and errors

3. **QueueEndpoint** (Stub exists but not functional)
   - Currently has unused fields warning
   - Needs proper implementation for database queuing

### 🎯 Implementation Order
1. ✅ Wire router to send_message tool (COMPLETE)
2. ✅ Set up live queries (COMPLETE)
3. ✅ Add slash commands for testing (COMPLETE)
4. 🧪 TEST: Agent-to-agent messaging with CLI endpoint
   - Run two agents in separate terminals
   - Use `/send` command to send messages between them
   - Verify messages are received and processed
5. Add scheduled wakeups (enables time-based features)
6. Implement additional endpoints as needed

### 💡 Design Decisions
- Each agent has its own router (not a singleton) for flexibility
- Database queuing provides natural buffering for offline agents
- Call chain prevents infinite loops while allowing controlled recursion
- Heartbeats and scheduled wakeups are separate systems with different use cases

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

6. **Data Sources** (`data_source/`)
   - Generic `DataSource` trait for pull/push data consumption
   - `FileDataSource` with watch and indexing support
   - `DataIngestionCoordinator` for multi-source management
   - Prompt templates for agent notifications
   - Type-erased wrapper for concrete→generic bridging

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
