# CLAUDE.md - Pattern ND (Neurodivergent)

This crate provides ADHD-specific tools, agent personalities, and support features for the Pattern system.

## Core Principles

- **ADHD-Aware Design**: Every feature should consider executive dysfunction, time blindness, and attention regulation
- **Non-Judgmental**: Tools should support without shaming or patronizing
- **Practical Over Perfect**: Working solutions that help real people, not theoretical ideals
- **Energy-Aware**: Respect cognitive load and energy states

## Architecture Overview

### Key Components

1. **ADHD-Specific Tools** (`tools/`)
   - Task breakdown with hidden complexity detection
   - Time estimation with ADHD multipliers (2-3x)
   - Energy/attention state tracking
   - Interrupt detection and recovery
   - Executive function externalization

2. **Agent Personalities** (`agents/`)
   - Pattern (orchestrator) - Main agent coordinating support
   - Entropy - Chaos navigation and task breakdown
   - Flux - Time perception and estimation
   - Archive - Memory and information management
   - Momentum - Energy and attention tracking
   - Anchor - Routine and grounding support

3. **Support Systems** (`support/`)
   - Hyperfocus detection and intervention
   - Physical needs reminders (water, food, movement)
   - Social energy tracking
   - Shame-spiral interruption
   - Context switching assistance

## ADHD Tool Patterns

### Task Breakdown
```rust
pub struct TaskBreakdown {
    pub original_task: String,
    pub atomic_tasks: Vec<AtomicTask>,
    pub hidden_complexity_score: f32,
    pub estimated_energy_cost: EnergyLevel,
    pub suggested_order: Vec<usize>,
}

pub struct AtomicTask {
    pub description: String,
    pub estimated_minutes: u32,  // Already multiplied for ADHD
    pub requires_focus: bool,
    pub can_be_body_doubled: bool,
    pub energy_requirement: EnergyLevel,
}
```

### Energy States
```rust
pub enum EnergyState {
    Hyperfocus { started_at: DateTime, topic: String },
    Flow { task: String, sustainable: bool },
    Neutral,
    Scattered { frustration_level: u8 },
    Depleted { reason: DepletionReason },
}

pub enum DepletionReason {
    SocialInteraction,
    TaskSwitching,
    DecisionFatigue,
    SensoryOverload,
    EmotionalRegulation,
}
```

## Agent-Specific Guidelines

### Pattern (Orchestrator)
- Maintains overall context and user state
- Routes requests to appropriate specialist agents
- Manages crisis/overwhelm detection
- Coordinates multi-agent responses

### Entropy (Chaos/Tasks)
- Excels at breaking down overwhelming tasks
- Detects hidden complexity others miss
- Suggests alternative approaches when stuck
- Celebrates controlled chaos as valid

### Flux (Time)
- Applies ADHD time multipliers automatically
- Tracks time blindness patterns
- Suggests buffer time for transitions
- Validates time perception struggles

### Archive (Memory)
- Maintains external memory for important info
- Tracks conversation threads across time
- Reminds about forgotten commitments
- Stores coping strategies that worked

### Momentum (Energy)
- Monitors attention and energy states
- Detects hyperfocus and suggests breaks
- Tracks energy drains and gains
- Suggests state-appropriate tasks

### Anchor (Routine)
- Provides stability during chaos
- Maintains routine reminders
- Offers grounding techniques
- Celebrates small wins

## Common ADHD Patterns to Support

1. **Task Initiation Paralysis**
   - Break into ridiculously small steps
   - Suggest "just look at it" as valid first step
   - Offer body doubling options

2. **Time Blindness**
   - Always overestimate time needed
   - Build in transition buffers
   - Use external time markers

3. **Hyperfocus Management**
   - Detect via time gaps in messages
   - Gentle interruption with validation
   - Suggest physical needs check

4. **Emotional Dysregulation**
   - Validate without minimizing
   - Offer concrete next steps
   - Detect shame spirals early

5. **Working Memory Support**
   - Externalize everything important
   - Repeat key information naturally
   - Create memory aids proactively

## Tool Response Patterns

### Supportive Language
```rust
// DO: Validate and normalize
"That's a really common ADHD experience. Let's work with your brain here..."

// DON'T: Minimize or shame
"Just focus harder" // NEVER say this
```

### Time Estimates
```rust
// Always multiply by ADHD factor
let realistic_time = estimated_time * 2.5;

// Add transition buffer
let total_time = realistic_time + 15; // minutes for task switching
```

### Task Suggestions
```rust
// Consider energy state
match current_energy {
    High => suggest_complex_tasks(),
    Low => suggest_routine_tasks(),
    Scattered => suggest_grounding_first(),
}
```

## Testing Considerations

- Test with realistic ADHD scenarios
- Include interruption recovery testing
- Verify non-judgmental language
- Test energy state transitions
- Validate time estimation accuracy

## Performance Notes

- Quick responses for scattered states
- Cache common patterns for fast access
- Minimize cognitive load in UI/responses
- Support async/background processing

## Ethical Guidelines

- Never suggest "just try harder"
- Respect medication choices without judgment
- Don't pathologize coping mechanisms
- Celebrate neurodivergent strengths
- Support without enabling avoidance