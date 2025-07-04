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
- [Memory and Groups Architecture](./docs/architecture/MEMORY_AND_GROUPS.md) - Memory hierarchy, Letta groups, and sleeptime strategy
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

### Phase 1: Core Foundation (Must Have First) ✅
1. **Multi-Agent Architecture Design** ✅
   - Pattern orchestrator + 5 specialist agents
   - Three-tier memory hierarchy
   - Letta groups for communication patterns
   - **Status**: Architecture defined, see [Memory and Groups doc](./docs/architecture/MEMORY_AND_GROUPS.md)

2. **Letta Groups Integration** 🚧
   - Replace custom routing with native groups API
   - Implement overlapping group configurations
   - Shared memory blocks via groups
   - **Why now**: Simplifies everything, native multi-agent support

3. **Custom Sleeptime Architecture** 🚧
   - Two-tier monitoring (cheap rules + expensive intervention)
   - Conditional Pattern awakening
   - Cost-optimized background processing
   - **Why now**: Core ADHD support without breaking the bank

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



## Groups Refactoring Plan (2025-01-04)

### Phase 1: Add Groups Support to MultiAgentSystem ⚡
1. **Add group management methods**:
   - `create_group()` - Create a new group with specified agents and manager
   - `get_or_create_group()` - Get existing or create new group
   - `list_user_groups()` - List all groups for a user
   - `delete_group()` - Remove a group

2. **Database schema for groups**:
   - Add `groups` table with columns: id, user_id, group_id, name, config
   - Similar caching strategy as agents
   - Store manager type and configuration

3. **Default group creation**:
   - Create "main" group on user initialization
   - All agents as members, dynamic manager
   - Keep individual agent support for backwards compatibility

### Phase 2: Flexible Group Creation
1. **Configurable group types**:
   - Support all manager types: dynamic, supervisor, round_robin, sleeptime
   - Allow custom shared memory blocks per group
   - Max turns, termination tokens, etc.

2. **Overlapping groups**:
   - Same agents can be in multiple groups
   - Different coordination patterns for different contexts
   - Store group metadata (purpose, context triggers)

### Phase 3: Update Message Routing
1. **Discord bot changes**:
   - Add group detection logic (keywords, context)
   - Route to appropriate group based on message content
   - Fallback to individual agents for specific requests

2. **MCP tool updates**:
   - Add `chat_with_group` tool
   - Add `create_custom_group` tool
   - Update existing tools to support group_id parameter

3. **Group selection logic**:
   ```rust
   fn select_group(message: &str, user_state: &UserState) -> Option<GroupId> {
       // Crisis detection → crisis group
       // Planning keywords → planning group  
       // Memory questions → archive-led group
       // Default → main conversational group
   }
   ```

### Phase 4: Example Patterns
1. **Main group** - Dynamic routing for natural conversation
2. **Sleeptime group** - Archive + Pattern for memory work
3. **Custom groups** - User-definable via config or API

### Implementation Order
1. ✅ Research Letta groups API
2. ✅ Add groups table to database schema
3. ✅ Add group CRUD methods to MultiAgentSystem
4. ✅ Create default "main" group on user init
5. ✅ Update Discord bot to use groups
6. ✅ Add MCP tools for groups
7. ⚡ Test different manager types
8. ⚡ Document group patterns

## Current TODOs

### High Priority
- [x] Add MCP tools for group messaging (send_group_message, list_groups, create_group, delete_group)
- [x] Unified messaging interface - single send_message tool with routing (2025-01-04)
- [ ] Test different group manager types (Dynamic, Supervisor, Sleeptime)
- [ ] Experiment with flexible group configurations
- [ ] Build custom tiered sleeptime monitor (rules-based + Pattern intervention)
- [ ] Implement passive knowledge sharing via Letta sources
- [ ] Add task CRUD operations to database module
- [ ] Create task manager with ADHD-aware task breakdown
- [ ] Add contract/client tracking (time, invoices, follow-ups)
- [ ] Implement social memory (birthdays, follow-ups, conversation context)

