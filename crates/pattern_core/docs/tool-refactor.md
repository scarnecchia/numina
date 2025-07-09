# Tool System Refactor: Type-Safe Tools with MCP-Compatible Schemas

## Overview

We've refactored the `AiTool` trait to use generics instead of `serde_json::Value`, providing compile-time type safety while maintaining compatibility with MCP's requirement for reference-free JSON schemas. This system now supports both external tools and built-in agent tools.

## Key Changes

### 1. Generic AiTool Trait

The new `AiTool` trait is generic over input and output types:

```rust
#[async_trait]
pub trait AiTool: Send + Sync + Debug {
    type Input: JsonSchema + for<'de> Deserialize<'de> + Serialize + Send + Sync;
    type Output: JsonSchema + Serialize + Send + Sync;

    fn name(&self) -> &str;
    fn description(&self) -> &str;
    async fn execute(&self, params: Self::Input) -> Result<Self::Output>;
    
    // New: to_genai_tool() method directly on AiTool trait
    fn to_genai_tool(&self) -> genai::chat::Tool {
        genai::chat::Tool::new(self.name())
            .with_description(self.description())
            .with_schema(self.parameters_schema())
    }
}
```

### 2. MCP-Compatible Schema Generation

The trait automatically generates MCP-compatible schemas with all references inlined:

```rust
fn parameters_schema(&self) -> Value {
    let mut settings = SchemaSettings::default();
    settings.inline_subschemas = true;  // Critical for MCP compatibility
    settings.meta_schema = None;

    let generator = SchemaGenerator::new(settings);
    let schema = generator.into_root_schema_for::<Self::Input>();
    
    serde_json::to_value(schema).unwrap_or_else(|_| {
        serde_json::json!({
            "type": "object",
            "properties": {},
            "additionalProperties": false
        })
    })
}
```

### 3. Dynamic Tool Support

For cases where dynamic dispatch is needed (e.g., storing tools in a registry), we provide a `DynamicTool` trait and `DynamicToolAdapter`:

```rust
#[async_trait]
pub trait DynamicTool: Send + Sync + Debug {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn parameters_schema(&self) -> Value;
    fn output_schema(&self) -> Value;
    async fn execute(&self, params: Value) -> Result<Value>;
    
    // New: validate_params() method for parameter validation
    fn validate_params(&self, params: &Value) -> Result<()>;
    
    // Also has to_genai_tool() method
    fn to_genai_tool(&self) -> genai::chat::Tool;
}
```

The `DynamicToolAdapter<T: AiTool>` automatically converts typed tools to dynamic tools, handling serialization/deserialization transparently.

### 4. Enhanced Tool Registry

The `ToolRegistry` now supports both typed and dynamic tools with thread-safe concurrent access:

```rust
impl ToolRegistry {
    // Uses DashMap internally for concurrent access
    tools: DashMap<CompactString, Box<dyn DynamicTool>>,
    
    // Register a typed tool - automatically wrapped in DynamicToolAdapter
    // Note: &self instead of &mut self for concurrent access
    pub fn register<T: AiTool + 'static>(&self, tool: T);
    
    // Register a pre-wrapped dynamic tool
    pub fn register_dynamic(&self, tool: Box<dyn DynamicTool>);
    
    // Execute any tool by name with JSON parameters
    pub async fn execute(&self, tool_name: &str, params: Value) -> Result<Value>;
}
```

The registry uses `DashMap` for thread-safe concurrent access and `CompactString` to optimize memory usage for tool names (small strings are stored inline on the stack).

## Benefits

1. **Type Safety**: Tool implementations get compile-time type checking for inputs and outputs
2. **Better IDE Support**: Auto-completion and type hints for tool parameters
3. **MCP Compatibility**: Generated schemas have no `$ref` fields, meeting MCP requirements
4. **Backward Compatibility**: The registry still accepts and returns JSON values
5. **Error Handling**: Type mismatches are caught at deserialization time with clear errors
6. **Thread Safety**: Registry operations are thread-safe thanks to DashMap
7. **Memory Efficiency**: Tool names use CompactString for optimal memory usage

## Example Usage

```rust
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
struct WeatherInput {
    city: String,
    #[serde(default)]
    country_code: Option<String>,
    unit: TemperatureUnit,
}

#[derive(Debug, Serialize, JsonSchema)]
struct WeatherOutput {
    temperature: f64,
    conditions: String,
}

#[derive(Debug)]
struct WeatherTool;

#[async_trait]
impl AiTool for WeatherTool {
    type Input = WeatherInput;
    type Output = WeatherOutput;

    fn name(&self) -> &str { "get_weather" }
    fn description(&self) -> &str { "Get current weather" }

    async fn execute(&self, params: Self::Input) -> Result<Self::Output> {
        // Type-safe implementation
        Ok(WeatherOutput {
            temperature: 22.5,
            conditions: "Sunny".to_string(),
        })
    }
}

// Usage
let registry = ToolRegistry::new();  // No mut needed - DashMap allows concurrent access
registry.register(WeatherTool);

// Still works with JSON for compatibility
let result = registry.execute(
    "get_weather",
    json!({ "city": "Tokyo", "unit": "celsius" })
).await?;
```

## Migration Guide

To migrate existing tools:

1. Define input/output structs with `Deserialize`/`Serialize` and `JsonSchema` derives
2. Update the tool implementation to use the new associated types
3. Replace `execute(&self, params: Value)` with `execute(&self, params: Self::Input)`
4. Return the typed output instead of JSON

The registry API remains unchanged, so tool consumers don't need modifications.

## Built-in Tools

Pattern agents come with built-in tools that use the same `AiTool` trait:

### UpdateMemoryTool
- Updates or creates memory blocks
- Uses `AgentHandle` for efficient access to agent memory
- Type-safe with `UpdateMemoryInput` and `UpdateMemoryOutput`

### SendMessageTool
- Sends messages to users, agents, groups, or channels
- Currently a stub implementation
- Will integrate with message routing system

### Registration

Built-in tools are registered automatically when creating an agent:

```rust
let builtin = BuiltinTools::default_for_agent(context.handle());
builtin.register_all(&context.tools);
```

### Customization

Built-in tools can be replaced with custom implementations:

```rust
let builtin = BuiltinTools::builder()
    .with_memory_tool(CustomMemoryTool::new())
    .build_for_agent(handle);
```

## Technical Notes

- We use `schemars` with `inline_subschemas = true` to ensure MCP compatibility
- The `r#gen` module syntax is required because `gen` is a reserved keyword in Rust 2024 (once we move to a newer crate version, this will change to `generate`)
- Input types must implement `Serialize` for the tool examples feature
- The `DynamicToolAdapter` handles all serialization/deserialization errors gracefully
- Both `AiTool` and `DynamicTool` now have `to_genai_tool()` methods for direct conversion to genai tools
- `DynamicTool` includes a `validate_params()` method for custom parameter validation
- Built-in tools use `AgentHandle` which provides cheap cloning without copying message history
- Memory uses `Arc<DashMap>` internally for thread-safe concurrent access
