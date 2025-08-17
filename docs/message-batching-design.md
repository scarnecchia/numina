# Message Batching Design

## Problem Statement

Currently, Pattern's message history can become inconsistent when:
1. Parallel tool executions complete out of order
2. Agents crash mid-execution leaving incomplete sequences
3. Context rebuilding encounters interleaved tool calls/results
4. Message compression breaks request/response boundaries
5. Continuation heartbeats create messages before tool responses are persisted
6. Backdated messages (like sleeptime interventions) disrupt ordering

## Proposed Solution: Implicit Batching via Snowflake IDs

Use snowflake IDs to create implicit batches without requiring a separate batch table. Messages track their batch membership and sequence directly.

## Design

### Core Structure Updates

```rust
// Add to Message struct
pub struct Message {
    pub id: MessageId,                      // Existing UUID-based ID
    pub snowflake_id: Option<SnowflakeId>,  // Unique ordering ID
    pub batch_id: Option<SnowflakeId>,      // ID of first message in batch
    pub sequence_num: Option<u32>,          // Position within batch
    pub batch_type: Option<BatchType>,      // Type of processing cycle
    // ... existing fields
}

pub enum BatchType {
    UserRequest,      // User-initiated interaction
    AgentToAgent,     // Inter-agent communication
    SystemTrigger,    // System-initiated (e.g., scheduled task)
    Continuation,     // Continuation of previous batch
}
```

### No Batch Table Required

Batches are reconstructed from messages at runtime:
- Messages with the same `batch_id` form a batch
- The batch is "complete" when we see a final assistant message without tool calls
- Batch metadata (type, timing) is inferred from the messages themselves

### Context Builder Changes

The context builder would:
1. Accept optional `current_batch_id` parameter to identify active processing
2. Query messages ordered by `snowflake_id` (or `created_at` as fallback)
3. Group consecutive messages by `batch_id`
4. Detect batch completeness by checking for:
   - Balanced tool calls/responses
   - Final assistant message without pending tools
   - No continuation markers
5. Include all complete batches
6. **Include the current active batch** (even if incomplete) - identified by matching `current_batch_id`
7. Drop all OTHER incomplete/orphaned batches

### Processing Flow

#### New Request Flow
1. User message arrives without `batch_id`
2. `process_message_stream` generates new `batch_id`
3. All messages in processing cycle use this `batch_id`
4. Sequence numbers increment for each message
5. Context builder receives `current_batch_id` to include incomplete batch

#### Continuation Flow
1. Heartbeat handler receives `HeartbeatRequest` with `batch_id`
2. Creates continuation message with same `batch_id`
3. Calls `process_message_stream(message)`
4. `process_message_stream` preserves existing `batch_id` from message
5. Continuation messages share batch with original request
6. Batch remains cohesive across async boundaries

#### Batch ID Propagation
```rust
// In process_message_stream
let batch_id = message.batch_id.unwrap_or_else(|| SnowflakeId::generate());

// In heartbeat handler
let continuation_message = Message {
    batch_id: Some(request.batch_id), // Preserve batch
    // ... other fields
};

// In context builder
pub fn build_context(&self, current_batch_id: Option<SnowflakeId>) -> Result<MemoryContext> {
    // Include current_batch_id batch even if incomplete
}
```

### Database Changes

```rust
// In agent_messages relation, sync position with snowflake_id
pub struct AgentMessageRelation {
    pub position: String,  // Now uses message.snowflake_id.to_string()
    // ... existing fields
}
```

No new tables needed - batching is entirely reconstructed from message metadata.

## Implementation Plan

### Phase 1: Core Infrastructure
1. Add `BatchType` enum to message module
2. Add batch tracking fields to `Message` struct:
   - `snowflake_id: Option<SnowflakeId>`
   - `batch_id: Option<SnowflakeId>`
   - `sequence_num: Option<u32>`
   - `batch_type: Option<BatchType>`
3. Create migration function to:
   - Generate snowflake_ids for existing messages (ordered by position)
   - Detect batch boundaries and assign batch_ids
   - Add sequence numbers within batches
   - Sync agent_messages.position with snowflake_id
4. Update message creation to generate snowflake_ids

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
