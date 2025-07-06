use async_trait::async_trait;
use schemars::{JsonSchema, r#gen::SchemaGenerator, r#gen::SchemaSettings};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fmt::Debug;

use crate::Result;

/// A tool that can be executed by agents with type-safe input and output
#[async_trait]
pub trait AiTool: Send + Sync + Debug {
    /// The input type for this tool
    type Input: JsonSchema + for<'de> Deserialize<'de> + Serialize + Send + Sync;

    /// The output type for this tool
    type Output: JsonSchema + Serialize + Send + Sync;

    /// Get the name of this tool
    fn name(&self) -> &str;

    /// Get a human-readable description of what this tool does
    fn description(&self) -> &str;

    /// Execute the tool with the given parameters
    async fn execute(&self, params: Self::Input) -> Result<Self::Output>;

    /// Get usage examples for this tool
    fn examples(&self) -> Vec<ToolExample<Self::Input, Self::Output>> {
        Vec::new()
    }

    /// Get the JSON schema for the tool's parameters (MCP-compatible, no refs)
    fn parameters_schema(&self) -> Value {
        let mut settings = SchemaSettings::default();
        settings.inline_subschemas = true;
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

    /// Get the JSON schema for the tool's output (MCP-compatible, no refs)
    fn output_schema(&self) -> Value {
        let mut settings = SchemaSettings::default();
        settings.inline_subschemas = true;
        settings.meta_schema = None;

        let generator = SchemaGenerator::new(settings);
        let schema = generator.into_root_schema_for::<Self::Output>();

        serde_json::to_value(schema).unwrap_or_else(|_| {
            serde_json::json!({
                "type": "object",
                "properties": {},
                "additionalProperties": false
            })
        })
    }
}

/// Type-erased version of AiTool for dynamic dispatch
#[async_trait]
pub trait DynamicTool: Send + Sync + Debug {
    /// Get the name of this tool
    fn name(&self) -> &str;

    /// Get a human-readable description of what this tool does
    fn description(&self) -> &str;

    /// Get the JSON schema for the tool's parameters
    fn parameters_schema(&self) -> Value;

    /// Get the JSON schema for the tool's output
    fn output_schema(&self) -> Value;

    /// Execute the tool with the given parameters
    async fn execute(&self, params: Value) -> Result<Value>;

    /// Get usage examples for this tool
    fn examples(&self) -> Vec<DynamicToolExample>;

    /// Convert to a genai Tool
    fn to_genai_tool(&self) -> genai::chat::Tool {
        genai::chat::Tool::new(self.name())
            .with_description(self.description())
            .with_schema(self.parameters_schema())
    }
}

/// Adapter to convert a typed AiTool into a DynamicTool
pub struct DynamicToolAdapter<T: AiTool> {
    inner: T,
}

impl<T: AiTool> DynamicToolAdapter<T> {
    pub fn new(tool: T) -> Self {
        Self { inner: tool }
    }
}

impl<T: AiTool> Debug for DynamicToolAdapter<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DynamicToolAdapter")
            .field("tool", &self.inner)
            .finish()
    }
}

#[async_trait]
impl<T> DynamicTool for DynamicToolAdapter<T>
where
    T: AiTool + 'static,
{
    fn name(&self) -> &str {
        self.inner.name()
    }

    fn description(&self) -> &str {
        self.inner.description()
    }

    fn parameters_schema(&self) -> Value {
        self.inner.parameters_schema()
    }

    fn output_schema(&self) -> Value {
        self.inner.output_schema()
    }

    async fn execute(&self, params: Value) -> Result<Value> {
        // Deserialize the JSON value into the tool's input type
        let input: T::Input = serde_json::from_value(params)
            .map_err(|e| crate::CoreError::tool_validation_error(self.name(), e.to_string()))?;

        // Execute the tool
        let output = self.inner.execute(input).await?;

        // Serialize the output back to JSON
        serde_json::to_value(output)
            .map_err(|e| crate::CoreError::tool_execution_error(self.name(), e.to_string()))
    }

    fn examples(&self) -> Vec<DynamicToolExample> {
        self.inner
            .examples()
            .into_iter()
            .map(|ex| DynamicToolExample {
                description: ex.description,
                parameters: serde_json::to_value(ex.parameters).unwrap_or(Value::Null),
                expected_output: ex
                    .expected_output
                    .map(|o| serde_json::to_value(o).unwrap_or(Value::Null)),
            })
            .collect()
    }
}

/// An example of how to use a tool with typed parameters
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolExample<I, O> {
    pub description: String,
    pub parameters: I,
    pub expected_output: Option<O>,
}

/// An example of how to use a tool with dynamic parameters
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DynamicToolExample {
    pub description: String,
    pub parameters: Value,
    pub expected_output: Option<Value>,
}

/// A registry for managing available tools
#[derive(Debug, Clone)]
pub struct ToolRegistry {
    tools: std::collections::HashMap<String, std::sync::Arc<dyn DynamicTool>>,
}

impl ToolRegistry {
    /// Create a new empty tool registry
    pub fn new() -> Self {
        Self {
            tools: std::collections::HashMap::new(),
        }
    }

