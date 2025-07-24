# Pattern Tool System Development

Pattern's tool system enables agents to perform actions through a type-safe, extensible framework. Tools are functions that agents can call during conversations to interact with memory, send messages, search data, or perform custom operations.

## Core Components

### 1. The `AiTool` Trait

```rust
#[async_trait]
pub trait AiTool: Send + Sync {
    type Input: JsonSchema + DeserializeOwned + Serialize + Send;
    type Output: JsonSchema + Serialize + Send;

    fn name(&self) -> &str;
    fn description(&self) -> &str;

    async fn execute(&self, params: Self::Input) -> Result<Self::Output>;

    // Optional: Usage rules for context
    fn usage_rule(&self) -> Option<String> { None }
}
```

This trait provides compile-time type safety for tool inputs and outputs.

### 2. The `DynamicTool` Trait

```rust
#[async_trait]
pub trait DynamicTool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn input_schema(&self) -> Value;
    fn usage_rule(&self) -> Option<String>;

    async fn execute_dynamic(&self, params: Value) -> Result<Value>;
}
```

This trait enables dynamic dispatch and runtime tool discovery.

### 3. The Bridge: `impl<T: AiTool> DynamicTool for T`

Pattern automatically implements `DynamicTool` for any type that implements `AiTool`. This bridge:
- Converts typed inputs/outputs to/from JSON
- Generates JSON schemas from Rust types
- Preserves type safety while enabling dynamic dispatch

## Flow Diagram

```mermaid
flowchart TB
    subgraph row1 [" "]
        subgraph dev ["Development"]
            A[Custom Tool] --> B[impl AiTool]
            B --> C[Auto impl<br/>DynamicTool]
        end

        subgraph reg ["Registration"]
            D[Box tool] --> E[Registry]
            E --> F[DashMap]
        end
    end

    subgraph row2 [" "]
        subgraph run1 ["Discovery"]
            G[Agent] --> H[Get schemas]
            H --> I[Send to LLM]
        end

        subgraph run2 ["Execution"]
            J[LLM call] --> K[JSON params]
            K --> L[Find tool]
            L --> M[Convert types]
            M --> N[Execute]
            N --> O[Return JSON]
        end
    end

    dev -.-> reg
    reg ==> run1
    run1 -.-> run2

    style A fill:#4299e1,stroke:#2b6cb0,color:#fff
    style J fill:#ed8936,stroke:#c05621,color:#fff
    style O fill:#48bb78,stroke:#2f855a,color:#fff
```

## Creating a Custom Tool

### Step 1: Define Your Input/Output Types

```rust
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
struct WeatherInput {
    city: String,
    #[serde(default)]
    units: String,  // "metric" or "imperial"
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
struct WeatherOutput {
    temperature: f32,
    description: String,
    humidity: u8,
}
```

### Step 2: Implement the Tool

```rust
#[derive(Debug, Clone)]
struct WeatherTool {
    api_key: String,
}

#[async_trait]
impl AiTool for WeatherTool {
    type Input = WeatherInput;
    type Output = WeatherOutput;

    fn name(&self) -> &str {
        "get_weather"
    }

    fn description(&self) -> &str {
        "Get current weather for a city"
    }

    async fn execute(&self, params: Self::Input) -> Result<Self::Output> {
        // Implementation that calls weather API
        Ok(WeatherOutput {
            temperature: 22.5,
            description: "Partly cloudy".to_string(),
            humidity: 65,
        })
    }

    fn usage_rule(&self) -> Option<String> {
        Some("Use this tool when the user asks about weather or temperature in a specific location.".to_string())
    }
}
```

### Step 3: Register with an Agent

```rust
// In your agent creation code
let weather_tool = WeatherTool { api_key: "...".to_string() };
registry.register(Box::new(weather_tool))?;

// The agent will automatically include this tool
let agent = DatabaseAgent::new(..., registry);
```

## Built-in Tools

Pattern includes several built-in tools following the Letta/MemGPT pattern:

### Memory Management (`context`)
- `append`: Add content to memory blocks
- `replace`: Replace specific content in memory
- `archive`: Move memory to long-term storage
- `swap`: Atomic archive + load operation

### Archival Storage (`recall`)
- `insert`: Store new archival memories
- `read`: Retrieve specific memories by label
- `delete`: Remove archival memories

### Search (`search`)
- Unified interface for searching across:
  - Archival memories
  - Conversation history
  - (Extensible to other domains)

### Communication (`send_message`)
- Send messages to users or other agents
- Supports different message types and metadata

## Common Patterns

### Multi-Operation Tools

Tools often support multiple operations:

```rust
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
struct ContextInput {
    operation: ContextOperation,
    #[serde(flatten)]
    params: ContextParams,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ContextOperation {
    Append,
    Replace,
    Archive,
    Swap,
}
```

### Error Handling

Tools should return descriptive errors:

```rust
async fn execute(&self, params: Self::Input) -> Result<Self::Output> {
    match params.operation {
        FileOperation::Read => {
            let content = tokio::fs::read_to_string(&params.path)
                .await
                .map_err(|e| CoreError::FileReadError {
                    path: params.path.clone(),
                    reason: e.to_string(),
                })?;
            Ok(FileOutput { content: Some(content), ..Default::default() })
        }
        // ...
    }
}
```

## Troubleshooting

### "Tool not found" Errors

- Check tool is registered: `registry.list_tools()`
- Verify name matches exactly (case-sensitive)
- Ensure tool is in agent's `available_tools()`

### Schema Generation Issues

- All fields must implement `JsonSchema`
- Use `#[schemars(default, with = "InnerType")]` for optional fields
- Avoid complex generic types in Input/Output

### Concurrent Execution Problems

- Tools must be `Send + Sync`
- Watch out for deadlocks when making tools that handle agent memory, it uses a DashMap internally. Make sure you don't try and hold two simultaneous references to a memory block. The default tools illustrate reasonable behaviour.
- The same is true for the registry itself.

### Type Conversion Errors

- Test JSON serialization separately
- Use `#[serde(rename_all = "snake_case")]` consistently
- Handle null/missing fields with `Option<T>` or defaults correctly

## Advanced Topics

### Tool Composition

Tools can call other tools through the registry:

```rust
async fn execute(&self, params: Self::Input) -> Result<Self::Output> {
    // First search for relevant data
    let search_result = self.registry
        .execute("search", json!({
            "domain": "archival_memory",
            "query": params.topic
        }))
        .await?;

    // Then process the results
    // ...
}
```

### Context-Aware Tools

Tools can receive an `AgentHandle` for context:

```rust
impl ContextTool {
    pub fn new(handle: AgentHandle) -> Self {
        Self { handle }
    }
}
```

This provides access to:
- Agent's memory blocks
- Database connection (with restrictions)
- Agent metadata


## Future plans

- WASM-based custom tool loading at runtime
