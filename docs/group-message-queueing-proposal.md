# Group Message Queueing Proposal

**Status**: DRAFT/PROPOSAL - Not yet implemented
**Date**: 2025-07-30
**Context**: Refactoring send_to_group to use proper coordination managers

## Problem Statement

Currently, `MessageRouter::send_to_group()` implements its own basic routing logic for each coordination pattern, duplicating what the `GroupManager` implementations already do properly. This leads to:

- Code duplication between router and managers
- Inconsistent behavior between direct group chat and send_message tool
- No support for advanced coordinator features (state management, streaming, etc.)
- Difficulty maintaining pattern-specific logic in two places

## Proposed Solution

Transform coordinators into pure state machines that emit messages into the database queue, removing all direct agent process_message calls. This creates a fully async, resilient architecture where agents independently process from their queues.

### Architecture Overview

```
Current Flow:
Agent -> send_message tool -> MessageRouter.send_to_group() -> Direct routing (bad)
GroupManager -> Directly calls agent.process_message() -> Synchronous processing

Proposed Flow:
Agent -> send_message tool -> MessageRouter.send_to_group() -> Queue group message
GroupMessageProcessor -> Dequeue -> GroupManager (as state machine) -> Queue individual messages
Agents -> Process from their own queues independently -> All output via send_message tool
GroupManager -> Emits stream events for monitoring only
```

### Core Architectural Principles

1. **No Direct Agent Calls**: Coordinators never call `agent.process_message()` directly
2. **Message Queue as Single Source**: All agent work comes from their message queue
3. **Agents are Independent**: Can be in multiple groups, process at their own pace
4. **State Machine Coordinators**: Coordinators only decide what messages to queue next
5. **Monitoring vs Output**: Streams are for monitoring; actual output goes through message router
6. **No Direct CLI Output**: Agents should use send_message tool, not write directly to CLI

### Key Components

#### 1. Enhanced QueuedMessage Schema

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum QueuedMessageType {
    AgentToAgent {
        content: String,
        metadata: Option<Value>,
        origin: Option<MessageOrigin>,
    },
    GroupMessage {
        group_id: GroupId,
        content: String,
        metadata: Option<Value>,
        origin: Option<MessageOrigin>,
        // Track coordinator state?
        coordinator_state: Option<Value>,
    },
}

pub struct QueuedMessage {
    pub id: MessageId,
    pub from_agent_id: AgentId,
    pub message_type: QueuedMessageType,
    pub call_chain: Vec<AgentId>,
    pub created_at: DateTime<Utc>,
    pub processed_at: Option<DateTime<Utc>>,
}
```

#### 2. Group Message Processor

A background task that:
1. Polls for unprocessed group messages
2. Loads the group and its members as full Agent instances
3. Instantiates the appropriate GroupManager
4. Calls route_message and processes the response stream
5. Handles response routing and state updates

```rust
pub struct GroupMessageProcessor {
    db: DatabaseConnection,
    context: Arc<Context>, // For creating agents
    managers: HashMap<String, Arc<dyn GroupManager>>,
}
```

### Benefits

1. **Single source of truth**: Coordination logic lives only in managers
2. **Full feature support**: State management, streaming, all patterns work correctly
3. **True async processing**: Agents work independently from their queues
4. **Natural resilience**: Offline agents just accumulate messages
5. **Debuggability**: Can inspect queue state, see what's pending/processed
6. **Proper parallelism**: Agents process concurrently without blocking
7. **Clean separation**: Coordinators coordinate, agents process, routers route
8. **Consistent output**: All agent output goes through message router
9. **State persistence**: Each step has clear DB state that can be persisted
10. **Agent flexibility**: Agents can be in multiple groups with different roles

### Open Questions

1. **Response Routing**
   - When agents in a group respond, where do those responses go?
   - Options:
     - Back to original sender only
     - To a group conversation/channel concept
     - Broadcast to all group members
     - Pattern-specific routing (e.g., Pipeline passes to next stage)

2. **Agent Instance Loading**
   - Need full DatabaseAgent instances, not just records
   - Requires access to model/embedding providers
   - Options:
     - Pass Context to processor
     - Create lightweight "processing" agents
     - Cache agent instances

3. **State Persistence**
   - Some patterns need state updates (RoundRobin index, Voting sessions)
   - Options:
     - Auto-persist after each message
     - Let coordinators handle it
     - Batch updates periodically

4. **Error Handling**
   - What if processing fails mid-stream?
   - Options:
     - Mark message as failed, retry later
     - Dead letter queue
     - Partial success tracking

5. **Service Ownership**
   - Where does GroupMessageProcessor live?
   - Options:
     - Part of Context (started with each agent)
     - Separate service (one per constellation)
     - Started by CLI/server binary

6. **Message Ordering**
   - How to ensure FIFO for group messages?
   - What about concurrent group messages?

7. **Performance Considerations**
   - Polling frequency
   - Batch processing
   - Stream timeout handling

### Implementation Phases

1. **Phase 1**: Extend QueuedMessage schema
2. **Phase 2**: Create basic processor that loads groups
3. **Phase 3**: Integrate with managers
4. **Phase 4**: Handle response routing
5. **Phase 5**: Add state persistence
6. **Phase 6**: Production hardening (errors, monitoring, etc.)

### Alternative Approaches Considered

1. **Direct streaming from send_message**: Would block the tool call
2. **Synchronous processing**: Would couple agent processing with group coordination
3. **Webhook-style callbacks**: Too complex for current architecture

### Next Steps

1. Let this proposal sit for a bit
2. Continue with other features to gain more context
3. Revisit with fresh perspective
4. Prototype minimal version to validate approach

### Notes

This is a significant architectural refactor that transforms the entire coordination system from synchronous direct calls to async message-based processing. While it's a bigger change, it aligns with the overall Pattern architecture of agents as independent entities that communicate through messages. The current direct-call approach was simpler to implement initially but this message-queue approach is the right long-term architecture for a resilient, debuggable, distributed agent system.