# CLAUDE.md - Pattern Crate Implementation Guide

This crate extends the letta-rs client to build an autonomous agent system with MCP server capabilities.

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

- Run `cargo check` frequently when producing code. This will help you catch errors early.
- NEVER use `unsafe{}`. If you feel you need to, stop, think about other ways, and ask the user for help if needed.
- NEVER ignore a failing test or change a test to make your code pass
- NEVER ignore a test
- ALWAYS fix compile errors before moving on.
- **ALWAYS ENSURE that tests will fail (via assert or panic with descriptive message) on any error condition**
- Use the web or context7 to help find docs, in addition to any other reference material

## MCP Tools Available

Claude Code has access to several MCP (Model Context Protocol) tools that enhance development workflows:

### Context7 Tools
- **`mcp__context7__resolve-library-id`**: Resolves package names to Context7 library IDs
  - Use when you need to look up documentation for external libraries
  - Returns matching libraries with trust scores and documentation coverage
- **`mcp__context7__get-library-docs`**: Fetches library documentation
  - Requires library ID from resolve-library-id
  - Can focus on specific topics within documentation
  - Default 10k token limit (configurable)

### Language Server Tools (rust-analyzer)
- **`mcp__language-server__definition`**: Jump to symbol definitions
  - Find where functions, types, constants are implemented
  - Requires qualified symbol names (e.g., `pattern::server::PatternServer`)
- **`mcp__language-server__diagnostics`**: Get file diagnostics
  - Shows errors, warnings from rust-analyzer
  - Use before/after edits to verify code health
- **`mcp__language-server__edit_file`**: Batch file edits
  - Apply multiple line-based edits efficiently
  - Alternative to standard Edit tool for complex changes
- **`mcp__language-server__hover`**: Get type info and docs
  - Position-based (file, line, column)
  - Shows types, trait implementations, documentation
- **`mcp__language-server__references`**: Find all symbol usages
  - Critical before refactoring to understand impact
  - Returns all locations where symbol appears
- **`mcp__language-server__rename_symbol`**: Safe symbol renaming
  - Updates symbol and all references automatically
  - Position-based operation

## MCP Workflow Examples

### 1. Researching External Dependencies
```
1. User asks about using a library (e.g., "How do I use tokio channels?")
2. mcp__context7__resolve-library-id("tokio") → Get library ID
3. mcp__context7__get-library-docs(id, topic="channels") → Get relevant docs
4. Implement based on official documentation
```

**Note on letta-rs documentation:**
- The letta crate may not be on Context7 yet
- Alternative sources:
  - Read source directly: `../letta-rs/src/` (we have local access)
  - Generate docs: `cd ../letta-rs && cargo doc --open`
  - Check docs.rs if published
  - Use language server tools to explore the API

### 2. Understanding Existing Code
```
1. Find a function call you don't understand
2. mcp__language-server__hover(file, line, col) → Get quick info
3. mcp__language-server__definition("module::function") → See implementation
4. mcp__language-server__references("module::function") → See usage patterns
```

### 3. Safe Refactoring
```
1. mcp__language-server__references("OldName") → Assess impact
2. mcp__language-server__rename_symbol(file, line, col, "NewName") → Rename everywhere
3. mcp__language-server__diagnostics(file) → Verify no breakage
4. cargo test → Ensure tests still pass
```

### 4. Fixing Compilation Errors
```
1. cargo check → Initial error list
2. mcp__language-server__diagnostics(file) → Detailed diagnostics with context
3. mcp__language-server__hover on error locations → Understand type mismatches
4. mcp__language-server__edit_file → Apply fixes
5. mcp__language-server__diagnostics(file) → Verify fixes
```

### 5. Implementing New Features
```
1. mcp__context7 tools → Research library APIs if using external deps
2. Glob/Grep → Find similar patterns in codebase
3. mcp__language-server__definition → Understand interfaces to implement
4. Write implementation
5. mcp__language-server__diagnostics → Catch issues early
6. cargo test → Verify functionality
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


## Architecture Overview

Pattern acts as an MCP server exposing:
- Letta agent management and tool execution
- Calendar/scheduling with smart time estimation
- Activity monitoring for context-aware interruptions
- Cross-platform messaging (Discord primary)
- Task management with breakdown capabilities

## Core Dependencies

```toml
# MCP SDK - official rust implementation
rmcp = { version = "0.1", features = ["server"] }
# or for latest: rmcp = { git = "https://github.com/modelcontextprotocol/rust-sdk", branch = "main" }

# Async runtime
tokio = { version = "1.40", features = ["full"] }
tokio-stream = "0.1"

# Discord bot
serenity = { version = "0.12", features = ["framework", "cache", "rustls_backend"] }

# Calendar & scheduling
chrono = { version = "0.4", features = ["serde"] }
chrono-tz = "0.10"
rrule = "0.13"  # recurring events

