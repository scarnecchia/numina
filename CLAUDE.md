# CLAUDE.md - Pattern ADHD Cognitive Support System

Pattern is a multi-agent cognitive support system designed specifically for ADHD brains. It uses Letta's multi-agent architecture with shared memory to provide external executive function through specialized cognitive agents inspired by Brandon Sanderson's Stormlight Archive.

## TODO Management

**IMPORTANT**: Always keep TODOs synchronized between:
1. The TodoWrite/TodoRead tools during development
2. This CLAUDE.md file for persistence across sessions

When starting work, check the TODO list below and load it into TodoWrite.
When finishing work, update this list with any changes.

## Git Workflow - Feature Branches

**IMPORTANT**: Once the project is stable, we use feature branches for all development:

1. **Before starting any work**, create a feature branch:
   ```bash
   git checkout -b feature/descriptive-name
   # Examples: feature/add-default-impls, fix/batch-api-errors, docs/improve-examples
   ```

2. **Commit regularly** as you work:
   - After each logical change or set of related edits
   - Use clear, descriptive commit messages
   - Example: `git commit -m "Add Default impl for UpdateMemoryBlockRequest"`

3. **When feature is complete**, create a pull request to main
   - This keeps main stable and CI runs only on complete changes
   - Allows for code review and discussion

4. **Branch naming conventions**:
   - `feature/` - New features or enhancements
   - `fix/` - Bug fixes
   - `docs/` - Documentation improvements
   - `refactor/` - Code refactoring
   - `test/` - Test additions or improvements

## Development Principles

- **ALWAYS check if files/scripts/functions exist before creating new ones** - Use `ls`, `find`, `grep`, or read existing code first
- Run `cargo check` frequently when producing code. This will help you catch errors early.
- NEVER use `unsafe{}`. If you feel you need to, stop, think about other ways, and ask the user for help if needed.
- NEVER ignore a failing test or change a test to make your code pass
- NEVER ignore a test
- ALWAYS fix compile errors before moving on.
- **ALWAYS ENSURE that tests will fail (via assert or panic with descriptive message) on any error condition**
- Use the web or context7 to help find docs, in addition to any other reference material

## MCP Tools & Documentation

See [MCP Integration Guide](./docs/guides/MCP_INTEGRATION.md) for detailed information about available MCP tools and workflows.

## Testing Strategy

All tests should validate actual behavior and be able to fail:
- **Unit tests**: Test individual functions with edge cases
- **Integration tests**: Test module interactions
- **Database tests**: Use in-memory SQLite for speed
- **No mock-heavy tests**: Prefer testing real behavior
- **Meaningful assertions**: Tests should catch actual bugs

Run tests with:
```bash
cargo test --lib           # Run all library tests
cargo test --lib -- db::   # Run specific module tests
just pre-commit-all        # Run all checks before committing
```

## Build & Validation Commands

**Required validation before any commit:**

```bash
# 1. Check compilation
cargo check

# 2. Build the project
cargo build

# 3. Run tests
cargo test

# 4. Check formatting
cargo fmt -- --check

# 5. Run clippy lints
cargo clippy -- -D warnings

# 6. Run pre-commit hooks (includes formatting)
just pre-commit-all
```

**Quick commands via justfile:**
- `just` - List all available commands
- `just run` - Run the project
- `just watch` - Auto-recompile on changes (using bacon)
- `just pre-commit-all` - Run all pre-commit hooks

**Development workflow:**
1. Make changes
2. Run `cargo check` frequently during development
3. Run `cargo test` to ensure tests pass
4. Run `just pre-commit-all` before committing
5. All checks must pass before creating PR

**Language Server Integration:**
- This project uses the `mcp__language-server` MCP server for enhanced code intelligence
- The language server provides real-time diagnostics, completions, and code navigation
- Errors and warnings from rust-analyzer will be surfaced automatically
- Use language server diagnostics to catch issues before running `cargo check`


## Project Documentation

### Architecture & Design
- [Pattern ADHD Architecture](./docs/architecture/PATTERN_ADHD_ARCHITECTURE.md) - Multi-agent cognitive support system design
- [Agent Routing](./docs/architecture/AGENT-ROUTING.md) - How messages are routed to agents
- [System Prompts](./docs/architecture/pattern-system-prompts.md) - Agent personality and behavior

### Integration Guides
- [MCP Integration](./docs/guides/MCP_INTEGRATION.md) - MCP tools and workflows
- [Letta Integration](./docs/guides/LETTA_INTEGRATION.md) - Multi-agent implementation with Letta
- [Discord Setup](./docs/guides/DISCORD_SETUP.md) - Discord bot configuration

