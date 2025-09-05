# Built-in Tools Architecture

This document describes the built-in tools system in Pattern, which provides standard agent capabilities following the Letta/MemGPT pattern for stateful memory management.

## Overview

Pattern agents come with a set of built-in tools that provide core functionality for memory management, archival storage, and communication. These tools are implemented using the same `AiTool` trait as external tools, ensuring consistency and allowing customization.

## Architecture

### AgentHandle

The `AgentHandle` is a lightweight, cheaply-cloneable struct that provides built-in tools with controlled access to agent internals:

```rust
#[derive(Clone)]
pub struct AgentHandle {
    pub agent_id: AgentId,
    pub memory: Memory,  // Memory uses Arc<DashMap> internally
    db: Option<Arc<SurrealEmbedded>>,  // Private DB access for archival operations
    // Future: message_sender for inter-agent communication
}
```

Built-in tools access the database through controlled methods on AgentHandle:
- `search_archival_memories()` - Full-text search with BM25
- `insert_archival_memory()` - Add new archival memories
- `delete_archival_memory()` - Remove archival memories
- `count_archival_memories()` - Get archival memory count

## Built-in Tools

### 1. Context Tool

Manages core memory blocks following the Letta/MemGPT pattern. Each operation modifies memory and requires the agent to continue their response.

**Operations:**
- `append` - Add content to existing memory (always uses \n separator)
- `replace` - Replace specific content within memory
- `archive` - Move a core memory block to archival storage
- `load` - Load an archival memory block into working/core
- `swap` - Atomic operation to archive one block and load another

```rust
// Example: Append to memory
{
    "operation": "append",
    "label": "human",
    "content": "Prefers morning meetings"
}

// Example: Swap memory blocks
{
    "operation": "swap",
    "archive_label": "old_project",
    "load_label": "new_project"
}
```

### 2. Recall Tool

Manages long-term archival storage with full-text search capabilities.

**Operations:**
- `insert` - Add new memories to archival storage
- `append` - Add content to existing archival memory
- `read` - Read specific archival memory by label
- `delete` - Remove archived memories

```rust
// Example: Insert new archival memory
{
    "operation": "insert",
    "label": "meeting_notes_2024_01",
    "content": "Discussed project timeline..."
}
```

### 3. Search Tool

Unified search interface across different domains.

**Domains:**
- `archival_memory` - Search archival storage
- `conversations` - Search message history
- `all` - Search everything

```rust
// Example: Search archival memory
{
    "domain": "archival_memory",
    "query": "project deadline",
    "limit": 10
}
```

### 4. Send Message Tool

Sends messages to the user (required for agents to yield control):

```rust
// Input
{
    "message": "I've updated your preferences. How else can I help?"
}

// Output
{
    "success": true,
    "message": "Message sent successfully"
}
```

## Registration and Customization

### Default Registration

```rust
// In DatabaseAgent::new()
let builtin = BuiltinTools::default_for_agent(context.handle());
builtin.register_all(&context.tools);
```

### Custom Tools

Users can replace built-in tools with custom implementations:

```rust
// Custom memory backend (e.g., Redis)
#[derive(Clone)]
struct RedisMemoryTool {
    handle: AgentHandle,
    redis: Arc<RedisClient>,
}

#[async_trait]
impl AiTool for RedisMemoryTool {
    type Input = UpdateMemoryInput;
    type Output = UpdateMemoryOutput;

    fn name(&self) -> &str { "update_memory" }

    async fn execute(&self, params: Self::Input) -> Result<Self::Output> {
        // Store in Redis instead of local memory
        self.redis.set(&params.label, &params.value).await?;
        // ...
    }
}

// Register custom tool
let builtin = BuiltinTools::builder()
    .with_memory_tool(RedisMemoryTool::new(redis_client))
    .build_for_agent(handle);
```

## Design Decisions

### Why Use the Same Tool System?

1. **Consistency**: All tools go through the same registry and execution path
2. **Discoverability**: Agents can list all available tools, including built-ins
3. **Testability**: Built-in tools can be tested like any other tool
4. **Flexibility**: Easy to override or extend built-in behavior

### Why AgentHandle?

1. **Performance**: Avoids multiple levels of Arc/Weak dereferencing
2. **Clarity**: Tools only access what they need
3. **Thread Safety**: Memory uses Arc<DashMap> for safe concurrent access
4. **Extensibility**: Easy to add new capabilities to the handle

### Why Not Special-Case Built-ins?

We considered having built-in tools as methods on the Agent trait or handled specially, but chose the unified approach because:

1. Users might want to disable or replace built-in tools
2. The tool registry provides a single source of truth
3. Special-casing would complicate the execution path
4. The performance overhead is minimal

## Implementation Details

### Type-Safe Tools

