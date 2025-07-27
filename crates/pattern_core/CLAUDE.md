# CLAUDE.md - Pattern Core

Core agent framework, memory management, and coordination system for Pattern's multi-agent ADHD support. Inspired by MemGPT architecture for stateful agents.

## Current Status (2025-07-24)

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

### Agent Groups ✅ COMPLETE (2025-07-24) - NEEDS USER TESTING
- **Configuration**: `GroupConfig` and `GroupMemberConfig` in config system
- **Database Operations**: Full CRUD operations with constellation relationships
- **Coordination Patterns**: All 6 patterns working (Dynamic, RoundRobin, Pipeline, Supervisor, Voting, Sleeptime)
- **CLI Integration**: group list/create/add-member/status commands
- **Group Chat**: `pattern-cli chat --group <name>` routes through coordination patterns
- **Type-erased Agents**: Groups work with `Arc<dyn Agent>` for flexibility and shared ownership
- **Trait Refactoring**: `GroupManager` trait made dyn-compatible with `async_trait`
- **Testing Status**: Basic operations work, but ~2 group-related tests failing after refactor
- **⚠️ NEEDS USER TESTING**: Overall integrity and edge cases need validation
- **Design Note**: Using `Arc<dyn Agent>` is intentional - agents can belong to multiple groups (overlap allowed)

### Tool System Refactoring ✅ COMPLETE
- All core tools refactored to follow Letta/MemGPT patterns
- Domain-based multi-operation tools implemented
- Unified search tool consolidates all search operations
- Context builder includes archival memory labels
- Full test coverage with realistic scenarios

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

4. **Concurrent Memory Access**:
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

## Current Limitations

1. **Transaction Retry Logic**: Database operations that were previously spawned as background tasks have been made synchronous to avoid transaction conflicts. When running multiple operations in parallel, proper retry logic for transaction conflicts will be needed.

2. **Redundant Arc Wrapping**: DatabaseAgent contains internal Arc fields, so wrapping it in Arc<dyn Agent> is redundant. Consider implementing Clone for DatabaseAgent to allow direct ownership while maintaining shared internal state.

## Model Configuration ✅ COMPLETE (2025-07-24)

### What We Fixed
1. **Created `model/defaults.rs`** with comprehensive registry:
   - Accurate July 2025 model specifications
   - Context windows: Anthropic (200k), Gemini (1M/2M), OpenAI (128k-1M)
   - Max output tokens: Model-specific limits (4k-100k)
   - Pricing information for all major models
   - Embedding model defaults (dimensions, max tokens, pricing)

2. **Smart Token Management**:
   - `enhance_model_info()` fixes provider-supplied ModelInfo
   - `calculate_max_tokens()` respects model-specific limits
   - Dynamic calculation: user config > model max > context/4
   - Case-insensitive matching with fuzzy fallback

3. **Context Compression** ✅:
   - Integrated `MessageCompressor` with multiple strategies
   - `CompressionStrategy`: Truncate, RecursiveSummarization, ImportanceBased, TimeDecay
   - Made `ContextBuilder::build()` async for proper compression
   - Maintains Gemini API compatibility (valid message sequences)

4. **Smart Caching** ✅:
   - Added `CacheControl::Ephemeral` to optimize Anthropic token usage
   - Applied to first message and every 20th message
   - Works with all compression strategies

## Message System Implementation Status

### ✅ Completed Components

#### 1. **Heartbeat System** ✅ COMPLETE (2025-07-24)
- Added `request_heartbeat: bool` field to ALL tool input structs (context, recall, search, send_message)
- Agent checks for heartbeat requests in `process_message()` before returning
- CLI creates heartbeat monitor task that triggers new agent turns
- Works in practice - tested and functional!

#### 2. **Message Queue Entities** ✅ COMPLETE
- Created `QueuedMessage` entity with:
  - Separate `QueuedMessageId` type for the queue_msg table
  - Support for agent-to-agent and user-to-agent messages
  - Call chain for loop prevention
  - Read/unread tracking with timestamps
- Created `ScheduledWakeup` entity with:
  - One-time and recurring wakeup support
  - Active/inactive state management
  - Metadata field for extensibility

