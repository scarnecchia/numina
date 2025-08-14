# CLAUDE.md - Pattern ADHD Cognitive Support System

Pattern is a multi-agent ADHD support system inspired by MemGPT's architecture to provide external executive function through specialized cognitive agents.

## Project Status

**Current State**: Core multi-agent framework operational, expanding integrations

### 🚧 Current Development Priorities

1. **MCP Client Integration** - 🔴 HIGH PRIORITY
   - Integrate MCP client to consume external tools
   - Allow agents to use MCP-provided capabilities
   - Tool discovery and registration
   
2. **Bug Fixes** - 🔴 IMMEDIATE
   - Address any critical issues blocking usage
   - See current issue tracker

3. **Backend API Server** - 🟡 ACTIVE DEVELOPMENT
   - Basic Axum server structure exists
   - Need to implement actual handlers
   - Required for multi-user hosting
   - Enable non-technical user access

4. **Discord Slash Commands** - 🟡 IN PROGRESS  
   - Core bot functionality working
   - Missing slash command implementations
   - Message handling and coordination complete

5. **Task Management System** - 🟢 QUEUED
   - Database schema exists
   - Need CLI commands and user-facing features
   - ADHD-aware task breakdown planned

## Completed Features

### ✅ Agent Groups 
- Full CLI support with create/add-member/status/list commands
- All coordination patterns working (RoundRobin, Dynamic, Pipeline, Supervisor, Voting, Sleeptime)
- Discord and Bluesky integration
- Runtime message routing through patterns
- More use cases and templates to be added

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
│   ├── pattern_cli/      # Command-line testing tool
│   ├── pattern_core/     # Agent framework, memory, tools, coordination
│   ├── pattern_nd/       # ADHD-specific tools and agent personalities
│   ├── pattern_mcp/      # MCP server implementation (stub)
│   ├── pattern_discord/  # Discord bot integration
│   ├── pattern_server/   # Backend API server (in development)
│   └── pattern_main/     # Main orchestrator binary
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