### API References
- [Letta API Reference](./docs/api/LETTA_API_REFERENCE.md) - Common patterns and gotchas

### Troubleshooting
- [Discord Issues](./docs/troubleshooting/DISCORD_ISSUES.md) - Known Discord integration issues
- [MCP HTTP Setup](./docs/guides/MCP_HTTP_SETUP.md) - MCP transport configuration

## Quick Overview

Pattern is a multi-agent ADHD cognitive support system using Letta. See documentation above for details.




## Build Priority Breakdown

### Phase 1: Core Foundation (Must Have First)
1. **Multi-Agent Architecture Design**
   - Design Pattern orchestrator + 5 specialist agents
   - Define shared memory schema
   - Plan agent communication patterns
   - **Why first**: Architecture drives everything else

2. **Shared Memory Implementation**
   - Implement shared memory blocks in database
   - Create memory sync mechanism
   - Build memory access layer
   - **Why early**: Agents need shared state to coordinate

3. **Letta Multi-Agent Integration**
   - Extend AgentManager for multi-agent support
   - Implement agent creation with shared memory
   - Build inter-agent messaging
   - **Why early**: Core functionality depends on agent coordination

### Phase 2: Core Features (MVP)
4. **Shared Agent Tools**
   - Implement check_vibe, context_snapshot, find_pattern, suggest_pivot
   - Create tool registry for agent access
   - **Why**: Agents need common capabilities

5. **Pattern Sleeptime Agent**
   - Implement 20-30min background checks
   - Hyperfocus detection, physical needs monitoring
   - **Why**: Core ADHD support mechanism

6. **Entropy Agent (Tasks)**
   - Task breakdown into atoms
   - Hidden complexity detection
   - **Why**: Task paralysis is core ADHD challenge

### Phase 3: Specialist Agents
7. **Flux Agent (Time)**
   - ADHD time translation (5min = 30min)
   - Auto-buffering with multipliers
   - Time blindness compensation

8. **Momentum Agent (Energy)**
   - Energy state tracking
   - Flow vs burnout detection
   - Task/energy alignment

9. **Archive Agent (Memory)**
   - External memory bank
   - Context recovery ("what was I doing?")
   - Pattern detection across thoughts

10. **Anchor Agent (Habits)**
    - Basic needs tracking (meds, water, food)
    - Minimum viable human protocols
    - Routine adaptation to capacity

### Phase 4: Integration & Polish
11. **Discord Bot Integration**
    - Slash commands for agent interaction
    - Proactive notifications from Pattern
    - Multi-modal conversations

12. **Advanced Features**
    - Vector search for semantic memory
    - Cross-platform messaging
    - Energy pattern learning
    - Relationship evolution tracking



## Current TODOs

### High Priority
- [ ] Test SSE MCP integration with Letta (now implemented, needs testing)
- [ ] Implement Pattern as sleeptime agent with 20-30min background checks
- [ ] Add task CRUD operations to database module
- [ ] Create task manager with ADHD-aware task breakdown (Entropy agent)
- [ ] Create shared tools (check_vibe, context_snapshot, find_pattern, suggest_pivot)
- [ ] Add contract/client tracking (time, invoices, follow-ups)
- [ ] Implement social memory (birthdays, follow-ups, conversation context)

### Medium Priority
- [ ] Add task-related MCP tools
- [ ] Implement time tracking with ADHD multipliers (Flux agent)
- [ ] Add energy/attention monitoring (Momentum agent)
- [ ] Consider alternative to MCP if SSE doesn't work (direct tool registration via Letta API)

### Low Priority
- [ ] Add activity monitoring for interruption detection
- [ ] Bluesky integration for public accountability posts (see docs/BLUESKY_SHAME_FEATURE.md)

### Completed
- [x] Letta integration layer with agent management
- [x] Library restructure with feature flags
- [x] Database module with SQLite migrations
- [x] Multi-agent system architecture (Pattern + 5 agents)
- [x] Discord bot with slash commands and agent routing
- [x] MCP server with 10+ tools (official SDK patterns)
- [x] Multiple MCP transports (stdio, HTTP, SSE)
- [x] Comprehensive test suite
- [x] Documentation refactoring

## Current Status (2025-01-03)

**Architecture**: Unified binary with feature flags
- Single `pattern` binary can run Discord bot, MCP server, and background tasks
- Feature flags: `discord`, `mcp`, `binary`, `full`
- Modular service architecture via PatternService

**Database** ✅:
- SQLite with migrations
- Users table now has `name` field (+ optional `discord_id`)
- Shared memory, agents, tasks, events, time tracking
- Contract/client tracking schema ready
- Social memory schema ready

