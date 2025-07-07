# CLAUDE.md - Pattern ADHD Cognitive Support System

Pattern is a multi-agent ADHD support system using Letta's architecture to provide external executive function through specialized cognitive agents.

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

- **ALWAYS prefer enums to string validation** - Rust enums are very powerful! Use them for type safety. The ability to have unit, tuple, and struct variants means you can have e.g. an `AgentType` enum with all standard agents and then a `Custom(String)` variant as a catch-all.
- **ALWAYS check if files/scripts/functions exist before creating new ones** - Use `ls`, `find`, `grep`, or read existing code first
- Run `cargo check` frequently when producing code. This will help you catch errors early.
- NEVER use `unsafe{}`. If you feel you need to, stop, think about other ways, and ask the user for help if needed.
- NEVER ignore a failing test or change a test to make your code pass
- NEVER ignore a test
- ALWAYS fix compile errors before moving on.
- **ALWAYS ENSURE that tests will fail (via assert or panic with descriptive message) on any error condition**
- Use the web or context7 to help find docs, in addition to any other reference material
- **AVOID C dependencies** - Prefer pure Rust implementations to avoid build complexity
- **Use `#[serde(skip_serializing_if = "Option::is_none")]`** on all `Option<T>` fields in serializable types to reduce JSON payload size

## Workspace Structure

The project is now organized as a Rust workspace with the following crates:
- `crates/pattern_core` - Core agent framework and memory system
- `crates/pattern_nd` - Neurodivergent support tools and ADHD agents
- `crates/pattern_mcp` - MCP server implementation
- `crates/pattern_discord` - Discord bot integration
- `crates/pattern_main` - Main orchestrator that ties everything together

**Note**: Crate directories use underscores but package names use hyphens (e.g., directory `pattern_core` contains package `pattern-core`).

### Crate-Specific Documentation

Each crate has its own `CLAUDE.md` file with detailed implementation guidelines:
- [`pattern_core/CLAUDE.md`](./crates/pattern_core/CLAUDE.md) - Agent framework, memory, tools, context building
- [`pattern_nd/CLAUDE.md`](./crates/pattern_nd/CLAUDE.md) - ADHD tools, agent personalities, support patterns
- [`pattern_mcp/CLAUDE.md`](./crates/pattern_mcp/CLAUDE.md) - MCP server, transport layers, tool schemas
- [`pattern_discord/CLAUDE.md`](./crates/pattern_discord/CLAUDE.md) - Discord bot, routing, privacy
- [`pattern_main/CLAUDE.md`](./crates/pattern_main/CLAUDE.md) - Integration, configuration, lifecycle

**When working on a specific crate, always check its CLAUDE.md first for crate-specific patterns and guidelines.**

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



## Build Priority Breakdown

### Phase 1: Core Foundation (In Progress)
1. **Letta Groups Integration** 🚧
   - Test different manager types (dynamic, supervisor, sleeptime)
   - Document group patterns and best practices
   - **Status**: Groups implemented, need testing and documentation

2. **Custom Sleeptime Architecture** ✅
   - Two-tier monitoring (cheap rules + expensive intervention)
   - Conditional Pattern awakening
   - Cost-optimized background processing
   - **Status**: Implemented with configurable intervals and ADHD-aware triggers

### Phase 2: Core Features (MVP)
1. **Task Management System**
   - Add task CRUD operations to database module
   - Create task manager with ADHD-aware task breakdown
   - Hidden complexity detection and atomic task creation
   - Add task-related MCP tools
   - **Why**: Task paralysis is core ADHD challenge

2. **Pattern Sleeptime Agent**
   - Implement 20-30min background checks
   - Hyperfocus detection, physical needs monitoring
   - **Why**: Core ADHD support mechanism

3. **Shared Agent Tools**
   - Implement check_vibe, context_snapshot, find_pattern, suggest_pivot
   - Create tool registry for agent access
   - **Why**: Agents need common capabilities

### Phase 3: Specialist Agent Features
1. **Contract & Client Tracking**
   - Time entry tracking with billable hours
   - Invoice aging alerts (30/60/90 days)
   - Follow-up reminders

2. **Social Memory**
   - Birthday/anniversary tracking with reminders
   - Conversation context storage
   - Follow-up suggestions
   - Energy cost tracking for social interactions

3. **Time & Energy Monitoring**
   - ADHD time multipliers (Flux agent)
   - Energy/attention state monitoring (Momentum agent)
   - Activity monitoring for interruption detection

