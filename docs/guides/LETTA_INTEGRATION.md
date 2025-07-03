# Letta Integration Guide

This document covers Pattern's integration with Letta for multi-agent functionality.

## Letta Agent Management

Pattern provides stateful agent management with caching:

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

## Key Learnings

- Letta uses `Block` types, not `ChatMemory`
- Memory updates require the blocks API
- `LettaId` is its own type, not just a String
- Message responses come as `LettaMessageUnion::AssistantMessage`

## Multi-Agent System with Letta

Pattern uses Letta's agent system to implement the multi-agent cognitive support:

1. **Agent Creation**: Each user gets their own constellation of 6 agents
2. **Shared Memory**: Agents share memory blocks for coordination
3. **Message Routing**: Discord messages are routed to appropriate agents
4. **State Management**: Agent state is cached and persisted to SQLite

## Common Patterns

### Creating Agents with Shared Memory

```rust
// Create agent with shared memory blocks
let request = CreateAgentRequest::builder()
    .name(&format!("pattern_{}_{}", user_id, agent_name))
    .memory_block(Block::persona(&agent_prompt))
    .memory_block(Block::human(&format!("User {} (Discord: {})", user_id, discord_username)))
    // Shared memory blocks
    .memory_block(Block::new("current_state", "", 200))
    .memory_block(Block::new("active_context", "", 400))
    .memory_block(Block::new("bond_evolution", "", 600))
    .build();
```

### Sending Messages to Agents

```rust
pub async fn send_message_to_agent(
    &self,
    user_id: UserId,
    agent_name: &str,
    message: &str,
) -> Result<String> {
    let agent_id = self.get_or_create_agent(user_id, agent_name).await?;
    
    let request = CreateMessagesRequest::builder()
        .messages(vec![MessageCreate::user(message)])
        .build();
        
    let response = self.letta.messages().create(&agent_id, request).await?;
    
    // Extract assistant response
    for msg in response.messages {
        if let LettaMessageUnion::AssistantMessage(m) = msg {
            return Ok(m.content);
        }
    }
}
```

## Performance Considerations

### Model Selection

The default `letta/letta-free` model has severe timeout issues. Use faster models:

```bash
export LETTA_MODEL="groq/llama-3.1-70b-versatile"
export LETTA_EMBEDDING_MODEL="letta/letta-free"
```

### Caching Strategy

- Agent instances are cached in memory after first creation
- Cache is checked before database or Letta API calls
- This significantly reduces latency for repeat interactions

### Connection Handling

```rust
// Retry logic for Letta connections
let mut retry_delay = Duration::from_secs(1);
for attempt in 1..=max_retries {
    match client.agents().list(None).await {
        Ok(_) => break,
        Err(e) if attempt < max_retries => {
            warn!("Letta connection attempt {} failed: {}", attempt, e);
            tokio::time::sleep(retry_delay).await;
            retry_delay *= 2; // Exponential backoff
        }
        Err(e) => return Err(e.into()),
    }
}
```

## Integration with MCP

Letta agents are exposed through MCP tools:

- `chat_with_agent`: Send messages to specific agents
- `get_agent_memory`: Retrieve agent memory blocks
- `update_agent_memory`: Update agent memory (via blocks API)

These tools allow external systems to interact with the Pattern agent constellation.

## Troubleshooting

### Agent Creation Timeouts
- Use faster models (Groq, Anthropic, etc.)
- Implement retry logic with exponential backoff
- Consider pre-creating agents during user initialization

### Memory Update Issues
- Use the blocks API, not direct memory updates
- Ensure block IDs are valid before updating
- Check block limits (default 2000 chars)

### Message Routing Problems
- Verify agent names match expected format
- Check Discord user ID mapping
- Ensure Letta client is properly authenticated

## References

- [Letta API Reference](../api/LETTA_API_REFERENCE.md) - Common gotchas and patterns
- [Letta Documentation](https://docs.letta.com/)
- [Model Configuration Guide](../../LETTA_MODEL_CONFIG.md) - Performance optimization