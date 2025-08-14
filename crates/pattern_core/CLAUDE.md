# CLAUDE.md - Pattern Core

Core agent framework, memory management, and coordination system for Pattern's multi-agent ADHD support.

## Current Status

### ✅ Complete Features
- **Agent Groups**: Full database operations, CLI integration, all coordination patterns working
- **Data Sources**: Generic abstraction with file/Discord/Bluesky implementations
- **Memory System**: MemGPT-style blocks with archival, semantic search, thread-safe access
- **Tool System**: Multi-operation tools (context, recall, search) with automatic rule bundling
- **Message Router**: Agent-to-agent messaging with anti-looping protection
- **Model Configuration**: Comprehensive registry with provider-specific optimizations

### 🚧 In Progress
- **Memory Block Pass-through**: Router needs to create RELATE edges for attached blocks
- **MCP Client Integration**: Consume external MCP tools (high priority)

## Critical Implementation Notes

### Entity System Sacred Patterns ⚠️

**Edge Entity Pattern - DO NOT CHANGE**:
The macro looks redundant but it's required. Users should ONLY specify `edge_entity`:

```rust
// Correct usage:
#[entity(edge_entity = "agent_memories")]
pub memories: Vec<(TestMemory, AgentMemoryRelation)>,

// WRONG - don't specify both:
// #[entity(relation = "agent_memories", edge_entity = "AgentMemoryRelation")]
```

### Database Patterns (CRITICAL)

1. **SurrealDB Response Handling**:
   - NEVER try to get `Vec<serde_json::Value>` from a SurrealDB response
   - ALWAYS print raw response before `.take()` when debugging
   - Use `unwrap_surreal_value` helper for nested value extraction

2. **Parameterized Queries**:
   - ALWAYS use parameter binding to prevent SQL injection
   - Example: `query("SELECT * FROM user WHERE id = $id").bind(("id", user_id))`

3. **Concurrent Memory Access**:
   - Use `alter_block` for atomic updates
   - Never hold refs across async boundaries
   - Extract data and drop locks immediately

## Tool System Architecture

Following Letta/MemGPT patterns with multi-operation tools:

### Core Tools
1. **context** - Operations on context blocks
   - `append`, `replace`, `archive`, `load_from_archival`, `swap`

2. **recall** - Long-term storage operations
   - `insert`, `append`, `read`, `delete`
   - Full-text search with SurrealDB's BM25 analyzer

3. **search** - Unified search across domains
   - Supports archival_memory, conversations, all
   - Domain-specific filters and limits

4. **send_message** - Agent communication
   - Routes through AgentMessageRouter
   - Supports CLI, Group, Discord, Queue endpoints

### Implementation Notes
- Each tool has single entry point with operation enum
- Tool usage rules bundled with tools via `usage_rule()` trait
- ToolRegistry automatically provides rules to context builder
- Archival labels included in context for intelligent memory management

## Message System Architecture

### Router Design
- Each agent has its own router (not singleton)
- Database queuing provides natural buffering
- Call chain prevents infinite loops
- Anti-looping: 30-second cooldown between rapid messages

### Endpoints
- **CliEndpoint**: Terminal output ✅
- **GroupEndpoint**: Coordination pattern routing ✅
- **DiscordEndpoint**: Discord integration ✅
- **QueueEndpoint**: Database persistence (stub)
- **BlueskyEndpoint**: ATProto posting ✅

## Architecture Overview

### Key Components

1. **Agent System** (`agent/`, `context/`)
   - Base `Agent` trait with memory and tool access
   - DatabaseAgent with SurrealDB persistence
   - AgentType enum with feature-gated ADHD variants

2. **Memory System** (`memory.rs`)
   - Arc<DashMap> for thread-safe concurrent access
   - Character-limited blocks with overflow handling
   - Persistent between conversations

3. **Tool System** (`tool/`)
   - Type-safe `AiTool<Input, Output>` trait
   - Dynamic dispatch via `DynamicTool`
   - Thread-safe `ToolRegistry` using DashMap

4. **Coordination** (`coordination/`)
   - All patterns implemented and working
   - Type-erased `Arc<dyn Agent>` for group flexibility
   - Message routing and response aggregation

5. **Database** (`db/`)
   - SurrealKV embedded database
   - Entity system with `#[derive(Entity)]` macro
   - RELATE-based relationships (no foreign keys)

6. **Data Sources** (`data_source/`)
   - Generic trait for pull/push consumption
   - Type-erased wrapper for concrete→generic bridging
   - Prompt templates using minijinja

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
return Err(CoreError::memory_not_found(&agent_id, &block_name, available_blocks));
```

## Performance Notes
- CompactString inlines strings ≤ 24 bytes
- DashMap shards internally for concurrent access
- AgentHandle provides cheap cloning for built-in tools
- Database operations are non-blocking with optimistic updates

## Embedding Providers
- **Candle (local)**: Pure Rust with Jina models (512/768 dims)
- **OpenAI**: text-embedding-3-small/large
- **Cohere**: embed-english-v3.0
- **Ollama**: Stub only - TODO

**Known Issues**: BERT models fail in Candle (dtype errors), use Jina models instead.