### Phase 4: Integration & Polish
1. **Discord Enhancements**
    - Discord context tools to MCP (channel history, user info)
    - `/reload_config` command for hot-reload
    - Proactive notifications from Pattern
    - Multi-modal conversations

12. **Advanced Features**
    - Vector search for semantic memory
    - Cross-platform messaging
    - Energy pattern learning
    - Relationship evolution tracking



## Groups Implementation Status

✅ **Completed**: Native Letta groups with dynamic, supervisor, round-robin, and sleeptime managers
- Database schema and caching with Foyer
- Default groups created on user init: main, crisis, planning, memory
- Discord bot routes to groups based on message content
- Unified `send_message` tool handles all routing

🚧 **Remaining**:
- Test different manager types in production
- Document group patterns and best practices
- Add configurable custom groups via API/config

## Current TODOs

### High Priority
- Implement core agent framework in pattern-core (Agent trait, memory system, tool execution) 🚧
- ~~Set up SurrealDB integration for persistence~~ ✅
- Migrate existing agent logic from Letta to pattern-core
- Add task CRUD operations to database module
- Create task manager with ADHD-aware task breakdown
- Add contract/client tracking (time, invoices, follow-ups)
- Implement social memory (birthdays, follow-ups, conversation context)

### Medium Priority
- Add Discord context tools to MCP (channel history, user info)
- Add task-related MCP tools
- Implement time tracking with ADHD multipliers (Flux agent)
- Add energy/attention monitoring (Momentum agent)
- Add `/reload_config` Discord command to hot-reload model mappings and agent configs

### Low Priority
- Add activity monitoring for interruption detection
- Bluesky integration for public accountability posts (see docs/BLUESKY_SHAME_FEATURE.md)

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
- [x] Implement passive knowledge sharing via Letta sources (2025-07-04)
- [x] Knowledge tools for agents (write_agent_knowledge, read_agent_knowledge, sync_knowledge_to_letta)
- [x] Implement schedule_event MCP tool with database storage (2025-07-05)
- [x] Implement check_activity_state MCP tool with energy state tracking (2025-07-05)
- [x] Add record_energy_state MCP tool for tracking ADHD energy/attention states (2025-07-05)
- [x] Refactor caching from SQLite KV store to foyer library (disk-backed async cache) (2025-07-05)
- [x] Fixed MCP tool schema issues - removed references, enums, nested structs, unsigned ints (2025-07-05)
- [x] Implemented custom tiered sleeptime monitor with rules-based checks and Pattern intervention (2025-07-06)
- [x] Refactored to multi-crate workspace structure (2025-07-06)
- [x] Created pattern-core with agent framework, memory system, and tool registry
- [x] Created pattern-nd with ADHD-specific tools and agent personalities
- [x] Created pattern-mcp with MCP server implementation
- [x] Created pattern-discord with Discord bot functionality
- [x] Implemented rich error types using miette for all crates
- [x] Removed C dependencies (using SurrealDB without rocksdb, rustls instead of native TLS)
- [x] Set up proper Nix flake for workspace builds

## Current Status

**Last Updated**: 2025-07-06

### System Architecture ✅
- **Unified binary** with feature flags (`discord`, `mcp`, `binary`, `full`)
- **Foyer caching layer**: Hybrid memory/disk caching for agents, groups, memory blocks
- **Multi-agent constellation**: Pattern orchestrator + 5 specialist agents (Entropy, Flux, Archive, Momentum, Anchor)
- **Native Letta groups**: Main (dynamic), crisis (round-robin), planning (supervisor), memory (sleeptime)
- **Three-tier memory**: Core blocks, Letta sources, archival storage
- **Custom sleeptime monitor**: Two-tier ADHD monitoring (lightweight checks + Pattern intervention)
- **Database backend**: Pure Rust SurrealKV with vector search, migrations, and embedding support