# Embedded data persistence (no external services needed)
sqlx = { version = "0.8", features = ["runtime-tokio", "sqlite", "uuid", "chrono"] }
sled = "0.34"  # embedded key-value store

# Vector search
hnsw = "0.11"  # embedded vector similarity search

# Activity monitoring (platform-specific)
sysinfo = "0.32"
[target.'cfg(windows)'.dependencies]
windows = { version = "0.60", features = ["Win32_UI_WindowsAndMessaging", "Win32_System_Threading"] }
[target.'cfg(target_os = "linux")'.dependencies]
x11 = "2.21"

# Existing letta crate
letta = { path = "../letta-rs" }
```

## MCP Server Implementation

Using the official `rmcp` SDK ([docs](https://github.com/modelcontextprotocol/rust-sdk)):

```rust
use rmcp::{ServerHandler, model::ServerInfo, schemars, tool};
use letta::LettaClient;
use std::sync::Arc;

#[derive(Debug, Clone)]
pub struct PatternServer {
    letta_client: Arc<LettaClient>,
    db_pool: Arc<sqlx::PgPool>,
    discord_bot: Arc<serenity::Client>,
}

// Define tools using rmcp macros
#[tool(tool_box)]
impl PatternServer {
    #[tool(description = "Schedule an event with smart time estimation")]
    async fn schedule_event(
        &self,
        #[tool(aggr)] req: ScheduleEventRequest,
    ) -> Result<String, rmcp::Error> {
        // Implementation
        Ok("Event scheduled".to_string())
    }

    #[tool(description = "Send message to user via Discord")]
    async fn send_message(
        &self,
        #[tool(param)] channel_id: u64,
        #[tool(param)] message: String,
    ) -> Result<String, rmcp::Error> {
        // Send via Discord
        Ok("Message sent".to_string())
    }

    #[tool(description = "Check activity state for interruption timing")]
    fn check_activity_state(&self) -> Result<ActivityState, rmcp::Error> {
        // Platform-specific activity monitoring
        Ok(ActivityState::default())
    }
}
```

## Key Features to Implement

### 1. Letta Agent Wrapper ✅ IMPLEMENTED

Provides stateful agent management with caching:

```rust
// src/agent.rs - ACTUAL IMPLEMENTATION
pub struct AgentManager {
    letta: Arc<LettaClient>,
    db: Arc<db::Database>,
    cache: Arc<RwLock<HashMap<UserId, AgentInstance>>>,
}

impl AgentManager {
    pub async fn get_or_create_agent(&self, user_id: UserId) -> Result<AgentInstance> {
        // Check cache first, then DB, then create new
        let request = CreateAgentRequest::builder()
            .name(&agent_name)
            .memory_block(Block::persona("You are a helpful assistant..."))
            .memory_block(Block::human(&format!("User {}", user_id.0)))
            .build();
        
        // Store in DB and cache
        Ok(AgentInstance { agent_id: agent.id, user_id, name })
    }
    
    // Memory update workaround using blocks API
    pub async fn update_agent_memory(&self, user_id: UserId, memory: AgentMemory) -> Result<()> {
        for block in memory.blocks {
            self.letta.blocks().update(block_id, UpdateBlockRequest { ... }).await?;
        }
    }
}
```

**Key learnings**:
- Letta uses `Block` types, not `ChatMemory` 
- Memory updates require the blocks API
- `LettaId` is its own type, not just a String
- Message responses come as `LettaMessageUnion::AssistantMessage`

### 2. Smart Calendar Management

References:
- [Time blocking for ADHD](https://www.tiimoapp.com/resource-hub/time-blocking-for-adhders) - multiply time estimates by 2-4x
- [Calendar organization guide](https://akiflow.com/blog/adhd-calendar-guide/) - buffer time between tasks

```rust
struct SmartScheduler {
    calendar: CalendarService,
    user_patterns: HashMap<UserId, UserTimePatterns>,
}

impl SmartScheduler {
    async fn schedule_task(&self, task: Task, user_id: UserId) -> Result<Event> {
        let patterns = self.user_patterns.get(&user_id);

        // Apply time multiplier based on historical accuracy
        let duration = task.estimated_duration * patterns.time_multiplier;

        // Add buffer time (5min per 30min of task)
        let buffer = duration.num_minutes() / 30 * 5;

        // Find optimal slot considering energy levels
        let slot = self.find_slot(duration + buffer, patterns).await?;

        Ok(Event {
            start: slot.start,
            end: slot.end,
            buffer_before: 5,
            buffer_after: buffer,
            ..task.into()
        })
    }
}
```

### 3. Context-Aware Interruptions

Based on research showing [78-98% accuracy in detecting natural stopping points](https://dl.acm.org/doi/10.1145/3290605.3300589):

```rust
struct ActivityMonitor {
    last_input: Instant,
    current_app: String,
    typing_intensity: f32,
}

