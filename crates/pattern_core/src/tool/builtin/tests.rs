#[cfg(test)]
mod tests {
    use super::super::*;
    use crate::{
        AgentId, UserId,
        context::AgentHandle,
        memory::Memory,
        tool::builtin::memory::{UpdateMemoryInput, UpdateMemoryOutput, UpdateMemoryTool},
        tool::{AiTool, ToolRegistry},
    };

    #[tokio::test]
    async fn test_builtin_tools_registration() {
        // Create a memory and handle
        let memory = Memory::with_owner(UserId::generate());
        memory.create_block("test", "initial value").unwrap();

        let handle = AgentHandle {
            agent_id: AgentId::generate(),
            memory,
        };

        // Create a tool registry
        let registry = ToolRegistry::new();

        // Register built-in tools
        let builtin = BuiltinTools::default_for_agent(handle.clone());
        builtin.register_all(&registry);

        // Verify tools are registered
        let tool_names = registry.list_tools();
        assert!(tool_names.iter().any(|name| name == "update_memory"));
        assert!(tool_names.iter().any(|name| name == "send_message"));
    }

    #[tokio::test]
    async fn test_update_memory_through_registry() {
        // Create a memory and handle
        let memory = Memory::with_owner(UserId::generate());
        memory.create_block("test", "initial value").unwrap();

        let handle = AgentHandle {
            agent_id: AgentId::generate(),
            memory: memory.clone(),
        };

        // Create and register tools
        let registry = ToolRegistry::new();
        let builtin = BuiltinTools::default_for_agent(handle);
        builtin.register_all(&registry);

        // Execute update_memory tool through registry
        let params = serde_json::json!({
            "label": "test",
            "value": "updated value",
            "description": "Test block"
        });

        let result = registry.execute("update_memory", params).await.unwrap();

        // Verify the result
        assert_eq!(result["success"], true);
        assert_eq!(result["previous_value"], "initial value");

        // Verify the memory was actually updated
        let block = memory.get_block("test").unwrap();
        assert_eq!(block.content, "updated value");
        assert_eq!(block.description, Some("Test block".to_string()));
    }

    #[tokio::test]
    async fn test_send_message_through_registry() {
        // Create a handle
        let handle = AgentHandle {
            agent_id: AgentId::generate(),
            memory: Memory::with_owner(UserId::generate()),
        };

        // Create and register tools
        let registry = ToolRegistry::new();
        let builtin = BuiltinTools::default_for_agent(handle);
        builtin.register_all(&registry);

        // Execute send_message tool
        let params = serde_json::json!({
            "target": {
                "type": "user"
            },
            "content": "Hello from test!"
        });

        let result = registry.execute("send_message", params).await.unwrap();

        // Verify the result
        assert_eq!(result["success"], true);
        assert!(result["message_id"].is_string());
    }

    #[tokio::test]
    async fn test_custom_memory_tool() {
        use crate::tool::DynamicToolAdapter;

        // Create a custom memory tool that adds a prefix
        #[derive(Debug, Clone)]
        struct PrefixedMemoryTool {
            handle: AgentHandle,
            prefix: String,
        }

        #[async_trait::async_trait]
        impl AiTool for PrefixedMemoryTool {
            type Input = UpdateMemoryInput;
            type Output = UpdateMemoryOutput;

            fn name(&self) -> &str {
                "update_memory"
            }

            fn description(&self) -> &str {
                "Update memory with a prefix"
            }

            async fn execute(&self, mut params: Self::Input) -> crate::Result<Self::Output> {
                params.value = format!("{}: {}", self.prefix, params.value);
                UpdateMemoryTool {
                    handle: self.handle.clone(),
                }
                .execute(params)
                .await
            }
        }

        // Create memory and handle
        let memory = Memory::with_owner(UserId::generate());
        let handle = AgentHandle {
            agent_id: AgentId::generate(),
            memory: memory.clone(),
        };

        // Use builder to replace default memory tool
        let custom_tool = PrefixedMemoryTool {
            handle: handle.clone(),
            prefix: "PREFIXED".to_string(),
        };

        let builtin = BuiltinTools::builder()
            .with_memory_tool(DynamicToolAdapter::new(custom_tool))
            .build_for_agent(handle);

        // Register tools
        let registry = ToolRegistry::new();
        builtin.register_all(&registry);

        // Execute the custom tool
        let params = serde_json::json!({
            "label": "test",
            "value": "hello world"
        });

        registry.execute("update_memory", params).await.unwrap();

        // Verify the prefix was added
        let block = memory.get_block("test").unwrap();
        assert_eq!(block.content, "PREFIXED: hello world");
    }
}
