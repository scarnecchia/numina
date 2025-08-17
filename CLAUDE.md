# CLAUDE.md - Pattern ADHD Cognitive Support System

⚠️ **CRITICAL WARNING**: DO NOT run `pattern-cli` or any agent commands during development!
Agents are currently running in production. Any CLI invocation will disrupt active agents.
Migrations and testing must be done offline after stopping production agents.

Pattern is a multi-agent ADHD support system inspired by MemGPT's architecture to provide external executive function through specialized cognitive agents.

## Project Status

**Current State**: Core multi-agent framework operational, expanding integrations

### 🚧 Current Development Priorities

1. **Message Batching** - 🔴 IMMEDIATE - IN PROGRESS
   - Fix message ordering issues causing tool call/response mismatches
   - Add snowflake IDs for absolute ordering
   - Implement batch tracking for atomic request/response cycles
   - See `/home/booskie/pattern/docs/message-batching-design.md` for full design
   
   **Completed:**
   - ✅ Added `BatchType` enum (UserRequest, AgentToAgent, SystemTrigger, Continuation)
   - ✅ Created `SnowflakePosition` wrapper type with proper serde/FromStr support
   - ✅ Updated entity macro to handle SnowflakePosition<->String database conversions
   - ✅ Updated `get_next_message_position()` to return SnowflakePosition
   - ✅ Added batch fields to Message struct (position, batch, sequence_num, batch_type)
   - ✅ Added batch fields to AgentMessageRelation for query efficiency
   - ✅ Updated all Message and AgentMessageRelation constructors
   - ✅ Created migration function to populate snowflake_ids for existing messages
   - ✅ Ensured persist_agent_message syncs all batch fields to AgentMessageRelation
   
   **Implementation Details:**
   - SnowflakePosition is a newtype wrapper around SnowflakeMastodonId with `#[repr(transparent)]`
   - Implements Display using ferroid's base32 encoding for efficient string conversion
   - Implements FromStr using ferroid's decode() method from Base32SnowExt trait
   - Entity macro automatically handles conversion between SnowflakePosition and String for DB storage
   - Migration function (v2) generates snowflake IDs and detects batch boundaries based on:
     - User messages starting new conversation turns
     - Time gaps > 30 minutes between messages
     - Tool call/response patterns
   
   **Remaining Tasks:**
   - 🔲 Update default message constructors to generate snowflake_ids automatically
   - 🔲 Create batch-aware message constructors that accept batch_id/sequence_num
   - 🔲 Modify process_message_stream to propagate batch_id
   - 🔲 Update context builder to accept current_batch_id parameter
   - 🔲 Add batch_id to HeartbeatRequest for continuations
   - 🔲 Implement batch completeness detection in context builder
   - 🔲 Test with parallel tool execution scenarios

2. **Backend API Server** - 🟡 ACTIVE DEVELOPMENT
   - Basic Axum server structure exists
   - Auth handlers partially implemented
   - Most endpoints still need implementation
   - Required for multi-user hosting

3. **MCP Client Refinement** - 🟢 NEEDS VERIFICATION
   - All transports implemented (stdio, HTTP, SSE)
   - Tool discovery and wrapper system working
   - Needs testing with real MCP servers
   - Auth support may need improvements

4. **Task Management System** - 🟢 QUEUED
   - Database schema exists
   - Need CLI commands and user-facing features
   - ADHD-aware task breakdown planned

5. **MCP Server** - 🟢 LOWER PRIORITY
   - Stub implementation only
   - Expose Pattern tools to external clients

## Completed Features

### ✅ Agent Groups
- Full CLI support with create/add-member/status/list commands
- All coordination patterns working (RoundRobin, Dynamic, Pipeline, Supervisor, Voting, Sleeptime)
- Discord and Bluesky integration
- Runtime message routing through patterns
- More use cases and templates to be added

### ✅ Discord Integration
- Full message handling with batching and merging
- Typing indicators with auto-refresh
- Reaction handling on bot messages
- All slash commands implemented (/help, /status, /memory, /archival, /context, /search, /list)
- Group integration with coordination patterns
- Data source mode for event ingestion

### ✅ MCP Client
- All three transports implemented (stdio, HTTP, SSE)
- Tool discovery and dynamic wrapper system
- Integration with Pattern's tool registry
- Mock tools for testing when no server available
- Basic auth support (Bearer tokens)

### ✅ Bluesky/ATProto Integration
- Jetstream firehose consumer fully operational
- Thread context fetching with constellation API
- Memory block creation for users
- Rich text processing with mentions/links
- Reply handling and posting capabilities

### ✅ Data Source Framework
- Flexible trait supporting pull/push patterns
- File watching with indexing
- Discord message ingestion
- Coordinator managing multiple sources
- Prompt templates for notifications

### ✅ Model Configuration
- Comprehensive model registry with July 2025 specs
- Dynamic token calculation
- Smart caching with Anthropic optimization
- Message compression strategies

## Development Principles

- **Type Safety First**: Use Rust enums over string validation
- **Pure Rust**: Avoid C dependencies to reduce build complexity
- **Test-Driven**: All tests must validate actual behavior and be able to fail
- **Entity Relationships**: Use SurrealDB RELATE for all associations, no foreign keys
- **Atomic Operations**: Database operations are non-blocking with optimistic updates

## Workspace Structure

