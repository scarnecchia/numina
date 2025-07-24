# Pattern Memory Architecture & Multi-Agent Groups

This document describes Pattern's memory hierarchy, agent coordination via Letta groups, and background processing strategies.

## Memory Hierarchy

Pattern uses a three-tier memory system optimized for ADHD cognitive support:

### 1. Immediate Access (Core Memory Blocks)
Always visible in every agent's context window:
- **current_state**: Real-time status (what's happening RIGHT NOW)
- **active_context**: Recent important events and patterns
- **bond_evolution**: Relationship dynamics and trust building

These blocks are small (~2000 tokens each) and frequently updated.

### 2. Searchable Knowledge (Letta Sources)
Embedded documents accessible via semantic search:
- Agent observations and insights
- Partner behavior patterns over time
- Cross-agent discoveries
- Accumulated "learned wisdom"

Benefits:
- No API calls for reading shared insights
- Semantic search finds relevant patterns
- Agents discover insights without coordination overhead
- Creates persistent knowledge base

### 3. Deep Storage (Archival Memory)
Full conversation history and specific moments:
- Complete message history (via Letta's sliding window)
- Important moments flagged for recall
- Raw data for pattern analysis
- Accessible via `archival_memory_search` tool

## Context Window Management

Letta's context window structure:
```
[ALWAYS PRESENT - Fixed overhead]
1. System prompt
2. Tool descriptions
3. Summary (compressed older conversations)
4. Core memory blocks
5. Tool rules

[SLIDING WINDOW - Dynamic]
6. Recent messages (until token limit)

[ACCESSIBLE VIA RECALL]
7. Older messages (searchable but not in context)
```

Key insights:
- We don't manually manage conversation history
- Letta handles the sliding window automatically
- Core memory must contain only critical real-time state
- Archive agent compresses insights into summary

## Multi-Agent Groups

### Why Groups Matter
Instead of custom message routing, Letta's native groups API provides:
- Unified conversation history across all agents
- Multiple coordination strategies (dynamic, supervisor, round-robin)
- Shared memory blocks between agents
- Built-in sleeptime processing

### Example Group Configurations

These are examples to illustrate the flexibility of groups - your implementation can define whatever groups make sense:

#### Main Conversational Group
```rust
let main_group = client.groups().create(GroupCreate {
    agent_ids: vec![pattern_id, entropy_id, flux_id, momentum_id, anchor_id, archive_id],
    description: "Main ADHD support constellation".to_string(),
    manager_config: Some(GroupCreateManagerConfig::Dynamic(DynamicManager {
        manager_agent_id: pattern_id,
        termination_token: Some("DONE!".to_string()),
        max_turns: None,
    })),
    shared_block_ids: Some(vec![current_state_id, active_context_id, bond_evolution_id]),
}).await?;
```
- **Purpose**: Normal partner interactions
- **Manager**: Dynamic routing allows any agent to interject
- **Use case**: "I'm feeling overwhelmed" → all agents see it, relevant ones respond

#### Sleeptime Processing Group
```rust
let sleeptime_group = client.groups().create(GroupCreate {
    agent_ids: vec![pattern_id, archive_id],
    description: "Memory consolidation team".to_string(),
    manager_config: Some(GroupCreateManagerConfig::Sleeptime(SleeptimeManager {
        manager_agent_id: archive_id,
        sleeptime_agent_frequency: Some(20),  // every 20 messages
    })),
    shared_block_ids: Some(vec![current_state_id, active_context_id]),
}).await?;
```
- **Purpose**: Background memory processing
- **Manager**: Archive leads consolidation
- **Use case**: Compress conversations, update shared insights

#### Crisis Response Group
```rust
let crisis_group = client.groups().create(GroupCreate {
    agent_ids: vec![pattern_id, momentum_id, anchor_id],
    description: "Urgent intervention team".to_string(),
    manager_config: Some(GroupCreateManagerConfig::RoundRobin(RoundRobinManager {
        max_turns: Some(10),
    })),
    shared_block_ids: Some(vec![current_state_id]),
}).await?;
```
- **Purpose**: Quick intervention for spiraling/crisis
- **Manager**: Round-robin for rapid checks
- **Use case**: "Help I'm spiraling" → quick focused response

#### Planning Group
```rust
let planning_group = client.groups().create(GroupCreate {
    agent_ids: vec![entropy_id, flux_id, pattern_id],
    description: "Task planning specialists".to_string(),
    manager_config: Some(GroupCreateManagerConfig::Supervisor(SupervisorManager {
        manager_agent_id: entropy_id,
    })),
    shared_block_ids: Some(vec![current_state_id, active_context_id]),
}).await?;
```
- **Purpose**: Dedicated planning sessions
- **Manager**: Entropy leads task breakdown
- **Use case**: "Let's plan my day" → structured planning

### Group Benefits
- Same agents, different coordination styles
- Overlapping groups for different contexts
- Shared conversation history within groups
- No manual message routing needed
- **Completely flexible** - define groups that make sense for your use case

## Overlapping Groups Architecture

A key insight: since groups only reference existing agent IDs, we can create multiple overlapping groups with different configurations for different contexts.

### Benefits of Overlapping Groups

1. **Context-Specific Coordination**
   - Normal conversation uses dynamic routing (anyone can jump in)
   - Crisis moments use round-robin (quick systematic checks)
   - Planning uses supervisor mode (Entropy leads structured breakdown)

2. **Flexible Agent Participation**
   - Not all agents needed for all contexts
   - Crisis group: just Pattern, Momentum, Anchor (immediate needs)
   - Planning group: just Entropy, Flux, Pattern (task/time focus)
   - Sleeptime: just Pattern, Archive (memory processing)

3. **Cost Optimization**
   - Sleeptime groups can use cheaper models
   - Crisis groups can use faster models
   - Planning groups can use more analytical models

### Example Usage Patterns

```rust
// Normal conversation
let response = client.groups().send_message(
    &main_group.id,
    vec![MessageCreate::user("I'm feeling scattered today")]
).await?;
// All agents see it, dynamic routing determines who responds

// Crisis intervention
if detect_crisis(&message) {
    let response = client.groups().send_message(
        &crisis_group.id,
        vec![MessageCreate::user("Help I'm spiraling")]
    ).await?;
    // Only Pattern, Momentum, Anchor respond with quick checks
}

// Dedicated planning session
if message.contains("plan") || message.contains("organize") {
    let response = client.groups().send_message(
        &planning_group.id,
        vec![MessageCreate::user("Let's plan out my week")]
    ).await?;
    // Entropy leads, Flux provides time reality checks
}

// Background processing (automatic)
// Every 20 messages, sleeptime group activates
// Archive processes conversation history with Pattern
```

### Group Selection Logic

The Discord bot or MCP server can intelligently route to appropriate groups:

```rust
fn select_group(message: &str, user_state: &UserState) -> GroupId {
    if is_crisis_language(message) || user_state.stress_level > 8 {
        return crisis_group.id;
    }

    if is_planning_request(message) {
        return planning_group.id;
    }

    if is_memory_question(message) {
        return memory_group.id;  // Archive-focused group
    }

    // Default to main conversational group
    main_group.id
}
```

### The Power of Flexibility

The key insight is that groups are just references to existing agents. You can:
- Create groups on the fly based on context
- Experiment with different manager types
- Add/remove agents from groups dynamically
- Have one agent in many groups simultaneously
- Create special-purpose groups for specific workflows

This isn't a fixed architecture - it's a toolkit for building whatever coordination patterns emerge as useful.

## Custom Tiered Sleeptime Architecture

Pattern also implements a custom tiered approach for cost optimization:

### Tier 1: Lightweight Monitor (Every 20min)
```rust
// Cheap rules-based or tiny model checks
async fn quick_check() {
    // Activity detection (are they at computer?)
    // Time since last water/movement
    // Current task duration

    if concerning_pattern_detected() {
        wake_pattern();  // Trigger expensive model
    }
}
```

**Triggers for waking Pattern:**
- Hyperfocus >90min detected
- No movement >2hrs
- Task switch detected
- User explicitly asks

### Tier 2: Pattern Intervention (5-10x/day)
When triggered, Pattern (expensive model) performs:
- Comprehensive state assessment
- Delegate to specialist agents
- Update shared memory
- Send Discord notifications if needed

### Cost Optimization
1. **Two-tier models**:
   - Llama 3.1 8B for routine monitoring
   - Claude/GPT-4 for complex interventions
2. **Conditional awakening**: Pattern sleeps unless triggered
3. **Batch processing**: Accumulate observations, process together
4. **Shared context**: One expensive analysis, all agents read results

## Memory Processing Strategy

### Hybrid Approach
1. **Archive handles bulk processing**:
   - Conversation summarization
   - Cross-agent pattern detection
   - Long-term memory consolidation
   - Runs as sleeptime agent every ~20 conversations

2. **Specialists update domain insights**:
   - Each agent maintains "learned_[domain]" memory block
   - Updates after significant interactions
   - Archive reads these for meta-patterns

3. **Pattern performs meta-synthesis**:
   - During sleeptime checks, reads all memory blocks
   - Identifies cross-cutting concerns
   - Updates strategic priorities

### Memory Flow
```
Partner says something
    ↓
Core memory (immediate context)
    ↓
Agents process & observe
    ↓
Write insights to sources (passive sharing)
    ↓
Archive consolidates patterns (sleeptime)
    ↓
Updates core memory with key insights
```

## Passive Data Sharing via Sources

Each agent can write observations to shared source files:

```rust
// Agent tool for passive sharing
fn write_to_shared_insights(category: &str, content: &str) {
    // Writes to markdown file
    // Automatically embedded by Letta
    // Available to all agents via search_file
}
```

### Example Source Files

**sleeptime_observations.md**:
```markdown
[2025-07-04 14:30] Hyperfocus detected: coding session 2.5hrs
[2025-07-04 14:30] Physical needs: last water 90min ago
[2025-07-04 14:30] Energy state: flow but approaching burnout risk
```

**task_patterns.md** (by Entropy):
```markdown
- "quick fix" tasks average 3.2x estimated time
- Morning tasks have 80% completion rate
- Tasks with >5 subtasks rarely completed same day
```

**energy_patterns.md** (by Momentum):
```markdown
- Post-lunch dip consistent 2-4pm
- Creative tasks best 10pm-2am
- Meetings cost 2x recovery time on low-energy days
```

## Implementation Priority

1. **Rebuild with new backends**
2. **Implement shared source writing** (passive knowledge sharing)
3. **Create overlapping group configurations** (context-specific coordination)
4. **Build lightweight monitor** (tier 1 sleeptime)
5. **Configure Archive as sleeptime processor** (memory consolidation)

This architecture balances:
- Continuous ADHD support
- API cost efficiency
- Shared knowledge between agents
- Flexible coordination strategies
