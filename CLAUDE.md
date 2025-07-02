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

see @LSP_EDIT_GUIDE.md for details on potential pitfalls

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

Pattern implements a multi-agent cognitive support system with:

### Agent Constellation
```
Pattern (Sleeptime Orchestrator)
├── Entropy (Task/Complexity Agent)
├── Flux (Time/Scheduling Agent)
├── Archive (Memory/Knowledge Agent)
├── Momentum (Flow/Energy Agent)
└── Anchor (Habits/Structure Agent)
```

### Core Features
- **Sleeptime Orchestration**: Pattern runs background checks every 20-30 minutes for attention drift, physical needs, transitions
- **Shared Memory Blocks**: All agents access common state (current_state, active_context, bond_evolution)
- **ADHD-Specific Design**: Time blindness compensation, task breakdown, energy tracking, interruption awareness
- **Evolving Relationship**: Agents develop understanding of user patterns over time
- **MCP Server Interface**: Exposes agent capabilities through Model Context Protocol

## Core Dependencies

```toml
# MCP SDK - official rust implementation (must use git version)
rmcp = { git = "https://github.com/modelcontextprotocol/rust-sdk", branch = "main", optional = true }
# Must use schemars 0.8.x to match rmcp's version
schemars = "0.8.22"

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

## ADHD-Specific Design Principles

Pattern is built on deep understanding of ADHD cognition:

### Core Principles
- **Different, Not Broken**: ADHD brains operate on different physics - time blindness and hyperfocus aren't bugs
- **External Executive Function**: Pattern provides the executive function support that ADHD brains need
- **No Shame Spirals**: Never suggest "try harder" - validate struggles as logical responses
- **Hidden Complexity**: "Simple" tasks are never simple - everything needs breakdown
- **Energy Awareness**: Attention and energy are finite resources that deplete non-linearly

### Key Features for ADHD
- **Time Translation**: Automatic multipliers (1.5x-3x) for all time estimates
- **Proactive Monitoring**: Background checks prevent 3-hour hyperfocus crashes
- **Context Recovery**: External memory for "what was I doing?" moments
- **Task Atomization**: Break overwhelming projects into single next actions
- **Physical Needs**: Track water, food, meds, movement without nagging
- **Flow Protection**: Recognize and protect rare flow states

### Relationship Evolution
Agents evolve from professional assistant to trusted cognitive partner:
- **Early**: Helpful professional who "gets it"
- **Building**: Developing shorthand, recognizing patterns
- **Trusted**: Inside jokes, gentle ribbing, shared language
- **Deep**: Finishing thoughts about user's patterns

## Multi-Agent Shared Memory Architecture

### Shared Memory Blocks

All agents share these memory blocks for coordination without redundancy:

```rust
// Shared state accessible by all agents
pub struct SharedMemory {
    // Real-time energy/attention/mood tracking (200 char limit)
    current_state: Block,

    // What they're doing NOW, including blockers (400 char limit)
    active_context: Block,

    // Growing understanding of this human (600 char limit)
    bond_evolution: Block,
}

// Example state format
current_state: "energy: 6/10 | attention: fragmenting | last_break: 127min | mood: focused_frustration"
active_context: "task: letta integration | start: 10:23 | progress: 40% | friction: api auth unclear"
bond_evolution: "trust: building | humor: dry->comfortable | formality: decreasing | shared_refs: ['time is fake', 'brain full no room']"
```

### Agent Communication

Agents coordinate through:
- Shared memory updates (all agents see changes immediately)
- `send_message_to_agent_async` for non-blocking coordination
- Shared tools that any agent can invoke

## MCP Server Implementation

Using the official `rmcp` SDK from git (see [MCP_SDK_GUIDE.md](./MCP_SDK_GUIDE.md) for detailed patterns):

```rust
use rmcp::{
    handler::server::tool::Parameters,
    model::{CallToolResult, Content, Implementation, InitializeResult as ServerInfo},
    Error as McpError, ServerHandler,
};
use serde::{Deserialize, Serialize};
use schemars::JsonSchema;

#[derive(Debug, Clone)]
pub struct PatternMcpServer {
    letta_client: Arc<letta::LettaClient>,
    db: Arc<Database>,
    multi_agent_system: Arc<MultiAgentSystem>,
}