### Recent Achievements (2025-07-04 to 2025-07-06)
- **Caching migration**: SQLite KV store → Foyer library with automatic tiering
- **MCP schema fixes**: Removed unsupported references, enums, nested structs
- **Partner threading**: Agents receive actual partner names and Discord IDs
- **Unified messaging**: Single `send_message` tool routes to agents/groups/Discord
- **ADHD tools**: Energy tracking, event scheduling, interruptibility detection
- **Auto tool attachment**: MCP tools automatically attached to agents on creation
- **Group management**: Proper caching prevents constant recreation
- **Agent configuration**: External TOML files for prompts and model mappings
- **Sleeptime monitoring**: Automatic ADHD support with hyperfocus detection, break reminders
- **Workspace refactor**: ✅ Migrated from monolithic to multi-crate workspace structure (2025-07-06)
- **Removed Letta dependency**: ✅ Building our own agent framework with SurrealDB
- **Rich error types**: ✅ All errors now use miette for detailed diagnostics
- **Pure Rust dependencies**: ✅ Removed C dependencies (no rocksdb, using rustls)
- **Tool registry optimization**: ✅ DashMap for concurrent access, CompactString for memory efficiency
- **JSON payload optimization**: ✅ Added `skip_serializing_if` to all Option fields
- **Database implementation**: ✅ SurrealDB embedded with schema migrations and vector search
- **Embedding providers**: ✅ Feature-gated providers - Candle (Jina/BERT), OpenAI, Cohere, Ollama (stub)
- **Simplified abstractions**: ✅ Removed Java-esque patterns (repositories, factories) for direct operations

### Key Features Implemented

**Database**: 
- SQLite with migrations, Foyer caching layer (pattern-main)
- SurrealDB with vector search, migrations (pattern-core)
- Users, agents, groups, tasks, events, energy states tables
- Memory blocks with embeddings for semantic search
- Contract/client and social memory schemas ready
- Unique constraints prevent duplicate agents/groups

**Embeddings**:
- ✅ Candle (local) - Pure Rust embeddings with Jina and BERT models
- ✅ OpenAI - text-embedding-3-small/large
- ✅ Cohere - embed-english-v3.0
- 🚧 Ollama - Stub implementation only
- Automatic embedding generation for memory blocks
- Semantic search with cosine similarity

**Testing**: 40+ tests that validate actual behavior (not mocks)

**Discord Bot**:
- Natural language chat with slash commands
- Smart group routing based on message content
- DM support with agent detection patterns
- `/debug_agents` command for troubleshooting

**MCP Server**:
- Unified `send_message` tool for all communication
- ADHD-aware tools: energy tracking, event scheduling
- Model capability abstraction (routine/interactive/investigative/critical)
- SSE transport recommended (`mcp.transport = "sse"`)

**Configuration**:
- `pattern.toml` for models, agents, MCP settings
- External agent config via `AGENT_CONFIG_PATH`
- Per-agent model overrides supported

### Running Pattern
```bash
cargo run --features full          # Discord + MCP + background
cargo run --features binary,discord # Just Discord
cargo run --features binary,mcp     # Just MCP
```

## Partner/Conversant Architecture (2025-07-03)

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

## Module Organization

**Workspace Structure:**
```
pattern/
├── Cargo.toml            # Workspace root
├── crates/
│   ├── pattern_core/     # Core agent framework
│   ├── pattern_nd/       # Neurodivergent tools
│   ├── pattern_mcp/      # MCP server
│   ├── pattern_discord/  # Discord bot
│   └── pattern_main/     # Main orchestrator
├── migrations/           # Database migrations
├── docs/                 # Documentation
└── nix/                  # Nix flake configuration
```

Each crate has its own dependencies and can be developed/tested independently.

## Next Steps

### Workspace Refactor ✅
- **Multi-crate workspace structure implemented**:
  - `pattern-core`: Agent framework, memory, tools (no Letta dependency)
  - `pattern-nd`: ADHD-specific tools and agent personalities
  - `pattern-mcp`: MCP server with multiple transports
  - `pattern-discord`: Discord bot integration
  - `pattern-main`: Main orchestrator
  - All crates use miette for rich error diagnostics
  - Pure Rust dependencies (no C deps)
  - Nix flake properly configured for workspace builds

### Custom Sleeptime Implementation ✅
- **Two-tier monitoring system implemented**:
  - Tier 1: Lightweight rules-based checks every 20 minutes
  - Tier 2: Pattern agent intervention for concerning patterns
  - Monitors: hyperfocus duration, sedentary time, hydration, energy levels
  - MCP tools: `trigger_sleeptime_check`, `update_sleeptime_state`
  - Automatic monitoring for all users with Discord IDs
  - Currently in `pattern_main/src/sleeptime.rs` (to be migrated to pattern-nd)

### Task Management (High Priority)
1. **Extend database module** with task operations ✅ Schema ready
   - Add CRUD methods for tasks in `db::ops`
   - Implement task status transitions
   - Add task breakdown storage (parent/child tasks)
   - Track estimated vs actual time
   - Integrate embeddings for semantic task search

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
   - Semantic search across all insights (✅ embedding infrastructure ready)

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