impl ActivityMonitor {
    fn detect_interruptibility(&self) -> InterruptibilityScore {
        // Detect hyperfocus: >45min without break, high input intensity
        if self.last_input.elapsed() < Duration::from_secs(5)
           && self.typing_intensity > 0.8 {
            return InterruptibilityScore::Low;
        }

        // Natural break points: app switch, idle time
        if self.last_input.elapsed() > Duration::from_mins(2) {
            return InterruptibilityScore::High;
        }

        InterruptibilityScore::Medium
    }
}
```

### 4. Discord Integration

Handle long-running operations with Discord's interaction model:

```rust
use serenity::builder::CreateInteractionResponseMessage;
use serenity::model::application::CommandInteraction;

async fn handle_analysis_command(
    ctx: &Context,
    interaction: &CommandInteraction,
) -> Result<()> {
    // Defer response for long operations
    interaction.create_response(&ctx.http, |r| {
        r.kind(InteractionResponseType::DeferredChannelMessageWithSource)
    }).await?;

    // Perform analysis
    let result = perform_long_analysis().await?;

    // Send followup within 15min window
    interaction.create_followup(&ctx.http, |f| {
        f.content(format!("Analysis complete: {}", result))
    }).await?;

    Ok(())
}
```

## Data Architecture

### Embedded Storage (No External Databases)

All data stored locally using embedded databases:

```rust
struct DataStore {
    sqlite: SqlitePool,        // relational data
    kv: sled::Db,             // key-value cache
    vectors: hnsw::HNSW,      // vector similarity search
}

impl DataStore {
    async fn new(path: &Path) -> Result<Self> {
        // Everything in one directory
        let sqlite = SqlitePool::connect(&format!("sqlite://{}/data.db", path)).await?;
        let kv = sled::open(path.join("cache"))?;

        // Build vector index from stored embeddings
        let vectors = Self::load_or_create_index(&sqlite).await?;

        Ok(Self { sqlite, kv, vectors })
    }
}
```

**SQLite** handles:
- User data, agent configs
- Calendar events and tasks
- Stored embeddings (as blobs)
- Full-text search via FTS5

**Sled** provides:
- Session cache
- Activity state tracking
- Rate limiting counters
- Fast ephemeral data

**HNSW** enables:
- Semantic memory search
- No external vector DB needed
- Rebuilds from SQLite on startup

Alternative: [sqlite-vss](https://github.com/asg017/sqlite-vss) extension for vectors directly in SQLite.

### Memory Management

Leverage Letta's 4-tier memory with local storage:
- **Core**: Current context (2KB limit) - in-memory
- **Archival**: SQLite + HNSW for unlimited storage with vector search
- **Message**: Recent history in Sled cache
- **Recall**: Semantic search via HNSW index

## Build Priority Breakdown

### Phase 1: Core Foundation (Must Have First)
1. **Basic MCP Server Setup**
   - Implement minimal `PatternServer` with rmcp
   - Basic tool registration
   - Server lifecycle management
   - **Why first**: Nothing works without this

2. **Letta Integration Layer**
   - `AgentManager` wrapper around letta client
   - Basic agent creation/retrieval
   - Message passing to agents
   - **Why early**: Core functionality depends on agents

3. **Local Data Storage**
   - SQLite setup with migrations
   - Basic user/agent persistence
   - **Why early**: Need persistence before anything stateful

### Phase 2: Core Features (MVP)
4. **Simple Task Management**
   - Basic task CRUD in SQLite
   - Task breakdown tool (no fancy AI yet)
   - **Why**: Immediate utility, simpler than calendar

5. **Discord Bot Integration**
   - Basic bot setup with slash commands
   - Message forwarding to/from agents
   - **Why**: Primary UI, needed for testing

6. **Basic Calendar**
   - Event storage and retrieval
   - Simple scheduling (no smart features yet)
   - **Why**: Foundation for smart scheduling later

### Phase 3: Intelligence Layer
7. **Smart Time Estimation**
   - Historical tracking of estimate vs actual
   - Time multiplier per user/task type
   - ADHD-aware buffering

8. **Activity Monitoring**
   - Platform-specific window/app tracking
   - Interruptibility detection
   - Focus state recognition

9. **Vector Search & Memory**
   - HNSW index for semantic search
   - Integration with Letta's archival memory
   - Smart context retrieval

### Phase 4: Advanced Features
10. **Recurring Events & Patterns**
    - rrule integration
    - User routine learning
    - Energy level patterns

11. **Cross-Platform Messaging**
    - Email integration
    - SMS/other platforms
    - Unified notification system

12. **Advanced Task Intelligence**
    - AI-powered task breakdown
    - Dependency tracking
    - Priority optimization

## Current TODOs

### High Priority
- [ ] Create basic task management
  - [ ] Add task CRUD operations to database module
  - [ ] Create task manager with smart task breakdown
  - [ ] Add task-related MCP tools

### Medium Priority
- [ ] Set up Discord bot integration
- [ ] Implement smart time estimation with ADHD awareness
- [ ] Add activity monitoring for interruption detection

### Completed
- [x] Implement Letta integration layer
  - [x] Create agent manager module (src/agent.rs)
  - [x] Implement agent creation/retrieval with caching
  - [x] Add Letta tools to MCP server (chat_with_agent, get_agent_memory, update_agent_memory)
  - [x] Implement memory update workaround using blocks API
- [x] Restructure as library with optional binary
  - [x] Create lib.rs with core PatternService
  - [x] Move MCP server to bin/mcp.rs
  - [x] Update Cargo.toml with feature flags (mcp, binary, mcp-sse)
- [x] Set up basic MCP server structure
- [x] Create database module with SQLite schema
- [x] Design initial schema (users, agents, tasks, events, time_tracking)
- [x] Create migrations
- [x] Create architecture breakdown and prioritization plan
- [x] Add build priority breakdown to CLAUDE.md
- [x] Add TODO mirroring note to CLAUDE.md

## Current Status

**Architecture**: Library crate with optional MCP server binary
- Core library provides PatternService for agent and task management
- MCP server binary behind `mcp` feature flag
- Clean separation between library and binary concerns

**Dependencies**:
- Core: sqlx, letta, sled, tokio, serde, miette, tracing
- Optional (MCP): rmcp
- Optional (binary): clap, tracing-subscriber, tracing-appender
- Note: async-trait was listed but isn't actually needed

**Features**:
- `default`: Core library functionality only
- `mcp`: Enables MCP server support + binary features
- `mcp-sse`: Enables SSE transport for MCP
- `binary`: Enables CLI and logging features

**Agent Integration** ✅:
- `AgentManager` in `src/agent.rs` handles all Letta operations
- Caching layer to minimize API calls
- Database persistence of agent mappings
- Memory update workaround implemented via blocks API

**Database** ✅:
- SQLite with migrations support
- Schema: users, agents, tasks, events, time_tracking tables
- Database module with entity structs and basic operations
- Agent persistence working (stores user->agent mappings)

**MCP Server** (when enabled) ✅:
- Six tools implemented:
  - `chat_with_agent` - Send messages to Letta agents and get responses ✅
  - `get_agent_memory` - Retrieve agent's current memory state ✅
  - `update_agent_memory` - Update agent memory blocks ✅
  - `schedule_event` - Schedule events (TODO: implement)
  - `send_message` - Send Discord messages (TODO: implement)
  - `check_activity_state` - Check activity/interruptibility (TODO: implement)
- Runs on stdio transport by default
- SSE transport available with feature flag
- Command-line options: `--db-path`, `--letta-url`, `--letta-api-key`

**Running the MCP server**:
```bash
# With local Letta server
cargo run --features mcp --bin mcp -- --letta-url http://localhost:8000

