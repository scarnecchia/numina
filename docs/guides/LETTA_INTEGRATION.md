# Letta Integration Guide

This document covers Pattern's integration with Letta for multi-agent functionality.

## Letta Agent Management

Pattern provides stateful agent management with caching:

```rust
// src/agent/constellation.rs - ACTUAL IMPLEMENTATION
pub struct MultiAgentSystem {
    letta: Arc<LettaClient>,
    db: Arc<Database>,
    config: AgentConfigs,
    model_config: ModelConfig,
}

impl MultiAgentSystem {
    pub async fn create_or_get_agent(&self, user_id: UserId, agent_id: AgentId) -> Result<Agent> {
        // Check DB cache first, then create if needed
        let config = self.get_agent_config(agent_id)?;
        let request = CreateAgentRequest::builder()
            .name(&agent_name)
            .memory_block(Block::system(&config.system_prompt))
            .memory_block(Block::persona(&config.persona))
            .memory_block(Block::human(&format!("User {}", user_id.0)))
            .build();

        // Store in DB cache
        Ok(agent)
    }

    // Shared memory using blocks API
    pub async fn update_shared_memory(&self, user_id: UserId, block_name: &str, content: &str) -> Result<()> {
        let block_id = self.get_or_create_memory_block(user_id, block_name).await?;
        self.letta.blocks().update(&block_id, UpdateBlockRequest { ... }).await?;
    }
}
```

## Key Learnings

- Letta uses `Block` types, not `ChatMemory`
- Memory updates require the blocks API
- `LettaId` is its own type, not just a String
- Message responses come as `LettaMessageUnion::AssistantMessage`

## Multi-Agent System with Letta

Pattern uses Letta's agent system with flexible coordination patterns:

1. **Agent Creation**: Each user gets their own constellation of agents
2. **Memory Hierarchy**: 
   - Core memory blocks (immediate access)
   - Letta sources (searchable knowledge)
   - Archival memory (deep storage)
3. **Native Groups API**: Create flexible agent groups with different coordination styles
4. **State Management**: Agent state is cached and persisted to SQLite

See [Memory and Groups Architecture](../architecture/MEMORY_AND_GROUPS.md) for details.

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

### Using Letta Groups for Multi-Agent Coordination

```rust
use letta_rs::types::groups::{GroupCreate, GroupCreateManagerConfig, DynamicManager};

// Create a flexible agent group
let group = client.groups().create(GroupCreate {
    agent_ids: vec![pattern_id, entropy_id, flux_id, momentum_id, anchor_id, archive_id],
    description: "ADHD support constellation".to_string(),
    manager_config: Some(GroupCreateManagerConfig::Dynamic(DynamicManager {
        manager_agent_id: pattern_id,
        termination_token: None,
        max_turns: None,
    })),
    shared_block_ids: Some(vec![current_state_id, active_context_id]),
}).await?;

// Send message to entire group
let response = client.groups().send_message(
    &group.id,
    vec![MessageCreate::user("I'm feeling overwhelmed with tasks")]
).await?;
```

### Sending Messages to Individual Agents

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

### Passive Knowledge Sharing via Sources

Agents can share insights through Letta's source documents:

```rust
// Agent writes observation to shared source
let observation = "User consistently underestimates task time by 3x";
client.sources().create(SourceCreate {
    name: "task_patterns.md".to_string(),
    content: observation,
    // Automatically embedded for semantic search
}).await?;

// Other agents can search for relevant patterns
let results = client.sources().search("time estimation patterns").await?;
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
- **Database caching**: Agent and group IDs stored in SQLite
- **Batch operations**: Fetch all agents once during initialization
- **Group caching** (2025-01-06): Full Group structs cached as JSON
- **Fast-path methods**: `get_agent_cached()`, `get_group_cached()` skip API calls
- **Future**: Migrate to foyer library for proper async disk-backed cache

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
- **FIXED**: Database caching now prevents repeated API calls for existing agents

### Memory Update Issues
- Use the blocks API, not direct memory updates
- Ensure block IDs are valid before updating
- Check block limits (default 2000 chars)

### Message Routing Problems
- Verify agent names match expected format
- Check Discord user ID mapping
- Ensure Letta client is properly authenticated
- **FIXED**: Tool name conflicts causing infinite loops (removed generic send_message)

## References

- [Letta API Reference](../api/LETTA_API_REFERENCE.md) - Common gotchas and patterns
- [Letta Documentation](https://docs.letta.com/)
- [Model Configuration Guide](../../LETTA_MODEL_CONFIG.md) - Performance optimization