// Request structs need JsonSchema derive
#[derive(Debug, Deserialize, Serialize, schemars::JsonSchema)]
struct ScheduleEventRequest {
    user_id: i64,
    title: String,
    start_time: String,
    description: Option<String>,
    duration_minutes: Option<u32>,
}

// Define tools using rmcp macros
#[rmcp::tool_router]
impl PatternMcpServer {
    #[rmcp::tool(description = "Schedule an event with smart time estimation")]
    async fn schedule_event(
        &self,
        params: Parameters<ScheduleEventRequest>,
    ) -> Result<CallToolResult, McpError> {
        let params = params.0;
        // Implementation with ADHD time multipliers
        Ok(CallToolResult::success(vec![Content::text(
            "Event scheduled with ADHD-aware buffers"
        )]))
    }

    #[rmcp::tool(description = "Send message to user via Discord")]
    async fn send_message(
        &self,
        params: Parameters<SendMessageRequest>,
    ) -> Result<CallToolResult, McpError> {
        // Send via Discord bot instance
        Ok(CallToolResult::success(vec![Content::text(
            "Message sent via Discord"
        )]))
    }

    #[rmcp::tool(description = "Check activity state for interruption timing")]
    async fn check_activity_state(&self) -> Result<CallToolResult, McpError> {
        // Platform-specific activity monitoring
        Ok(CallToolResult::success(vec![Content::text(
            serde_json::to_string_pretty(&json!({
                "interruptible": true,
                "current_focus": null,
                "last_activity": "idle"
            })).unwrap()
        )]))
    }
}
```

## Key Features to Implement

### 1. Multi-Agent System Architecture

Each agent has a specific role in the cognitive support system:

#### Pattern (Sleeptime Orchestrator)
- Runs background checks every 20-30 minutes
- Monitors hyperfocus duration, physical needs, transitions
- Coordinates other agents based on current needs
- Personality: "friend who slides water onto your desk"

#### Entropy (Task/Complexity Agent)
- Breaks down overwhelming tasks into atoms
- Recognizes hidden complexity in "simple" tasks
- Validates task paralysis as logical response
- Finds the ONE next action when everything feels impossible

#### Flux (Time/Scheduling Agent)
- Translates between ADHD time and clock time
- Automatically adds buffers (1.5x-3x multipliers)
- Recognizes time blindness patterns
- Creates temporal anchors for transitions

#### Archive (Memory/Knowledge Agent)
- External memory bank for dumped thoughts
- Surfaces relevant context without prompting
- Finds patterns across scattered data points
- Answers "what was I doing?" with actual context

#### Momentum (Flow/Energy Agent)
- Distinguishes hyperfocus from burnout
- Maps energy patterns (Thursday 3pm crash, 2am clarity)
- Suggests task pivots based on current capacity
- Protects flow states when they emerge

#### Anchor (Habits/Structure Agent)
- Tracks basics: meds, water, food, sleep
- Builds loose structure that actually works
- Celebrates basic self-care as real achievements
- Adapts routines to current capacity

### Work & Social Support Features

Pattern's agents provide specific support for contract work and social challenges:

#### Contract/Client Management
- **Time Tracking**: Flux automatically tracks billable hours with "what was I doing?" recovery
- **Invoice Reminders**: Pattern notices unpaid invoices aging past 30/60/90 days
- **Follow-up Prompts**: "Hey, you haven't heard from ClientX in 3 weeks, might be time to check in"
- **Meeting Prep**: Archive surfaces relevant context before client calls
- **Project Switching**: Momentum helps context-switch between clients without losing state

#### Social Memory & Support
- **Birthday/Anniversary/Medication Tracking**: Anchor maintains important dates with lead-time warnings
- **Conversation Threading**: Archive remembers "they mentioned their dog was sick last time"
- **Follow-up Suggestions**: "Sarah mentioned her big presentation was today, maybe check how it went?"
- **Energy-Aware Social Planning**: Momentum prevents scheduling social stuff when depleted
- **Masking Support**: Pattern tracks social energy drain and suggests recovery time

Example interactions:
```
Pattern: "heads up - invoice for ClientCorp is at 45 days unpaid. want me to draft a friendly follow-up?"

Archive: "before your 2pm with Alex - last meeting you promised to review their API docs (you didn't)
and they mentioned considering migrating to Leptos"

Anchor: "Mom's birthday is next Tuesday. you usually panic-buy a gift Monday night.
maybe handle it this weekend while you have energy?"

