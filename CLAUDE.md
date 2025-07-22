# CLAUDE.md - Pattern ADHD Cognitive Support System

Pattern is a multi-agent ADHD support system inspired by MemGPT's architecture to provide external executive function through specialized cognitive agents.

## Project Status

**Current Status**: Core foundation complete, ready for feature development

### ✅ Completed Infrastructure
- **Multi-crate workspace**: pattern-core, pattern-nd, pattern-mcp, pattern-discord, pattern-main
- **Entity system**: Macro-based with SurrealDB graph relationships
- **Agent framework**: Generic DatabaseAgent with tool system and memory management
- **Message system**: Full persistence with edge relationships and compression
- **Agent coordination**: Dynamic, round-robin, sleeptime patterns implemented
- **Database**: SurrealDB with vector search, migrations, and embedding support
- **Testing**: 92 tests passing, comprehensive test coverage

### 🚧 Current Development Priorities
1. **Task Management System** - ADHD-aware task breakdown and tracking
2. **MCP Tools Integration** - Task-related tools and agent communication
3. **Agent Groups** - Main, crisis, planning, memory group implementations
4. **Message Compression** - Context window management with archival

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
│   ├── pattern_core/     # Agent framework, memory, tools, coordination
│   ├── pattern_nd/       # ADHD-specific tools and agent personalities
│   ├── pattern_mcp/      # MCP server implementation
│   ├── pattern_discord/  # Discord bot integration
│   └── pattern_main/     # Main orchestrator binary
├── docs/                 # Architecture and integration guides
└── migrations/           # Database schema migrations
```

**Each crate has its own `CLAUDE.md` with specific implementation guidelines.**

## Quick Start

```bash
# Development
cargo check                    # Validate compilation
cargo test --lib              # Run all tests
just pre-commit-all           # Full validation pipeline

# Running
cargo run --features full    # Discord + MCP + background monitoring
```

## Core Architecture

### Agent Framework
- **DatabaseAgent**: Generic over ModelProvider and EmbeddingProvider
- **Built-in tools**: update_memory, send_message with type-safe registry
- **Message persistence**: RELATE edges with Snowflake ID ordering
- **Memory system**: Thread-safe with semantic search capabilities

### Coordination Patterns
- **Dynamic**: Selector-based routing (random, capability, load-balancing)
- **Round-robin**: Fair distribution with skip-inactive support
- **Sleeptime**: Background monitoring with intervention triggers
- **Pipeline**: Sequential processing through agent stages

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

## Feature Development Workflow

1. **Branch Creation**: `git checkout -b feature/task-management`
2. **Implementation**: Follow crate-specific CLAUDE.md guidelines
3. **Testing**: Add tests that validate actual behavior
4. **Validation**: Run `just pre-commit-all` before commit
5. **PR**: Create pull request with clear description

## Current TODO List

### High Priority
- [X] Implement message compression with archival
- [X] Add live query support for agent stats
- [X] Built agent groups framework
- [ ] Create basic binary (CLI/TUI) for user testing

### Medium Priority
- [ ] Implement task CRUD operations in pattern-core or pattern-nd
- [ ] Create ADHD-aware task manager with breakdown (pattern-nd)
- [ ] Add task-related MCP tools (create, update, list, breakdown)
- [ ] Complete more pattern-specific agent groups implementation (main, crisis, planning, memory)
- [ ] Add Discord context tools to MCP
- [ ] Implement time tracking with ADHD multipliers
- [ ] Add energy/attention monitoring

### Documentation
Each major component has dedicated docs in `docs/`:
- **Architecture**: System design and component interactions
- **Guides**: Integration and setup instructions
- **API**: Common patterns and gotchas
- **Troubleshooting**: Known issues and solutions

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

## Partner-Centric Architecture

Pattern uses a partner-centric model ensuring privacy:
- **Partner**: Person receiving ADHD support (owns constellation)
- **Conversant**: Someone interacting through partner's agents
- **Privacy**: DM content never bleeds into public channels
- **Scaling**: Each partner gets full constellation, hibernated when inactive

## References

- [MCP Rust SDK](https://github.com/modelcontextprotocol/rust-sdk)
- [MemGPT Paper](https://arxiv.org/abs/2310.08560) - Stateful agent architecture
- [SurrealDB Documentation](https://surrealdb.com/docs) - Graph database patterns
