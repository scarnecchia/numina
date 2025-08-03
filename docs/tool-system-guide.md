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
    fn usage_rule(&self) -> Option<&'static str> { None }
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
    fn usage_rule(&self) -> Option<&'static str>;

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

    fn usage_rule(&self) -> Option<&'static str> {
        Some("Use this tool when the user asks about weather or temperature in a specific location.")
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
- Has built-in rule: "the conversation will end when called"

## Tool Rules System

Pattern includes a sophisticated tool rules system that allows fine-grained control over tool execution flow, dependencies, and constraints. This enables agents to follow complex workflows, enforce tool ordering, and optimize performance.

### Rule Types

```rust
pub enum ToolRuleType {
    /// Tool starts the conversation (must be called first)
    StartConstraint,
    
    /// Maximum number of times this tool can be called
    MaxCalls(u32),
    
    /// Tool ends the conversation loop when called
    ExitLoop,
    
    /// Tool continues the conversation loop when called
    ContinueLoop,
    
    /// Minimum cooldown period between calls
    Cooldown(Duration),
    
    /// This tool must be called after specified tools
    RequiresPreceding,
}
```

### Configuring Tool Rules

Tool rules can be configured in three ways:

#### 1. In TOML Configuration

```toml
[agent]
name = "DataProcessor"
tools = ["load_data", "validate", "process", "save_results"]

[[agent.tool_rules]]
tool_name = "load_data"
rule_type = "StartConstraint"
priority = 10

[[agent.tool_rules]]
tool_name = "validate"
rule_type = "RequiresPreceding"
conditions = ["load_data"]
priority = 8

[[agent.tool_rules]]
tool_name = "process"
rule_type = { MaxCalls = 3 }
priority = 5

[[agent.tool_rules]]
tool_name = "save_results"
rule_type = "ExitLoop"
priority = 9
```

#### 2. Via CLI Commands

```bash
# Add a rule that makes 'send_message' end the conversation
pattern-cli agent add-rule MyAgent send_message exit-loop

# Add a dependency rule
pattern-cli agent add-rule MyAgent validate requires-preceding -c load_data

# Add a max calls rule
pattern-cli agent add-rule MyAgent api_request max-calls -p 5

# List all rules for an agent
pattern-cli agent list-rules MyAgent

# Remove a specific rule
pattern-cli agent remove-rule MyAgent send_message exit-loop
```

#### 3. Programmatically

```rust
use pattern_core::agent::tool_rules::{ToolRule, ToolRuleType};

let rules = vec![
    ToolRule {
        tool_name: "initialize".to_string(),
        rule_type: ToolRuleType::StartConstraint,
        conditions: vec![],
        priority: 10,
        metadata: None,
    },
    ToolRule {
        tool_name: "cleanup".to_string(),
        rule_type: ToolRuleType::ExitLoop,
        conditions: vec![],
        priority: 9,
        metadata: None,
    },
];

// When creating an agent
let agent = DatabaseAgent::new(
    // ... other parameters ...
    tool_rules: rules,
);
```

### How Tool Rules Work

1. **Start Constraints**: Tools marked with `StartConstraint` are automatically executed when a conversation begins
2. **Dependencies**: Tools with `RequiresPreceding` can only be called after their prerequisite tools
3. **Call Limits**: Tools with `MaxCalls` enforce usage limits per conversation
4. **Loop Control**: 
   - `ExitLoop` tools terminate the conversation after execution
   - `ContinueLoop` tools don't require heartbeat checks (performance optimization)
5. **Cooldowns**: Prevent rapid repeated calls to expensive tools

### Rule Enforcement

The tool rules engine validates every tool call before execution:

```rust
// If a tool violates a rule, the agent receives an error:
"Tool rule violation: Tool 'process' cannot be executed: prerequisites ['validate'] not met"
```

### Performance Benefits

Tool rules enable performance optimizations:

1. **Heartbeat Optimization**: Tools marked with `ContinueLoop` skip heartbeat checks, reducing overhead
2. **Early Termination**: `ExitLoop` tools prevent unnecessary continuation prompts
3. **Automatic Initialization**: `StartConstraint` tools run automatically without LLM prompting

### Example: ETL Pipeline Agent

```toml
[[agent.tool_rules]]
tool_name = "connect_db"
rule_type = "StartConstraint"
priority = 10

[[agent.tool_rules]]
tool_name = "extract_data"
rule_type = "RequiresPreceding"
conditions = ["connect_db"]
priority = 8

[[agent.tool_rules]]
tool_name = "transform_data"
rule_type = "RequiresPreceding"
conditions = ["extract_data"]
priority = 7

[[agent.tool_rules]]
tool_name = "load_warehouse"
rule_type = "RequiresPreceding"
conditions = ["transform_data"]
priority = 6

[[agent.tool_rules]]
tool_name = "disconnect_db"
rule_type = "ExitLoop"
priority = 10
```

This ensures the ETL pipeline executes in the correct order and terminates cleanly.

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