Momentum: "you've got 3 social things scheduled this week. based on last month's pattern,
that's gonna wreck you. which one can we move?"
```

### 2. Shared Agent Tools

All agents can access these core functions:

```rust
pub trait SharedTools {
    // Any agent can pulse-check current state
    async fn check_vibe(&self) -> VibeState;

    // Capture current state for later recovery
    async fn context_snapshot(&self) -> String;

    // Search across all memory for patterns
    async fn find_pattern(&self, query: &str) -> Vec<Pattern>;

    // When current task/energy mismatch detected
    async fn suggest_pivot(&self) -> Suggestion;
}
```

### 3. Letta Agent Management ✅ PARTIALLY IMPLEMENTED

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
- [ ] Add MCP tools to send Discord messages from agents
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

### Low Priority
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
- [x] Design multi-agent system with Pattern as sleeptime orchestrator + 5 specialist agents
- [x] Implement shared memory blocks (current_state, active_context, bond_evolution)
- [x] Refactor multi-agent system to be more generic and flexible
- [x] Set up Discord bot integration with configurable agent routing
  - [x] Create Discord bot module (src/discord.rs)
  - [x] Implement slash commands and DM support
  - [x] Add dynamic agent routing based on configured agents
  - [x] Create discord_bot binary with configuration support
- [x] Refactor to single persistent binary with Discord + MCP features
  - [x] Create unified main.rs with feature flags
  - [x] Update MCP server to use new architecture
  - [x] Add background task support (sleeptime orchestrator)
- [x] Update MCP server to official SDK patterns
  - [x] Fix tool definitions using `#[rmcp::tool]` and `#[rmcp::tool_router]`
  - [x] Update to use `Parameters<T>` wrapper types
  - [x] Fix response types using `CallToolResult::success(vec![Content::text(...)])`
  - [x] Update error handling with specific constructors
  - [x] Document learnings in MCP_SDK_GUIDE.md

## Current Status

**Architecture**: Unified binary with feature flags
- Single `pattern` binary can run Discord bot, MCP server, and background tasks
- Feature flags: `discord`, `mcp`, `binary`, `full`
- Modular service architecture via PatternService

**Multi-Agent System** ✅:
- Generic, flexible architecture with configurable agents
- Shared memory blocks (current_state, active_context, bond_evolution)
- Background sleeptime orchestrator (30min intervals)
- Dynamic agent routing in Discord - no hardcoded names
- Actual agent message routing to Letta implemented

**Discord Bot** ✅:
- Natural language chat with slash commands
- DM support with agent routing (@agent, agent:, /agent)
- Configurable agent detection
- Message chunking for long responses

**MCP Server** ✅:
- Six tools: chat_with_agent, get/update_agent_memory, schedule_event, send_message, check_activity_state
- Stdio transport (streamable HTTP available when needed)
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

### Agent Message Routing (Immediate)
1. **Implement actual agent responses**
   - Route messages to specific Letta agents based on agent_id
   - Use agent-specific prompts from AgentConfig
   - Handle multi-turn conversations

2. **Discord ↔️ MCP Integration**
   - Implement `send_message` MCP tool to send Discord messages
   - Share Discord bot instance with MCP server
   - Enable agents to proactively reach out

### Task Management (High Priority)
1. **Extend database module** with task operations
   - Add CRUD methods for tasks
   - Implement task status transitions
   - Add task breakdown storage

2. **Create task manager module** (`src/tasks.rs`)
   - Task creation with ADHD-aware defaults
   - Task breakdown with Entropy agent
   - Time multiplication for realistic estimates

## Letta-rs API Notes

See [LETTA_API_REFERENCE.md](./LETTA_API_REFERENCE.md) for detailed API patterns and common gotchas discovered during implementation.

## References

- [MCP SDK Implementation Guide](./MCP_SDK_GUIDE.md) - Detailed patterns for using the official SDK
- [Official MCP Rust SDK](https://github.com/modelcontextprotocol/rust-sdk) - Use git version only
- [MCP Specification](https://modelcontextprotocol.io/specification/2025-06-18)
- [Letta Documentation](https://docs.letta.com/)
- [Letta API Reference](./LETTA_API_REFERENCE.md) - Common gotchas and patterns
- [Discord.py Interactions Guide](https://discordpy.readthedocs.io/en/stable/interactions/api.html) (concepts apply to serenity)
- [Activity Detection Research](https://dl.acm.org/doi/10.1145/3290605.3300589)