```
pattern/
├── crates/
│   ├── pattern_api/      # Shared API types and contracts
│   ├── pattern_cli/      # Command-line testing tool
│   ├── pattern_core/     # Agent framework, memory, tools, coordination
│   ├── pattern_discord/  # Discord bot integration
│   ├── pattern_mcp/      # MCP client (working) and server (stub)
│   ├── pattern_nd/       # ADHD-specific tools and agent personalities
│   ├── pattern_server/   # Backend API server (in development)
├── docs/                 # Architecture and integration guides
```

**Each crate has its own `CLAUDE.md` with specific implementation guidelines.**

## Core Architecture

### Agent Framework
- **DatabaseAgent**: Generic over ModelProvider and EmbeddingProvider
- **Built-in tools**: context, recall, search, send_message
- **Message persistence**: RELATE edges with Snowflake ID ordering
- **Memory system**: Thread-safe with semantic search, archival support, and atomic updates

### Coordination Patterns
- **Dynamic**: Selector-based routing (random, capability, load-balancing)
- **Round-robin**: Fair distribution with skip-inactive support
- **Sleeptime**: Background monitoring with intervention triggers
- **Pipeline**: Sequential processing through agent stages
- **Supervisor**: Hierarchical delegation
- **Voting**: Consensus-based decisions

### Entity System
Uses `#[derive(Entity)]` macro for SurrealDB integration:

```rust
#[derive(Entity)]
#[entity(entity_type = "user")]
pub struct User {
    pub id: UserId,
    pub username: String,

    // Relations via RELATE, not foreign keys
    #[entity(relation = "owns")]
    pub owned_agents: Vec<Agent>,
}
```

## Known Issues

### Message Ordering Issues
- **Tool call/response pairing**: Messages can appear out of order due to async processing
- **Continuation timing**: Heartbeat messages sometimes created before tool responses
- **Backdated messages**: Sleeptime interventions with past timestamps disrupt ordering
- **Temporary fix**: Sorting by created_at, waiting for Ready state in heartbeat
- **Permanent fix**: Message batching implementation in progress

### API Provider Issues
- **Anthropic Thinking Mode**: Message compression can create invalid sequences with tool calls
- **Gemini Response Structure**: Missing `/candidates/0/content/parts` path during heartbeat continuations
- **Gemini Empty Contents**: "contents is not specified" error when all messages filtered out
- **Tool call validation**: Compression sometimes leaves unpaired tool calls (affects Flux agent)
- See `docs/known-api-issues.md` for workarounds

### Export Issues
- **CAR Export**: Not archiving full message history - pattern matching issues preventing complete export
  - Related to unused `CompressionSettings` struct in `pattern_core/src/export/types.rs`
  - Lower priority but needs fixing for proper data portability

## Implementation Notes

### 🔧 Message Batching Implementation
**Branch**: Currently on feature branch for batching work
**Status**: Core data structures complete, logic implementation pending

Key decisions made:
- Using String for snowflake_id/batch_id storage (serde compatibility)
- Batch fields are Option<T> during migration, will be required after
- AgentMessageRelation duplicates ALL batch fields from Message for query efficiency
- No separate batch table - batches reconstructed at runtime from messages
- Position field in agent_messages will sync with message.snowflake_id
- persist_agent_message must sync batch_id, sequence_num, batch_type to relation
- Default message constructors should auto-generate snowflake_ids
- Need batch-aware constructors for messages within a processing cycle

### 🔧 Memory Block Pass-through
Data sources can attach memory blocks to messages for agent context:
- DataSource trait returns memory blocks with notifications
- Coordinator includes blocks in message metadata
- Bluesky creates/retrieves user profile blocks automatically
- Router needs to create RELATE edges for block attachment (TODO)

### 🔧 Anti-looping Protection
- Router returns errors instead of silently dropping messages
- 30-second cooldown between rapid agent-to-agent messages
- Prevents acknowledgment loops

### 🔧 Constellation Integration
- Thread siblings fetched from constellation.microcosm.blue
- Engagement metrics and agent interaction tracking
- Smart filtering based on agent DID and friend lists
- Rich thread context display with [YOU] markers

## Feature Development Workflow

1. **Branch Creation**: `git checkout -b feature/task-management`
2. **Implementation**: Follow crate-specific CLAUDE.md guidelines
3. **Testing**: Add tests that validate actual behavior
4. **Validation**: Run `just pre-commit-all` before commit
5. **PR**: Create pull request with clear description

## Build Commands

```bash
# Quick validation
cargo check
cargo test --lib

# Full pipeline (required before commit)
just pre-commit-all

# Development helpers
just watch                    # Auto-recompile on changes
cargo test --lib -- db::     # Run specific module tests
```

## Architecture Notes

### Partner-Centric Model
- **Partner**: Person receiving ADHD support (owns constellation)
- **Conversant**: Someone interacting through partner's agents
- **Privacy**: DM content never bleeds into public channels
- **Scaling**: Each partner gets full constellation, hibernated when inactive

### Backend API Server (In Development)
- Basic Axum server structure exists in `pattern_server`
- Handlers need implementation
- Required for multi-user hosting and non-technical users
- Will provide HTTP/WebSocket APIs, MCP integration, Discord bot hosting

## References

- [MCP Rust SDK](https://github.com/modelcontextprotocol/rust-sdk)
- [MemGPT Paper](https://arxiv.org/abs/2310.08560) - Stateful agent architecture
- [SurrealDB Documentation](https://surrealdb.com/docs) - Graph database patterns
