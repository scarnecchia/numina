# Message Batching Design

## Problem Statement

Currently, Pattern's message history can become inconsistent when:
1. Parallel tool executions complete out of order
2. Agents crash mid-execution leaving incomplete sequences
3. Context rebuilding encounters interleaved tool calls/results
4. Message compression breaks request/response boundaries

## Proposed Solution: Request Batching

Introduce a `MessageBatch` concept at the context level that groups related messages into atomic request/response cycles.

## Design

### Core Structure

```rust
// In pattern_core/src/context/batch.rs
pub struct MessageBatch {
    pub id: SnowflakeId,           // Batch identifier
    pub started_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
    pub messages: Vec<Message>,     // Ordered messages in this batch
    pub is_complete: bool,
    pub agent_id: AgentId,
    pub batch_type: BatchType,
}

pub enum BatchType {
    UserRequest,      // User-initiated interaction
    AgentToAgent,     // Inter-agent communication
    SystemTrigger,    // System-initiated (e.g., scheduled task)
    Continuation,     // Continuation of previous batch (for long responses)
}
```

### Message Updates

Add batch tracking to the Message struct:

```rust
pub struct Message {
    pub id: MessageId,                    // Already a snowflake
    pub batch_id: Option<SnowflakeId>,    // Which batch this belongs to
    pub sequence_num: Option<u32>,        // Order within the batch
    // ... existing fields
}
```

### Context Builder Changes

The context builder would:
1. Query messages WITH their snowflake IDs for absolute ordering
2. Group messages by `batch_id`
3. Sort batches chronologically by batch ID
4. Within each batch, sort by `sequence_num`
5. **Only include complete batches** (where `is_complete = true`)
6. Drop incomplete batches entirely from context

### Database Schema

```sql
-- New batch table
DEFINE TABLE batch SCHEMAFULL;
DEFINE FIELD id ON batch TYPE string;
DEFINE FIELD started_at ON batch TYPE datetime;
DEFINE FIELD completed_at ON batch TYPE option<datetime>;
DEFINE FIELD is_complete ON batch TYPE bool DEFAULT false;
DEFINE FIELD agent_id ON batch TYPE string;
DEFINE FIELD batch_type ON batch TYPE string;

-- Relation: batch contains messages
DEFINE TABLE batch_messages SCHEMAFULL;
DEFINE FIELD in ON batch_messages TYPE record(batch);
DEFINE FIELD out ON batch_messages TYPE record(message);
DEFINE FIELD sequence_num ON batch_messages TYPE int;
```

## Implementation Plan

### Phase 1: Core Infrastructure
1. Create `MessageBatch` struct and `BatchType` enum
2. Add batch tracking fields to `Message`
3. Create database migration for batch table and relations
4. Add batch creation/completion methods to DatabaseAgent

### Phase 2: Message Handling
1. Modify `process_message` to create a new batch for each request
2. Track all messages (user, assistant, tool calls, tool results) in batch
3. Mark batch complete when assistant sends final response
4. Handle batch continuation for multi-turn responses

### Phase 3: Context Building
1. Update context builder to work with batches
2. Implement batch completeness checking
3. Add batch-aware message ordering
4. Test with parallel tool execution scenarios

### Phase 4: Compression & Cleanup
1. Update compression to respect batch boundaries
2. Add batch-based retention policies
3. Implement batch export/import for migrations

## Benefits

1. **Atomicity**: Either a full request/response cycle is in context or nothing
2. **Parallel Safety**: Tool calls within a batch can execute in parallel without ordering issues
3. **Crash Recovery**: Incomplete batches are automatically excluded
4. **Cleaner Context**: No more orphaned tool results or half-completed thoughts
5. **Better Compression**: Can compress complete batches as units
6. **Debugging**: Easier to trace through conversation history

## Edge Cases to Handle

1. **Long-running operations**: Batch timeout mechanism
2. **Agent crashes**: Automatic batch invalidation after timeout
3. **Multi-agent coordination**: Nested batches for group conversations
4. **Streaming responses**: Progressive batch updates with temporary completeness
5. **System messages**: Special batch type for system-initiated messages

## Migration Strategy

1. Add batch support alongside existing system (backward compatible)
2. New messages get batch IDs, old messages work as-is
3. Gradually migrate old messages to batches based on timestamps
4. Eventually make batch_id required for all new messages

## Success Metrics

- No more context corruption from incomplete tool sequences
- Parallel tool execution without ordering issues  
- Reduced context rebuilding errors
- Cleaner conversation exports
- Improved debugging capability through batch inspection