### Medium Priority
- [ ] Add Discord context tools to MCP (channel history, user info)
- [ ] Add task-related MCP tools
- [ ] Implement time tracking with ADHD multipliers (Flux agent)
- [ ] Add energy/attention monitoring (Momentum agent)
- [ ] Add `/reload_config` Discord command to hot-reload model mappings and agent configs

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
- [x] Agent update functionality (instead of delete/recreate for cloud limits)
- [x] Fixed agent infinite loop issue when prefixing messages
- [x] Removed MCP tool name conflicts with Letta defaults
- [x] Implemented database caching for agent IDs (multi-agent support)
- [x] Load all system prompts and agent configurations from a config file
- [x] Optimized agent initialization to eliminate API calls for existing agents
- [x] Model capability abstraction system (Routine/Interactive/Investigative/Critical)
- [x] Configurable model mappings in pattern.toml (global and per-agent)

## Current Status (2025-01-04)

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
- **NEW**: Agent caching table with multi-agent support
- **NEW**: Unique constraint on (user_id, agent_type) for proper caching

**Testing** ✅:
- Comprehensive test suite that validates actual behavior
- Tests that can actually fail (not just string checks)
- Coverage across all modules: agent, db, mcp, discord, service
- All 27 tests passing after module reorganization

**Multi-Agent System** ✅:
- Generic, flexible architecture with configurable agents
- Three-tier memory hierarchy:
  - Core memory blocks (immediate access)
  - Letta sources (searchable knowledge)
  - Archival memory (deep storage)
- **NEW**: Native Letta groups for agent coordination:
  - Overlapping groups for different contexts
  - Multiple manager types (dynamic, supervisor, sleeptime)
  - Shared conversation history within groups
- Background sleeptime via Letta's sleeptime manager
- Dynamic agent routing in Discord - no hardcoded names
- Full Letta integration with message routing
- Pattern is primary conversant/interface, delegates to specialists
- Database caching eliminates API calls for existing agents
- See [Memory and Groups Architecture](./docs/architecture/MEMORY_AND_GROUPS.md)

**Agent Coordination** ✅:
- agent/coordination.rs module with message tagging system
- MessageSource enum to distinguish User/Agent/System/Tool messages
- Inter-agent communication rules in all system prompts
- Prevents infinite loops from tool confirmations
- cleanup_agents.sh script supports both local and Letta Cloud
- **NEW**: Added no-emoji rule to system prompts to prevent emoji explosions
- **NEW**: AgentCoordinator replaces non-existent AgentManager
- **NEW**: Both system prompt (unchangeable) and persona (evolvable) blocks configured
- **NEW**: External agent configuration file support (TOML format)
- **NEW**: Environment variable `AGENT_CONFIG_PATH` for config file location

**Discord Bot** ✅:
- Natural language chat with slash commands
- DM support with agent routing (@agent, agent:, /agent)
- Configurable agent detection
- Message chunking for long responses
- Channel name resolution ("Server/channel" format)
- Fixed ephemeral messages, timeouts, and initialization issues
- **NEW**: `/debug_agents` command to log agent chat histories to disk for debugging

**MCP Server** ✅:
- Core tools: chat_with_agent, get/update_agent_memory, schedule_event, check_activity_state, update_agent_model
- Group tools: send_group_message
- **NEW**: Unified send_message tool with routing to agents, groups, or Discord (2025-01-04)
- **REMOVED**: Individual messaging tools and group management tools per user request
- Discord channel resolution accepts both IDs and "guild/channel" names
- **NEW**: Model capability abstraction in update_agent_model tool:
  - `routine` - Everyday tasks, quick checks (default: groq/llama-3.1-8b-instant)
  - `interactive` - Normal conversations (default: groq/llama-3.3-70b-versatile)
  - `investigative` - Research, debugging, analysis (default: openai/gpt-4o)
  - `critical` - High-stakes decisions (default: anthropic/claude-3-opus-20240229)
  - Temperature is now configured in pattern.toml, not exposed in the MCP tool
- **NEW**: Configurable model mappings in pattern.toml:
  - Global defaults under `[models.default]`
  - Per-agent overrides under `[models.agents.{agent_id}]`
  - Temperature settings can be configured globally or per-agent
