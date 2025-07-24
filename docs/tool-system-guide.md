# Pattern Tool System Guide

This guide explains how Pattern's tool system works, from trait definitions to execution flow.

## Overview

Pattern's tool system enables agents to perform actions through a type-safe, extensible framework. Tools are functions that agents can call during conversations to interact with memory, send messages, search data, or perform custom operations.

## Core Components

### 1. The `AiTool` Trait (Type-Safe Foundation)

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

### 2. The `DynamicTool` Trait (Runtime Flexibility)

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
flowchart TD
    subgraph define ["1. Define Tool"]
        A[Custom Tool struct]
        B[impl AiTool with Input/Output types]
        C[Auto-generated impl DynamicTool]
        A --> B
        B --> C
    end
    
    subgraph register ["2. Register Tool"]
        D["Box::new(tool)"]
        E[ToolRegistry::register]
        F["Stored as Box&lt;dyn DynamicTool&gt;"]
        D --> E
        E --> F
    end
    
    subgraph discover ["3. Agent Discovery"]
        G[Agent calls available_tools]
        H[Registry returns tool list]
        I[Generate JSON schemas]
        J[Send schemas to LLM]
        G --> H
        H --> I
        I --> J
    end
    
    subgraph execute ["4. Tool Execution"]
        K[LLM requests tool call]
        L[JSON parameters]
        M[Registry finds tool by name]
        N[execute_dynamic called]
        K --> L
        L --> M
        M --> N
    end
    
    subgraph convert ["5. Type Conversion"]
        O[JSON → Rust type]
        P[Call tool.execute]
        Q[Rust type → JSON]
        R[Return result]
        N --> O
        O --> P
        P --> Q
        Q --> R
    end
    
    define --> register
    register --> discover
    discover --> execute
    execute --> convert
    
    style A fill:#2d3748,stroke:#1a202c,color:#fff
    style K fill:#e53e3e,stroke:#c53030,color:#fff
    style R fill:#38a169,stroke:#2f855a,color:#fff
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

## How Type Erasure Works

1. **Concrete Type**: `WeatherTool` implements `AiTool<Input=WeatherInput, Output=WeatherOutput>`
2. **Auto Bridge**: Compiler generates `impl DynamicTool for WeatherTool`
3. **Boxing**: `Box::new(weather_tool)` creates `Box<WeatherTool>`
4. **Trait Object**: Coerced to `Box<dyn DynamicTool>` for storage
5. **Runtime**: JSON ↔ Rust type conversion happens in the bridge impl

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

Following Letta's design, tools often support multiple operations:

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

1. Check tool is registered: `registry.list_tools()`
2. Verify name matches exactly (case-sensitive)
3. Ensure tool is in agent's `available_tools()`

### Schema Generation Issues

1. All fields must implement `JsonSchema`
2. Use `#[serde(default)]` for optional fields
3. Avoid complex generic types in Input/Output

### Async Execution Problems

1. Tools must be `Send + Sync`
2. Avoid holding locks across await points
3. Use `Arc<Mutex<_>>` for shared state, not `Rc<RefCell<_>>`

### Type Conversion Errors

1. Test JSON serialization separately
2. Use `#[serde(rename_all = "snake_case")]` consistently
3. Handle null/missing fields with `Option<T>` or defaults

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

Tools receive an `AgentHandle` for context:

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

### Performance Considerations

1. **Tool Registry**: Uses `DashMap` for concurrent access
2. **JSON Conversion**: Happens once per call, minimal overhead
3. **Schema Generation**: Cached after first generation
4. **Async Execution**: Non-blocking, allows concurrent tool calls

## Summary

Pattern's tool system provides:
- **Type Safety**: Compile-time checking for tool implementations
- **Flexibility**: Runtime tool discovery and dynamic dispatch
- **Extensibility**: Easy to add custom tools
- **Integration**: Built-in tools follow established patterns

The key insight is the automatic bridge from `AiTool` to `DynamicTool`, which gives you both type safety during development and flexibility at runtime.