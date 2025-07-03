# Letta-rs API Quick Reference

This document contains quick reference notes for the letta-rs crate API based on actual implementation experience.

## Key Types and Patterns

### Creating Agents

```rust
use letta::types::{CreateAgentRequest, Block};

// Use builder pattern with memory blocks
let request = CreateAgentRequest::builder()
    .name("agent_name")
    .memory_block(Block::persona("persona text"))  // NOT .memory(ChatMemory)!
    .memory_block(Block::human("human description"))
    .build();

let agent = client.agents().create(request).await?;
// agent.id is a LettaId, not String - use .to_string() when storing
```

### Sending Messages

```rust
use letta::types::{CreateMessagesRequest, MessageCreate};

// Build message request
let request = CreateMessagesRequest::builder()
    .messages(vec![MessageCreate::user("message")])  // NOT SendMessageRequest!
    .build();

// Send to agent
let response = client.messages().create(&agent_id, request).await?;
```

### Parsing Responses

```rust
use letta::types::LettaMessageUnion;

// Response messages are enum variants
for msg in response.messages {
    match msg {
        LettaMessageUnion::AssistantMessage(m) => {
            let content = m.content;  // NOT m.text!
        }
        LettaMessageUnion::SystemMessage(m) => {
            let content = m.content;
        }
        LettaMessageUnion::UserMessage(m) => {
            let content = m.content;
        }
        _ => {}
    }
}
```

### Memory Management

```rust
use letta::types::UpdateBlockRequest;

// No direct memory update API - use blocks instead
client.blocks().update(
    &block_id,  // Block must have an ID
    UpdateBlockRequest {
        value: Some(new_value),
        label: Some(new_label),
        limit: Some(2000),
        ..Default::default()
    }
).await?;

// Getting agent memory
let agent = client.agents().get(&agent_id).await?;
let memory = agent.memory.ok_or("No memory")?;
for block in memory.blocks {
    println!("{}: {}", block.label, block.value);
}
```

### Client Creation

```rust
// Cloud client
let client = LettaClient::cloud(api_key)?;  // Returns Result

// Local client with custom URL
let client = LettaClient::builder()
    .base_url(&url)  // NOT with_base_url()!
    .build()?;

// Local client with defaults
let client = LettaClient::local()?;  // No URL parameter!
```

### Working with LettaId

```rust
use letta::types::LettaId;

// Parse from string
let id: LettaId = "agent-550e8400-e29b-41d4-a716-446655440000".parse()?;

// Convert to string
let id_string = agent.id.to_string();

// LettaId implements Display
println!("Agent ID: {}", agent.id);
```

## Common Type Mappings

| What You Might Expect | What It Actually Is |
|-----------------------|---------------------|
| `ChatMemory` | `Block::persona()` + `Block::human()` |
| `SendMessageRequest` | `CreateMessagesRequest` |
| `message.text` | `message.content` |
| `client.send_message()` | `client.messages().create()` |
| `LettaClient::with_base_url()` | `LettaClient::builder().base_url()` |
| `agent_id: String` | `agent_id: LettaId` |

## API Method Locations

```rust
// Agent operations
client.agents().create(request)
client.agents().get(&agent_id)
client.agents().delete(&agent_id)
client.agents().list(params)

// Message operations
client.messages().create(&agent_id, request)
client.messages().list(&agent_id, params)

// Memory block operations
client.blocks().create(request)
client.blocks().update(&block_id, request)
client.blocks().get(&block_id)
client.blocks().list(params)

// Other APIs available
client.tools()
client.sources()
client.models()
client.providers()
```

## Common Gotchas

1. **LettaId is not a String** - Always use `.parse()` to convert from String or `.to_string()` to convert to String
2. **No ChatMemory type** - Use individual `Block` types with labels like "persona" and "human"
3. **No direct send_message method** - Use `messages().create()` with a `CreateMessagesRequest`
4. **Response messages are enums** - Use pattern matching on `LettaMessageUnion` variants
5. **Builder methods take &str** - Most builder methods want `&str` not `String`, use `&` when needed
6. **Client constructors return Result** - All client creation methods can fail, handle the Result
7. **Memory updates need block IDs** - Can't update memory directly, must update individual blocks by ID

## Error Handling

```rust
use letta::error::LettaError;
use miette::{miette, Result};

// Convert Letta errors to miette
match client.agents().create(request).await {
    Ok(agent) => Ok(agent),
    Err(e) => Err(miette!("Failed to create agent: {}", e))
}

// Or use map_err
let agent = client.agents().create(request).await
    .map_err(|e| miette!("Failed to create agent: {}", e))?;
```

## Useful Imports

```rust
// Common types you'll need
use letta::{
    types::{
        AgentMemory, Block, CreateAgentRequest, CreateMessagesRequest, 
        LettaId, MessageCreate, UpdateBlockRequest, LettaMessageUnion
    },
    LettaClient,
};
```