- **Transports implemented**:
  - Stdio transport for local development
  - Streamable HTTP (has issues with Letta's Python client)
  - SSE transport (recommended for Letta compatibility)
- **SSE configuration**: Set `mcp.transport = "sse"` and `mcp.port = 8081`
- Uses official modelcontextprotocol/rust-sdk from git
- Proper tool definitions with `#[rmcp::tool]` attribute
- Integrated with multi-agent system

**Debug Features** ✅:
- Agent history logging to disk with `log_agent_histories()` method
- Logs all message types: user, assistant, system, tool calls, reasoning
- Accessible via `/debug_agents` Discord command
- Files saved to `logs/agent_histories/` with timestamps
- Uses async file I/O to avoid blocking the runtime

**Running Pattern**:
```bash
# Full mode (Discord + MCP + background tasks)
cargo run --features full

# Just Discord
cargo run --features binary,discord

# Just MCP
cargo run --features binary,mcp
```

## Partner/Conversant Architecture (2025-01-03)

**Key Distinction**: Pattern uses a partner-centric model to ensure privacy and personalization.

### Terminology
- **Partner**: The person who owns a constellation of agents and gets ADHD support
- **Conversant**: Someone interacting with Pattern through the partner's agents (e.g., in Discord)

### Architecture Overview
```
Single MCP Server (shared infrastructure)
    ↓
Partner 1 Constellation:
- pattern_1 (orchestrator)
- entropy_1, flux_1, archive_1, momentum_1, anchor_1
- Shared memory: partner_1_state, partner_1_context, partner_1_bonds
- Private content: DMs, personal notes, sensitive context

Partner 2 Constellation:
- pattern_2 (orchestrator)
- entropy_2, flux_2, archive_2, momentum_2, anchor_2
- Shared memory: partner_2_state, partner_2_context, partner_2_bonds
- Private content: Isolated from Partner 1

Discord/Platform Context:
- DM from partner → exclusive access to their constellation
- Channel message from partner → their constellation + channel context
- Channel message from conversant → routed through channel owner's constellation
```

### Privacy & Context Isolation
1. **Strict Boundaries**: DM content NEVER bleeds into public channels
2. **Context Loading**: Dynamic memory block swapping based on interaction context
3. **Partner Registry**: Track constellation ownership and access permissions
4. **Channel Context**: Public interactions can see channel history but not private memory

### Implementation Strategy
1. **Agent State Updates**: Use Letta's update APIs instead of delete/recreate (cloud limits)
   - ✅ Added `update_agent()` method to update system prompts without recreation
   - ✅ Added `update_all_agents()` to batch update all agents for a user
   - ✅ Automatic update detection in `create_or_get_agent()`
2. **Dynamic Memory Loading**:
   - Primary blocks: Partner's personal memory (always loaded)
   - Secondary blocks: Conversant info (loaded for group interactions)
   - Channel blocks: Shared channel context (public interactions only)
3. **Routing Logic**:
   ```rust
   match message_source {
       DM(user_id) => load_partner_constellation(user_id),
       Channel(channel_id, user_id) => {
           let partner = get_channel_owner(channel_id);
           let constellation = load_partner_constellation(partner);
           constellation.add_conversant_context(user_id);
       }
   }
   ```

### Scaling Considerations
- Each partner gets a full constellation (6 agents)
- Agents persist between conversations (no constant recreation)
- Inactive agents can be "hibernated" after timeout
- Multi-partner MCP server handles routing and context switching

### Future Enhancements
- **Agent Pooling**: Share specialist agents across partners for better resource usage
- **Context Inheritance**: Learn from all interactions while maintaining privacy
- **Cross-Partner Insights**: Anonymous pattern detection across all partners
- **Conversant Profiles**: Build understanding of frequent conversants

This architecture ensures:
- Complete privacy for partner's personal context
- Consistent agent personalities within each constellation
- Efficient resource usage through shared infrastructure
- Flexibility for future multi-tenant scenarios

## Module Organization (Completed 2025-01-04)

The module structure has been reorganized for better clarity and maintainability:

**Current Structure:**
```
src/
├── agent/                  # All agent-related code
│   ├── mod.rs             # Shared agent types (AgentId, MemoryBlockId, StandardAgent, etc.)
│   ├── constellation.rs   # Multi-agent system (MultiAgentSystem, AgentConfig)
│   ├── builder.rs         # MultiAgentSystemBuilder for easier configuration
│   ├── coordination.rs    # Message routing and AgentCoordinator
│   └── human.rs           # UserId type, future Partner/Conversant types
├── mcp/                   # MCP server & tools
│   ├── mod.rs            # MCP request/response types
│   ├── server.rs         # MCP server implementation
│   ├── core_tools.rs     # Core MCP tools (chat, memory, events)
│   └── discord_tools.rs  # Discord-specific MCP tools
├── discord/              # Discord bot
│   └── mod.rs           # Discord bot implementation with slash commands
├── service.rs           # PatternService orchestrator
├── config.rs            # Configuration management
├── db.rs                # Database module with SQLite
├── error.rs             # Error types and handling
└── lib.rs               # Library entry point
```

**Key Changes from Refactor:**
- ✅ Consolidated all agent types in `agent/mod.rs`
- ✅ Replaced non-existent `AgentManager` with `AgentCoordinator`
- ✅ Fixed method name: `get_agent_memory` → `get_shared_memory`
- ✅ Removed `types.rs` - types now live in their respective modules
- ✅ All imports updated to new paths
- ✅ All 27 tests passing

**Import Path Changes:**
- `use crate::agents::` → `use crate::agent::constellation::`
- `use crate::server::` → `use crate::mcp::server::`
- `use crate::mcp_tools::` → `use crate::mcp::discord_tools::`
- `use crate::types::` → `use crate::agent::` (for agent types)

## Next Steps

### Implement Letta Groups Architecture (Immediate)
1. **Refactor to use native Letta groups**
   - Replace custom message routing with groups API
   - Experiment with different group configurations
   - Create groups dynamically based on context
   - Test different manager types (dynamic, supervisor, sleeptime)
   - See [Memory and Groups Architecture](./docs/architecture/MEMORY_AND_GROUPS.md)

### Custom Sleeptime Implementation
1. **Build tiered monitoring system**
   - Tier 1: Lightweight monitor (every 20min, rules-based)
   - Tier 2: Pattern intervention (triggered by concerning patterns)
   - Cost-optimized with conditional awakening
   - See sleeptime architecture in [Memory and Groups doc](./docs/architecture/MEMORY_AND_GROUPS.md#custom-tiered-sleeptime-architecture)

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
   - Use planning group for coordinated breakdown

3. **Add task-related MCP tools**
   - `create_task`, `update_task`, `list_tasks`
   - `break_down_task` (routes to planning group)
   - `estimate_task_time` (uses Flux in planning group)

### Shared Agent Tools via Sources (High Priority)
1. **Implement passive knowledge sharing**
   - `write_to_shared_insights()` tool for all agents
   - Auto-embedded markdown files per domain:
     - `task_patterns.md` (Entropy)
     - `energy_patterns.md` (Momentum)
     - `time_patterns.md` (Flux)
     - `routine_patterns.md` (Anchor)
   - Semantic search across all insights

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

## Future Enhancement Ideas

### Debug Agent Logging Improvements
- **Selective Agent Logging**: Add option to log only specific agents
- **Discord Channel Upload**: Send logs directly to a debug channel
- **Date Range Filtering**: Log only messages from last N hours/days
- **Zip Compression**: Bundle all logs into a downloadable archive
- **Log Formatting Options**: JSON, CSV, or markdown formats
- **Message Filtering**: Filter by message type (user/assistant/tool/etc)
- **Conversation Threading**: Group messages by conversation sessions

### Agent Configuration File
- Move all system prompts to YAML/TOML configuration
- Allow hot-reloading of agent personalities
- Version control for prompt iterations
- Environment-specific configurations (dev/prod)
- Agent capability matrices in config

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
