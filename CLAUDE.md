# CLAUDE.md - Pattern ADHD Cognitive Support System

Pattern is a multi-agent ADHD support system inspired by MemGPT's architecture to provide external executive function through specialized cognitive agents.

## Project Status

**Current Status**: Core foundation complete, ready for feature development

### 🚧 Current Development Priorities
1. **Agent Groups** - Make existing implementation usable via CLI and config
2. **Task Management System** - ADHD-aware task breakdown and tracking
3. **MCP Tools Integration** - Task-related tools and agent communication

## Agent Groups Implementation Plan

### Phase 1: Configuration Structure
Add group configuration to the existing config system:
- Add `GroupConfig` struct to `pattern_core/src/config.rs`
- Define `GroupMemberConfig` for agent membership with roles
- Integrate into main `PatternConfig` structure

### Phase 2: Database Operations
Implement CRUD operations in `pattern_core/src/db/ops.rs`:
- `create_group()` - Create a new agent group
- `get_group_by_name()` - Find group by name for a user
- `add_agent_to_group()` - Add an agent with a role
- `list_groups_for_user()` - List all groups owned by a user
- `get_group_members()` - Get all agents in a group

### Phase 3: CLI Commands
Add group management commands:
- `pattern-cli group list` - Show all groups
- `pattern-cli group create <name> --pattern <type>` - Create a group
- `pattern-cli group add-member <group> <agent> --role <role>` - Add agent to group
- `pattern-cli group status <name>` - Show group details
- `pattern-cli chat --group <name>` - Chat with a group

### Phase 4: ADHD-Specific Templates
Create predefined group configurations in `pattern_nd`:
- **Main Group**: Round-robin between executive function agents
- **Crisis Group**: Dynamic selection based on urgency
- **Planning Group**: Pipeline pattern for task breakdown
- **Memory Group**: Supervisor pattern for memory management

### Phase 5: Runtime Integration
Make groups work in chat:
- Group message routing through coordination patterns
- State persistence between messages
- Coordination pattern execution

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
│   ├── pattern_nd/       # Tools and agent personalities specific to the neurodivergent support constellation
│   ├── pattern_mcp/      # MCP server implementation
│   ├── pattern_discord/  # Discord bot integration
│   └── pattern_main/     # Main orchestrator binary (mostly legacy as of yet)
├── docs/                 # Architecture and integration guides
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
- **Built-in tools**: context, recall, search, send_message
- **Message persistence**: RELATE edges with Snowflake ID ordering
- **Memory system**: Thread-safe with semantic search, archival support, and atomic updates

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
- [X] Implement message compression with archival - COMPLETE
- [X] Add live query support for agent stats
- [X] Build agent groups framework
- [X] Create basic binary (CLI/TUI) for user testing - COMPLETE

### Medium Priority
- [ ] Make agent groups usable via CLI and config system
- [ ] Complete pattern-specific agent groups implementation (main, crisis, planning, memory)
- [ ] Implement task CRUD operations in pattern-core or pattern-nd
- [ ] Create ADHD-aware task manager with breakdown (pattern-nd)
- [ ] Add task-related MCP tools (create, update, list, breakdown)
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
