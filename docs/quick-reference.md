# Pattern Quick Reference Guide

This guide provides a quick overview of Pattern's current state and key implementation details.

## Project Structure

```
pattern/
├── crates/
│   ├── pattern_core/      # Core agent framework
│   ├── pattern_nd/        # Neurodivergent tools
│   ├── pattern_mcp/       # MCP server
│   ├── pattern_discord/   # Discord bot
│   └── pattern_main/      # Main orchestrator
├── migrations/            # Database migrations
├── docs/                  # Documentation
└── pattern.toml          # Configuration
```

## Key Types & IDs

All IDs use underscore format: `prefix_uuid`

- `user_12345678-...` - User IDs
- `agent_12345678-...` - Agent IDs
- `task_12345678-...` - Task IDs
- `mem_12345678-...` - Memory IDs (note: "mem" not "memory")
- `group_12345678-...` - Group IDs

## Agent Architecture

### DatabaseAgent

```rust
// Generic over Connection, ModelProvider, and EmbeddingProvider
let agent = DatabaseAgent::<C, M, E>::new(
    agent_id,
    db,
    model_provider,
    "model-name",
    embedding_provider,
    tool_registry,
).await?;
```

### AgentContext Structure

```rust
AgentContext {
    handle: AgentHandle {        // Cheap, cloneable
        agent_id: AgentId,
        memory: Memory,          // Arc<DashMap> internally
    },
    tools: ToolRegistry,         // DashMap internally
    history: Arc<RwLock<MessageHistory>>,  // Large data
    metadata: Arc<RwLock<AgentContextMetadata>>,  // Stats
}
```

## Built-in Tools

### Registration

```rust
// Automatic registration
let builtin = BuiltinTools::default_for_agent(context.handle());
builtin.register_all(&context.tools);

// Custom tools
let builtin = BuiltinTools::builder()
    .with_memory_tool(CustomMemoryTool::new())
    .build_for_agent(handle);
```

### Available Tools

1. **update_memory**
   ```json
   {
     "label": "human",
     "value": "User's name is Alice",
     "description": "User information"
   }
   ```

2. **send_message** (stub)
   ```json
   {
     "target": { "type": "user" },
     "content": "Hello!",
     "metadata": {}
   }
   ```

## Memory System

```rust
// Memory uses Arc<DashMap> internally for thread safety
let memory = Memory::with_owner(user_id);
memory.create_block("persona", "I am a helpful assistant")?;
memory.update_block_value("persona", "I am Pattern")?;

// Clone is cheap - shares underlying storage
let memory_clone = memory.clone();
```

## Tool Development

### Type-Safe Tools

```rust
#[derive(Debug, Clone)]
struct MyTool;

#[async_trait]
impl AiTool for MyTool {
    type Input = MyInput;   // Must impl JsonSchema + Deserialize
    type Output = MyOutput; // Must impl JsonSchema + Serialize

    fn name(&self) -> &str { "my_tool" }
    fn description(&self) -> &str { "Does something" }

    async fn execute(&self, params: Self::Input) -> Result<Self::Output> {
        // Implementation
    }
}
```

### MCP Schema Compatibility

```rust
// Schemas are automatically inlined (no $ref)
let schema = tool.parameters_schema();
assert!(!schema.to_string().contains("$ref"));
```

## Database Operations

### SurrealDB Patterns

```rust
// Create with typed ID
let record: DbUser = db
    .create((UserIdType::PREFIX, id.uuid().to_string()))
    .content(data)
    .await?
    .unwrap_surreal_value()?;  // Helper for nested responses

// Query with proper escaping
let query = format!(
    "SELECT * FROM {} WHERE owner_id = {}:`{}`",
    MemoryIdType::PREFIX,
    UserIdType::PREFIX,
    user_id.uuid()
);

// Cross-agent memory sharing
agent.share_memory_with(
    "task_insights",
    other_agent_id,
    MemoryAccessLevel::Read,
).await?;
```

## Testing

### Mock Providers

```rust
// For testing agents
let model = Arc::new(RwLock::new(MockModelProvider {
    response: "Test response".to_string(),
}));

let embeddings = Arc::new(MockEmbeddingProvider::default());

let agent = DatabaseAgent::new(
    agent_id,
    db,
    model,
    "mock-model",
    Some(embeddings),
    tools,
).await?;
```

## Common Patterns

### Error Handling

```rust
// Use specific error types with context
CoreError::memory_not_found(&agent_id, &label, available_blocks)
CoreError::tool_not_found(&name, available_tools)
```

### Feature Flags

```toml
[dependencies]
pattern-core = { features = ["embed-candle", "embed-cloud"] }
```

Available features:
- `embed-candle` - Local embeddings with Jina models
- `embed-cloud` - OpenAI/Cohere embeddings
- `embed-ollama` - Ollama embeddings (stub)



## Key Commands

```bash
just               # List all commands
just pre-commit-all # Run all checks before committing
just test          # Run all tests
just fmt           # Format code
just watch         # Auto-recompile on changes
```

## Configuration

### pattern.toml

```toml
[database]
path = "data/surreal.db"  # SurrealDB embedded storage

[mcp]
transport = "sse"  # or "stdio", "http"
port = 8080

[discord]
token = "YOUR_TOKEN"
prefix = "!"

[models]
default = "claude-3-5-sonnet-latest"

[models.mapping] # need to re-add this with the new system
routine = "claude-3-5-haiku-latest"
interactive = "claude-3-5-sonnet-latest"

[embedding]
provider = "candle"  # or "openai", "cohere"
model = "jinaai/jina-embeddings-v2-small-en"
```

## Debugging Tips

1. **Check ID formats**: All IDs should be `prefix_uuid` format
2. **Memory is thread-safe**: Can clone and share across threads
3. **Tools must be Clone**: Required for registry storage
4. **Use MockProviders**: For testing without API calls
5. **Check feature flags**: Some functionality requires specific features