Built-in tools use the generic `AiTool` trait for type safety:

```rust
#[async_trait]
impl AiTool for UpdateMemoryTool {
    type Input = UpdateMemoryInput;   // Strongly typed, deserializable
    type Output = UpdateMemoryOutput; // Strongly typed, serializable

    async fn execute(&self, params: Self::Input) -> Result<Self::Output> {
        // Compile-time type checking
    }
}
```

### Dynamic Dispatch

The `DynamicToolAdapter` wraps typed tools for storage in the registry:

```rust
Box::new(DynamicToolAdapter::new(UpdateMemoryTool { handle }))
```

### MCP Compatibility

Tool schemas are generated with `inline_subschemas = true` to ensure no `$ref` fields, meeting MCP requirements.

## Future Extensions

### Planned Built-in Tools

1. **search_memory**: Semantic search across memory blocks
2. **append_memory**: Append to existing memory blocks
3. **replace_in_memory**: Find and replace in memory
4. **list_memories**: Get all memory block labels
5. **send_to_group**: Send message to agent group
6. **schedule_reminder**: Set time-based reminders
7. **track_task**: Create and track ADHD-friendly tasks

## Memory Permissions and Enforcement

Memory blocks carry a `permission` (enum `MemoryPermission`). New blocks default to `read_write` unless configured. Tools enforce an ACL as follows:

- Read: always allowed.
- Append: allowed for `append`/`read_write`/`admin`; `partner`/`human` require approval via PermissionBroker; `read_only` denied.
- Overwrite/Replace: allowed for `read_write`/`admin`; `partner`/`human` require approval; `append`/`read_only` denied.
- Delete: `admin` only.

Tool-specific notes:

- `context.append` and `context.replace` enforce ACL and request `MemoryEdit { key }` when needed.
- `context.archive` checks Overwrite ACL if the archival label already exists; deleting the source context requires Admin.
- `context.load` behavior:
  - Same label: convert archival → working in-memory.
  - Different label: create new working block and retain archival.
  - Does not delete archival.
- `context.swap` enforces Overwrite ACL on the destination (with possible approval) and deletes the source archival only with Admin.
- `recall.append` enforces ACL; `recall.delete` requires Admin.

Consent prompts are routed with origin metadata (e.g., Discord channel) for fast approval.

### MessageSender Integration

The `AgentHandle` will be extended with a message sender:

```rust
pub struct AgentHandle {
    pub agent_id: AgentId,
    pub memory: Memory,
    pub message_sender: Arc<dyn MessageSender>, // Future addition
}
```

This will enable inter-agent communication, group messages, and platform-specific routing.

## Usage Examples

### Basic Memory Update

```rust
let tool = registry.get("update_memory").unwrap();
let result = tool.execute(json!({
    "label": "preferences",
    "value": "User prefers dark mode",
    "description": "UI preferences"
})).await?;
```

### Custom Memory Tool with Logging

```rust
#[derive(Clone)]
struct LoggingMemoryTool {
    inner: UpdateMemoryTool,
    logger: Arc<Logger>,
}

#[async_trait]
impl AiTool for LoggingMemoryTool {
    type Input = UpdateMemoryInput;
    type Output = UpdateMemoryOutput;

    fn name(&self) -> &str { "update_memory" }

    async fn execute(&self, params: Self::Input) -> Result<Self::Output> {
        self.logger.info(&format!("Updating memory: {}", params.label));
        let result = self.inner.execute(params).await?;
        self.logger.info(&format!("Memory update result: {:?}", result));
        Ok(result)
    }
}
```

## Best Practices

1. **Keep tools focused**: Each tool should do one thing well
2. **Use type safety**: Define proper Input/Output types with JsonSchema
3. **Handle errors gracefully**: Return meaningful error messages
4. **Document tool behavior**: Provide clear descriptions and examples
5. **Consider concurrency**: Use Arc and thread-safe types appropriately
6. **Test thoroughly**: Built-in tools are critical infrastructure

## Testing

Built-in tools should be tested at multiple levels:

1. **Unit tests**: Test the tool in isolation
2. **Integration tests**: Test with real Memory and AgentHandle
3. **Registry tests**: Test registration and execution through the registry
4. **Agent tests**: Test tools in the context of agent operations

Example test:

```rust
#[tokio::test]
async fn test_update_memory_tool() {
    let memory = Memory::with_owner(UserId::generate());
    let handle = AgentHandle {
        agent_id: AgentId::generate(),
        memory: memory.clone(),
    };

    let tool = UpdateMemoryTool { handle };

    let result = tool.execute(UpdateMemoryInput {
        label: "test".to_string(),
        value: "test value".to_string(),
        description: None,
    }).await.unwrap();

    assert!(result.success);
    assert_eq!(memory.get_block("test").unwrap().content, "test value");
}
```