    /// Register a typed tool
    pub fn register<T: AiTool + 'static>(&mut self, tool: T) {
        let dynamic_tool = DynamicToolAdapter::new(tool);
        self.tools.insert(
            dynamic_tool.name().to_string(),
            std::sync::Arc::new(dynamic_tool),
        );
    }

    /// Register a dynamic tool directly
    pub fn register_dynamic(&mut self, tool: std::sync::Arc<dyn DynamicTool>) {
        self.tools.insert(tool.name().to_string(), tool);
    }

    /// Get a tool by name
    pub fn get(&self, name: &str) -> Option<&std::sync::Arc<dyn DynamicTool>> {
        self.tools.get(name)
    }

    /// Get all tool names
    pub fn list_tools(&self) -> Vec<String> {
        self.tools.keys().cloned().collect()
    }

    /// Execute a tool by name
    pub async fn execute(&self, tool_name: &str, params: Value) -> Result<Value> {
        let tool = self
            .get(tool_name)
            .ok_or_else(|| crate::CoreError::tool_not_found(tool_name, self.list_tools()))?;

        tool.execute(params).await
    }

    /// Get all tools as genai tools
    pub fn to_genai_tools(&self) -> Vec<genai::chat::Tool> {
        self.tools
            .values()
            .map(|tool| tool.to_genai_tool())
            .collect()
    }
}

impl Default for ToolRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// The result of executing a tool
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResult<T> {
    pub success: bool,
    pub output: Option<T>,
    pub error: Option<String>,
    pub metadata: ToolResultMetadata,
}

/// Metadata about a tool execution
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ToolResultMetadata {
    #[serde(
        with = "crate::utils::duration_millis",
        skip_serializing_if = "Option::is_none"
    )]
    pub execution_time: Option<std::time::Duration>,
    pub retries: usize,
    pub warnings: Vec<String>,
    pub custom: Value,
}

impl<T> ToolResult<T> {
    /// Create a successful tool result
    pub fn success(output: T) -> Self {
        Self {
            success: true,
            output: Some(output),
            error: None,
            metadata: ToolResultMetadata::default(),
        }
    }

    /// Create a failed tool result
    pub fn failure(error: impl Into<String>) -> Self {
        Self {
            success: false,
            output: None,
            error: Some(error.into()),
            metadata: ToolResultMetadata::default(),
        }
    }

    /// Add execution time to the result
    pub fn with_execution_time(mut self, duration: std::time::Duration) -> Self {
        self.metadata.execution_time = Some(duration);
        self
    }

    /// Add a warning to the result
    pub fn with_warning(mut self, warning: impl Into<String>) -> Self {
        self.metadata.warnings.push(warning.into());
        self
    }
}

/// Helper macro to implement a simple tool
#[macro_export]
macro_rules! impl_tool {
    (
        name: $name:expr,
        description: $desc:expr,
        input: $input:ty,
        output: $output:ty,
        execute: $execute:expr
    ) => {
        #[derive(Debug)]
        struct Tool;

        #[async_trait::async_trait]
        impl $crate::tool::AiTool for Tool {
            type Input = $input;
            type Output = $output;

            fn name(&self) -> &str {
                $name
            }

            fn description(&self) -> &str {
                $desc
            }

            async fn execute(&self, params: Self::Input) -> $crate::Result<Self::Output> {
                $execute(params).await
            }
        }
    };
}

#[cfg(test)]
mod tests {
    use super::*;
    use schemars::JsonSchema;

    #[derive(Debug, Deserialize, Serialize, JsonSchema)]
    struct TestInput {
        message: String,
        #[serde(default)]
        count: Option<u32>,
    }

    #[derive(Debug, Serialize, JsonSchema)]
    struct TestOutput {
        response: String,
        processed_count: u32,
    }

    #[derive(Debug)]
    struct TestTool;

    #[async_trait]
    impl AiTool for TestTool {
        type Input = TestInput;
        type Output = TestOutput;

        fn name(&self) -> &str {
            "test_tool"
        }

        fn description(&self) -> &str {
            "A tool for testing"
        }

        async fn execute(&self, params: Self::Input) -> Result<Self::Output> {
            Ok(TestOutput {
                response: format!("Received: {}", params.message),
                processed_count: params.count.unwrap_or(1),
            })
        }

        fn examples(&self) -> Vec<ToolExample<Self::Input, Self::Output>> {
            vec![ToolExample {
                description: "Basic example".to_string(),
                parameters: TestInput {
                    message: "Hello".to_string(),
                    count: Some(5),
                },
                expected_output: Some(TestOutput {
                    response: "Received: Hello".to_string(),
                    processed_count: 5,
                }),
            }]
        }
    }

    #[tokio::test]
    async fn test_typed_tool() {
        let tool = TestTool;

        let result = tool
            .execute(TestInput {
                message: "Hello, world!".to_string(),
                count: Some(3),
            })
            .await
            .unwrap();

        assert_eq!(result.response, "Received: Hello, world!");
        assert_eq!(result.processed_count, 3);
    }

    #[tokio::test]
    async fn test_tool_registry() {
        let mut registry = ToolRegistry::new();
        registry.register(TestTool);

        assert_eq!(registry.list_tools(), vec!["test_tool".to_string()]);

        let result = registry
            .execute(
                "test_tool",
                serde_json::json!({
                    "message": "Hello, world!",
                    "count": 42
                }),
            )
            .await
            .unwrap();

        assert_eq!(result["response"], "Received: Hello, world!");
        assert_eq!(result["processed_count"], 42);
    }

    #[test]
    fn test_schema_generation() {
        let tool = TestTool;
        let schema = tool.parameters_schema();

        // Check that the schema has no $ref
        let schema_str = serde_json::to_string(&schema).unwrap();
        assert!(!schema_str.contains("\"$ref\""));

        // Check basic structure
        assert_eq!(schema["type"], "object");
        assert!(schema["properties"]["message"].is_object());
        assert!(schema["properties"]["count"].is_object());
    }

    #[test]
    fn test_tool_result() {
        let result = ToolResult::success("test output")
            .with_execution_time(std::time::Duration::from_millis(100))
            .with_warning("This is a test warning");

        assert!(result.success);
        assert_eq!(result.output, Some("test output"));
        assert_eq!(result.metadata.warnings.len(), 1);
    }
}
