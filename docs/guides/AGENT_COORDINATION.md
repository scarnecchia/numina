# Agent Coordination & Loop Prevention Guide

This guide documents the challenges with multi-agent coordination in Pattern and our solutions.

**Note**: Many coordination challenges are solved by using Letta's native groups API. See [Memory and Groups Architecture](../architecture/MEMORY_AND_GROUPS.md) for the recommended approach.

## The Problem

Pattern uses Letta's multi-agent system where agents can communicate via the `send_message_to_agent` tool. However, this creates several critical issues:

### 1. Message Type Confusion
- **Issue**: System messages from tools appear as user messages to recipient agents
- **Example**: When Pattern sends a message to Archive, Archive sees it as a user request
- **Impact**: Agents respond to system notifications, creating loops

### 2. Tool Result Echo Problem
- **Issue**: Agents receive their own tool results as new messages
- **Example**: "Message sent successfully" appears as user input
- **Impact**: Agents respond to their own success confirmations

### 3. The Pattern Emoji Explosion Incident
Real incident from 2025-01-03:
1. User asked Pattern to express itself with emojis
2. Pattern used `send_discord_message` tool with "🌊✨🎭"
3. Tool returned "Message sent to channel"
4. Pattern saw this as encouragement and sent more emojis
5. Created infinite loop of increasingly enthusiastic emojis
6. Only stopped when user said "STOP"

### 4. Broadcast Message Loops
- **Issue**: Messages sent to all agents trigger responses from everyone
- **Impact**: Exponential message growth as agents respond to each other

## Root Cause

Letta's message handling doesn't distinguish message sources. All messages appear as user messages in the agent's context, regardless of origin:
- User messages
- Agent-to-agent messages
- Tool success/failure notifications
- System messages

## Letta's Built-in Multi-Agent Tools

Letta provides three multi-agent communication tools:

1. **`send_message_to_agent_async`**: Send without waiting for reply
2. **`send_message_to_agent_and_wait_for_reply`**: Send and wait for response
3. **`send_message_to_agents_matching_all_tags`**: Broadcast to tagged agents

The broadcast tool uses agent tags for filtering, which we should leverage.

## Solutions Implemented

### 1. Use Letta Groups API (Recommended)

The best solution is to use Letta's native groups API instead of broadcast messages:

```rust
// Create a group for coordinated responses
let group = client.groups().create(GroupCreate {
    agent_ids: vec![pattern_id, entropy_id, flux_id],
    manager_config: Some(GroupCreateManagerConfig::Dynamic(DynamicManager {
        manager_agent_id: pattern_id,
        termination_token: None,
        max_turns: None,
    })),
    shared_block_ids: Some(vec![shared_memory_id]),
}).await?;

// Send message to group - agents coordinate naturally
client.groups().send_message(&group.id, vec![MessageCreate::user("...")]).await?;
```

Benefits:
- Shared conversation history
- No broadcast loops
- Built-in coordination patterns
- Flexible manager types

### 2. Agent Tagging Strategy (Legacy)

If using individual agents, leverage Letta's tagging:

```rust
// Agent creation with tags
CreateAgentRequest::builder()
    .name("pattern_12345")
    .tags(vec!["orchestrator", "sleeptime", "user_12345"])
    
CreateAgentRequest::builder()
    .name("entropy_12345")
    .tags(vec!["specialist", "tasks", "user_12345"])
```

### 2. Prompt Engineering (Immediate Fix)

Updated base system prompt with inter-agent communication rules:

```
CRITICAL Inter-Agent Communication Rules:
- When you receive "Message sent successfully" or similar tool confirmations, DO NOT RESPOND
- Tool success/failure messages are system notifications, not conversation
- Only respond to actual user messages or agent messages that ask questions
- If another agent sends you information without a question, acknowledge internally but don't reply
- Use send_message_to_agent ONLY when you need specific information or action from another agent
- Shared memory (current_state, active_context, bond_evolution) is preferred for coordination
```

### 2. Message Tagging System (coordination.rs)

Created a proper message tagging system to distinguish sources:

```rust
enum MessageSource {
    User(UserId),
    Agent(AgentId),
    System,
    Tool { name: String },
}

struct TaggedMessage {
    content: String,
    source: MessageSource,
    is_broadcast: bool,
    requires_response: bool,
}
```

Message formatting includes metadata:
- `[USER 42] Hello`
- `[AGENT pattern - NEEDS RESPONSE] What's the user's energy level?`
- `[AGENT archive - INFO ONLY] Stored memory update`
- `[SYSTEM] Configuration updated`
- `[TOOL send_message RESULT] Success`

### 3. Response Rules

Agents should only respond when:
- Message is from a user (always respond)
- Message is from another agent AND requires_response=true
- Message is NOT from themselves
- Message is NOT a system notification
- Message is NOT a tool result

### 4. Shared Memory Preference

For coordination without message loops, agents should prefer shared memory blocks:
- `current_state`: Real-time status updates
- `active_context`: Current activity tracking
- `bond_evolution`: Relationship development

## Best Practices

### 1. Agent Communication Patterns

**Good**: Information sharing without expecting response
```
Archive → Pattern: "[AGENT archive - INFO ONLY] User mentioned they have a deadline tomorrow"
```

**Good**: Specific question requiring response
```
Pattern → Flux: "[AGENT pattern - NEEDS RESPONSE] How much time should we allocate for this task?"
```

**Bad**: Vague broadcasts that trigger responses
```
Pattern → All: "User seems tired" (every agent will respond)
```

### 2. Tool Usage Guidelines

- Only use `send_message_to_agent` when you need specific information
- Always format messages to indicate if response is needed
- Never respond to tool confirmations
- Filter tool responses from message history

### 3. Coordination Through Shared Memory

Instead of messaging, update shared memory:
```rust
// Instead of: send_message_to_agent("Pattern", "User energy is low")
update_shared_memory("current_state", "energy: 3/10 | attention: scattered")
```

All agents see the update without triggering responses.

## Implementation Status

- ✅ Prompt engineering rules added to all agents
- ✅ Message tagging system implemented in coordination.rs
- ✅ Agent coordinator with broadcast and filtering utilities
- ⏳ Integration with MultiAgentSystem pending
- ⏳ Testing in production environment needed

## Future Improvements

1. **Letta Integration**: Work with Letta team to add native message source distinction
2. **Tool Rules**: Use Letta's tool rules to enforce communication patterns
3. **Message History Filtering**: Automatically filter system messages from context
4. **Coordination Metrics**: Track message volumes to detect loops early

## Testing Checklist

- [ ] Agents ignore "Message sent successfully" notifications
- [ ] Broadcast messages don't create response loops
- [ ] Agent questions get appropriate responses
- [ ] Info-only messages don't trigger responses
- [ ] Shared memory updates work without messaging
- [ ] Discord message tools don't create loops

## References

- [Letta Multi-Agent Guide](https://docs.letta.com/guides/agents/multi-agent)
- [Letta Tool Rules](https://docs.letta.com/guides/agents/tool-rules)
- [Discord Issues Documentation](../troubleshooting/DISCORD_ISSUES.md)