# With Letta cloud
cargo run --features mcp --bin mcp -- --letta-api-key your-api-key

# Custom database location
cargo run --features mcp --bin mcp -- --db-path /path/to/data.db --letta-url http://localhost:8000
```

## Next Steps

### Task Management (High Priority)
1. **Extend database module** with task operations
   - Add CRUD methods for tasks
   - Implement task status transitions
   - Add task breakdown storage

2. **Create task manager module** (`src/tasks.rs`)
   - Task creation with smart defaults
   - Task breakdown into subtasks
   - Priority and dependency management

3. **Add task MCP tools**
   - `create_task` - Create new task with optional breakdown
   - `list_tasks` - Get user's tasks with filtering
   - `update_task` - Update task status/details
   - `break_down_task` - AI-powered task decomposition

### Discord Integration (Medium Priority)
1. **Set up Discord bot**
   - Use serenity crate for Discord API
   - Implement slash commands
   - Handle long-running operations with deferred responses

2. **Connect bot to Pattern service**
   - Forward messages to Letta agents
   - Display task lists and calendars
   - Handle notifications and reminders

## Letta-rs API Notes

See [LETTA_API_REFERENCE.md](./LETTA_API_REFERENCE.md) for detailed API patterns and common gotchas discovered during implementation.

## References

- [Official MCP Rust SDK](https://github.com/modelcontextprotocol/rust-sdk)
- [MCP Specification](https://modelcontextprotocol.io/specification/2025-06-18)
- [Letta Documentation](https://docs.letta.com/)
- [Discord.py Interactions Guide](https://discordpy.readthedocs.io/en/stable/interactions/api.html) (concepts apply to serenity)
- [Activity Detection Research](https://dl.acm.org/doi/10.1145/3290605.3300589)
