use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fmt::Debug;

use crate::Result;

/// A tool that can be executed by agents
#[async_trait]
pub trait Tool: Send + Sync + Debug {
    /// Get the name of this tool
    fn name(&self) -> &str;

    /// Get a human-readable description of what this tool does
    fn description(&self) -> &str;

    /// Get the JSON schema for the tool's parameters
    fn parameters_schema(&self) -> Value;

    /// Execute the tool with the given parameters
    async fn execute(&self, params: Value) -> Result<Value>;

    /// Check if the provided parameters are valid
    fn validate_params(&self, _params: &Value) -> Result<()> {
        // Default implementation that always passes
        // Tools can override this for custom validation
        Ok(())
    }

    /// Get usage examples for this tool
    fn examples(&self) -> Vec<ToolExample> {
        Vec::new()
    }
}

/// An example of how to use a tool
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolExample {
    pub description: String,
    pub parameters: Value,
    pub expected_output: Option<Value>,
}

/// A registry for managing available tools
#[derive(Debug, Clone)]
pub struct ToolRegistry {
    tools: std::collections::HashMap<String, std::sync::Arc<dyn Tool>>,
}

impl ToolRegistry {
    /// Create a new empty tool registry
    pub fn new() -> Self {
        Self {
            tools: std::collections::HashMap::new(),
        }
    }

    /// Register a tool
    pub fn register(&mut self, tool: std::sync::Arc<dyn Tool>) {
        self.tools.insert(tool.name().to_string(), tool);
    }

    /// Get a tool by name
    pub fn get(&self, name: &str) -> Option<&std::sync::Arc<dyn Tool>> {
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

        tool.validate_params(&params)?;
        tool.execute(params).await
    }
}

impl Default for ToolRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// The result of executing a tool
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResult {
    pub success: bool,
    pub output: Value,
    pub error: Option<String>,
    pub metadata: ToolResultMetadata,
}

/// Metadata about a tool execution
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ToolResultMetadata {
    pub execution_time: Option<std::time::Duration>,
    pub retries: usize,
    pub warnings: Vec<String>,
    pub custom: Value,
}

impl ToolResult {
    /// Create a successful tool result
    pub fn success(output: Value) -> Self {
        Self {
            success: true,
            output,
            error: None,
            metadata: ToolResultMetadata::default(),
        }
    }

    /// Create a failed tool result
    pub fn failure(error: impl Into<String>) -> Self {
        Self {
            success: false,
            output: Value::Null,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug)]
    struct TestTool;

    #[async_trait]
    impl Tool for TestTool {
        fn name(&self) -> &str {
            "test_tool"
        }

        fn description(&self) -> &str {
            "A tool for testing"
        }

        fn parameters_schema(&self) -> Value {
            serde_json::json!({
                "type": "object",
                "properties": {
                    "message": {
                        "type": "string",
                        "description": "A test message"
                    }
                },
                "required": ["message"]
            })
        }

        async fn execute(&self, params: Value) -> Result<Value> {
            Ok(serde_json::json!({
                "response": format!("Received: {}", params["message"])
            }))
        }
    }

    #[tokio::test]
    async fn test_tool_registry() {
        let mut registry = ToolRegistry::new();
        registry.register(std::sync::Arc::new(TestTool));

        assert_eq!(registry.list_tools(), vec!["test_tool".to_string()]);

        let result = registry
            .execute(
                "test_tool",
                serde_json::json!({
                    "message": "Hello, world!"
                }),
            )
            .await
            .unwrap();

        assert_eq!(result["response"], "Received: Hello, world!");
    }

    #[test]
    fn test_tool_result() {
        let result = ToolResult::success(serde_json::json!({ "data": 42 }))
            .with_execution_time(std::time::Duration::from_millis(100))
            .with_warning("This is a test warning");

        assert!(result.success);
        assert_eq!(result.output["data"], 42);
        assert_eq!(result.metadata.warnings.len(), 1);
    }
}
