# Pattern Architecture - Multi-Agent Cognitive Support System

Pattern is a multi-agent cognitive support system designed specifically for ADHD brains. It uses Letta's multi-agent architecture with shared memory to provide external executive function through specialized cognitive agents inspired by Brandon Sanderson's Stormlight Archive.

## Agent Constellation

```
Pattern (Sleeptime Orchestrator)
├── Entropy (Task/Complexity Agent)
├── Flux (Time/Scheduling Agent)
├── Archive (Memory/Knowledge Agent)
├── Momentum (Flow/Energy Agent)
└── Anchor (Habits/Structure Agent)
```


## Core Features
- **Native Letta Groups**: Flexible agent coordination with overlapping groups
- **Three-Tier Memory**: Core blocks (immediate), Letta sources (searchable), archival (deep storage)
- **Cost-Optimized Sleeptime**: Two-tier monitoring with rules-based checks + AI intervention
- **Passive Knowledge Sharing**: Agents write insights to embedded documents for semantic search
- **ADHD-Specific Design**: Time blindness compensation, task breakdown, energy tracking, interruption awareness
- **Evolving Relationship**: Agents develop understanding of user patterns over time
- **MCP Server Interface**: Exposes agent capabilities through Model Context Protocol

For detailed architecture, see [Memory and Groups Architecture](./MEMORY_AND_GROUPS.md).

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

## Multi-Agent Architecture

Pattern uses Letta's native groups API for flexible agent coordination:

### Flexible Group Patterns
Groups are created dynamically based on needs:
- Different manager types (dynamic, supervisor, round-robin, sleeptime)
- Overlapping membership - same agents in multiple groups
- Context-specific coordination styles
- Experiment and evolve group configurations

### Memory Hierarchy
1. **Core Memory Blocks** (Always in context):
   - `current_state`: Real-time status
   - `active_context`: Recent important events
   - `bond_evolution`: Relationship dynamics

2. **Letta Sources** (Searchable knowledge):
   - Agent observations and insights
   - Pattern detection across time
   - Accumulated wisdom

3. **Archival Memory** (Deep storage):
   - Full conversation history
   - Important flagged moments

See [Memory and Groups Architecture](./MEMORY_AND_GROUPS.md) for implementation details.

## Legacy: Multi-Agent Shared Memory Architecture

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

## Agent Details

### Pattern (Sleeptime Orchestrator)
- Runs background checks every 20-30 minutes
- Monitors hyperfocus duration, physical needs, transitions
- Coordinates other agents based on current needs
- Personality: "friend who slides water onto your desk"

### Entropy (Task/Complexity Agent)
- Breaks down overwhelming tasks into atoms
- Recognizes hidden complexity in "simple" tasks
- Validates task paralysis as logical response
- Finds the ONE next action when everything feels impossible

### Flux (Time/Scheduling Agent)
- Translates between ADHD time and clock time
- Automatically adds buffers (1.5x-3x multipliers)
- Recognizes time blindness patterns
- Creates temporal anchors for transitions

### Archive (Memory/Knowledge Agent)
- External memory bank for dumped thoughts
- Surfaces relevant context without prompting
- Finds patterns across scattered data points
- Answers "what was I doing?" with actual context

### Momentum (Flow/Energy Agent)
- Distinguishes hyperfocus from burnout
- Maps energy patterns (Thursday 3pm crash, 2am clarity)
- Suggests task pivots based on current capacity
- Protects flow states when they emerge

### Anchor (Habits/Structure Agent)
- Tracks basics: meds, water, food, sleep
- Builds loose structure that actually works
- Celebrates basic self-care as real achievements
- Adapts routines to current capacity

## Work & Social Support Features

Pattern's agents provide specific support for contract work and social challenges:

### Contract/Client Management
- **Time Tracking**: Flux automatically tracks billable hours with "what was I doing?" recovery
- **Invoice Reminders**: Pattern notices unpaid invoices aging past 30/60/90 days
- **Follow-up Prompts**: "Hey, you haven't heard from ClientX in 3 weeks, might be time to check in"
- **Meeting Prep**: Archive surfaces relevant context before client calls
- **Project Switching**: Momentum helps context-switch between clients without losing state

### Social Memory & Support
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

## Shared Agent Tools

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

## Smart Calendar Management

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

## Context-Aware Interruptions

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

### Built-in Agent Tools

Pattern agents come with built-in tools for core functionality:

```rust
// Tool registration happens automatically
let builtin = BuiltinTools::default_for_agent(agent_handle);
builtin.register_all(&tool_registry);
```

**Standard Tools**:
- `update_memory`: Create/update persistent memory blocks
- `send_message`: Send to users, agents, groups, or channels
- `search_memory`: Semantic search across memory (planned)
- `schedule_reminder`: Time-based reminders (planned)

**Customization**:
```rust
// Replace with custom implementations
let builtin = BuiltinTools::builder()
    .with_memory_tool(RedisMemoryTool::new(redis))
    .build_for_agent(handle);
```

**AgentHandle Architecture**:
- Lightweight, cloneable access to agent internals
- Separates cheap data (ID, memory) from expensive (messages)
- Thread-safe with Arc<DashMap> for memory storage