**Testing** ✅:
- Comprehensive test suite that validates actual behavior
- Tests that can actually fail (not just string checks)
- Coverage across server, db, types, and agents modules

**Multi-Agent System** ✅:
- Generic, flexible architecture with configurable agents
- Shared memory blocks (current_state, active_context, bond_evolution)
- Background sleeptime orchestrator (30min intervals)
- Dynamic agent routing in Discord - no hardcoded names
- Full Letta integration with message routing
- **NEW**: Proper system prompt + persona separation in agent creation
- **NEW**: Agent tagging for broadcast filtering
- **NEW**: Pattern is primary conversant/interface, delegates to specialists

**Agent Coordination** ✅:
- Created coordination.rs module with message tagging system
- MessageSource enum to distinguish User/Agent/System/Tool messages
- Inter-agent communication rules in all system prompts
- Prevents infinite loops from tool confirmations
- cleanup_agents.sh script supports both local and Letta Cloud

**Discord Bot** ✅:
- Natural language chat with slash commands
- DM support with agent routing (@agent, agent:, /agent)
- Configurable agent detection
- Message chunking for long responses
- Channel name resolution ("Server/channel" format)
- Fixed ephemeral messages, timeouts, and initialization issues

**MCP Server** ✅:
- Ten tools: chat_with_agent, get/update_agent_memory, schedule_event, send_message, check_activity_state
- Discord integration tools: send_discord_message, send_discord_embed, get_channel_info, send_discord_dm
- Channel resolution accepts both IDs and "guild/channel" names
- **Transports implemented**: 
  - Stdio transport for local development
  - Streamable HTTP (has issues with Letta's Python client)
  - SSE transport (recommended for Letta compatibility)
- **SSE configuration**: Set `mcp.transport = "sse"` and `mcp.port = 8081`
- Uses official modelcontextprotocol/rust-sdk from git
- Proper tool definitions with `#[rmcp::tool]` attribute
- Integrated with multi-agent system

**Running Pattern**:
```bash
# Full mode (Discord + MCP + background tasks)
cargo run --features full

# Just Discord
cargo run --features binary,discord

# Just MCP
cargo run --features binary,mcp
```

## Next Steps

### Sleeptime Orchestrator (Immediate)
1. **Implement Pattern's background checks**
   - Actually perform meaningful checks every 20-30min
   - Monitor for hyperfocus duration (>45min warnings)
   - Check physical needs (water, food, movement)
   - Detect transitions and context switches
   - Update shared memory with findings
   - Optionally send Discord messages for important alerts

### Task Management (High Priority)
1. **Extend database module** with task operations
   - Add CRUD methods for tasks
   - Implement task status transitions
   - Add task breakdown storage (parent/child tasks)
   - Track estimated vs actual time

2. **Create task manager module** (`src/tasks.rs`)
   - Task creation with ADHD-aware defaults
   - Task breakdown into atomic units
   - Automatic time multiplication (2-3x)
   - Hidden complexity detection
   - Integration with Entropy agent

3. **Add task-related MCP tools**
   - `create_task`, `update_task`, `list_tasks`
   - `break_down_task` (uses Entropy agent)
   - `estimate_task_time` (uses Flux agent)

### Shared Agent Tools (High Priority)
1. **Implement core shared tools**
   - `check_vibe()`: Quick state assessment
   - `context_snapshot()`: Capture current context
   - `find_pattern()`: Search memory for patterns
   - `suggest_pivot()`: Task/energy mismatch detection

### Contract & Social Features (Medium Priority)
1. **Contract/Client tracking**
   - CRUD operations for clients, projects, invoices
   - Time entry tracking with billable hours
   - Invoice aging alerts (30/60/90 days)
   - Follow-up reminders

2. **Social memory**
   - Birthday/anniversary tracking with reminders
   - Conversation context storage
   - Follow-up suggestions
   - Energy cost tracking for social interactions

## References

### External Documentation
- [Official MCP Rust SDK](https://github.com/modelcontextprotocol/rust-sdk) - Use git version only
- [MCP Specification](https://modelcontextprotocol.io/specification/2025-06-18)
- [Letta Documentation](https://docs.letta.com/)
- [Discord.py Interactions Guide](https://discordpy.readthedocs.io/en/stable/interactions/api.html) (concepts apply to serenity)
- [Activity Detection Research](https://dl.acm.org/doi/10.1145/3290605.3300589)

### Project Documentation
See the organized documentation in the `docs/` directory, especially:
- Architecture guides in `docs/architecture/`
- Integration guides in `docs/guides/`
- API references in `docs/api/`
- Troubleshooting in `docs/troubleshooting/`