#### 3. **AgentMessageRouter** ✅ COMPLETE
- Core router implementation with:
  - Database-backed message queuing
  - Endpoint registry for extensible delivery
  - Support for User, Agent, Group, and Channel targets
  - Loop prevention via call chain checking
- `MessageEndpoint` trait for custom endpoints
- `CliEndpoint` implementation for stdout
- Added to `AgentHandle` as optional field

### 🚧 Next Steps

#### IMMEDIATE TODO: Switch to rustyline-async
**Problem**: Current readline implementation blocks the main thread, causing issues with:
- Heartbeat responses appearing at wrong times
- Messages from other agents interrupting the prompt
- Poor user experience when async events occur

**Solution**: Switch from `rustyline` to `rustyline-async` (https://github.com/zyansheep/rustyline-async)

**Changes needed**:
1. Update `pattern_cli/Cargo.toml`:
   - Replace `rustyline = "14.0"` with `rustyline-async = "0.4"`
   
2. Refactor `chat_with_agent()` in `agent_ops.rs`:
   - Use `rustyline_async::Readline` instead of `DefaultEditor`
   - Properly handle concurrent input/output with tokio::select!
   - Allow heartbeat responses and incoming messages to display without breaking the input line
   
   Example pattern:
   ```rust
   let mut rl = Readline::new("> ".to_string()).unwrap();
   
   loop {
       tokio::select! {
           // Handle user input
           line = rl.readline() => {
               match line {
                   Ok(line) => { /* process user input */ }
                   Err(_) => break,
               }
           }
           
           // Handle heartbeat responses
           Some(response) = response_receiver.recv() => {
               // Save current input
               let saved = rl.line();
               
               // Clear line and display response
               print!("\r\x1b[K"); // Clear current line
               display_response(response);
               
               // Restore prompt and input
               print!("{}{}", prompt, saved);
               io::stdout().flush()?;
           }
           
           // Handle incoming messages from other agents
           // (would need another channel for this)
       }
   }
   ```
   
3. Benefits:
   - Clean handling of async events (heartbeats, agent messages)
   - No more prompt corruption
   - Better user experience for multi-agent conversations

#### Phase 1: Wire Router to Tools (HIGH PRIORITY) ✅ COMPLETE
1. **Update SendMessageTool** to use AgentMessageRouter ✅
   - Get router from AgentHandle
   - Convert tool input to router method call
   - Handle missing router gracefully

2. **Create router during agent initialization** ✅
   - Modify DatabaseAgent to create router with DB connection
   - Pass appropriate endpoints (CLI for now)
   - Update AgentHandle during construction

#### Phase 2: Live Query Message Delivery ✅ COMPLETE
1. **Add incoming message monitoring to DatabaseAgent** ✅
   - Added `subscribe_to_agent_messages()` in db/ops.rs
   - Set up live query: `LIVE SELECT * FROM queue_msg WHERE to_agent = agent_id AND read = false`
   - Background task monitors incoming messages

2. **Message processing flow** ✅
   - Convert QueuedMessage to Message with proper role and metadata
   - Use weak reference to agent for processing in background task
   - Mark messages as read immediately to prevent reprocessing
   - Call process_message() directly on incoming messages

#### Phase 3: Scheduled Wakeups (MEDIUM PRIORITY)
1. **Background scheduler service**
   - Query for due wakeups periodically
   - Send system messages to trigger agents
   - Update recurring wakeups for next execution
   - Handle missed wakeups gracefully

2. **Integration with agents**
   - Add "schedule_wakeup" tool for agents to self-schedule
   - Include wakeup reason in trigger message
   - Allow cancellation of scheduled wakeups

#### Phase 3: Test CLI Endpoint ✅ READY FOR TESTING
- Added slash commands to chat loop:
  - `/send <agent> <message>` - Send message to another agent
  - `/list` - List all agents
  - `/status` - Show current agent status
  - `/help` - Show available commands
  - `/exit` or `/quit` - Exit chat
- CLI endpoint prints incoming messages to stdout
- Need to test with multiple agents running simultaneously

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
