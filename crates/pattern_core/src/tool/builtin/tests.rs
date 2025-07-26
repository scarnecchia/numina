#[cfg(test)]
mod tests {
    use super::super::*;
    use crate::{
        UserId,
        context::AgentHandle,
        memory::{Memory, MemoryPermission, MemoryType},
        tool::ToolRegistry,
    };

    #[tokio::test]
    async fn test_builtin_tools_registration() {
        // Create a memory and handle
        let memory = Memory::with_owner(&UserId::generate());
        memory.create_block("test", "initial value").unwrap();

        let handle = AgentHandle::test_with_memory(memory);

        // Create a tool registry
        let registry = ToolRegistry::new();

        // Register built-in tools
        let builtin = BuiltinTools::default_for_agent(handle.clone());
        builtin.register_all(&registry);

        // Verify tools are registered
        let tool_names = registry.list_tools();
        assert!(tool_names.iter().any(|name| name == "recall"));
        assert!(tool_names.iter().any(|name| name == "context"));
        assert!(tool_names.iter().any(|name| name == "search"));
        assert!(tool_names.iter().any(|name| name == "send_message"));
    }

    #[tokio::test]
    async fn test_context_append_through_registry() {
        // Create a memory and handle
        let memory = Memory::with_owner(&UserId::generate());
        memory.create_block("test", "initial value").unwrap();

        // Make it a core memory block with append permission
        if let Some(mut block) = memory.get_block_mut("test") {
            block.memory_type = MemoryType::Core;
            block.permission = MemoryPermission::Append;
        }

        let handle = AgentHandle::test_with_memory(memory.clone());

        // Create and register tools
        let registry = ToolRegistry::new();
        let builtin = BuiltinTools::default_for_agent(handle);
        builtin.register_all(&registry);

        // Execute context tool with append operation
        let params = serde_json::json!({
            "operation": "append",
            "name": "test",
            "content": " appended content"
        });

        let result = registry.execute("context", params).await.unwrap();

        // Verify the result
        assert_eq!(result["success"], true);

        // Verify the memory was actually updated
        let block = memory.get_block("test").unwrap();
        assert_eq!(block.value, "initial value\n\n appended content");
    }

    #[tokio::test]
    async fn test_send_message_through_registry() {
        // Create a handle
        let handle = AgentHandle::default();

        // Create and register tools
        let registry = ToolRegistry::new();
        let builtin = BuiltinTools::default_for_agent(handle);
        builtin.register_all(&registry);

        // Execute send_message tool
        let params = serde_json::json!({
            "target": {
                "target_type": "user"
            },
            "content": "Hello from test!"
        });

        let result = registry.execute("send_message", params).await.unwrap();

        // Verify the result
        assert_eq!(result["success"], true);
        assert!(result["message_id"].is_string());
    }

    #[tokio::test]
    async fn test_context_replace_through_registry() {
        // Create a memory and handle
        let memory = Memory::with_owner(&UserId::generate());
        memory
            .create_block("persona", "I am a helpful AI assistant.")
            .unwrap();

        // Make it a core memory block with ReadWrite permission
        if let Some(mut block) = memory.get_block_mut("persona") {
            block.memory_type = MemoryType::Core;
            block.permission = MemoryPermission::ReadWrite;
        }

        let handle = AgentHandle::test_with_memory(memory.clone());

        // Create and register tools
        let registry = ToolRegistry::new();
        let builtin = BuiltinTools::default_for_agent(handle);
        builtin.register_all(&registry);

        // Execute context tool with replace operation
        let params = serde_json::json!({
            "operation": "replace",
            "name": "persona",
            "old_content": "helpful AI assistant",
            "new_content": "knowledgeable AI companion"
        });

        let result = registry.execute("context", params).await.unwrap();

        // Verify the result
        assert_eq!(result["success"], true);

        // Verify the memory was actually updated
        let block = memory.get_block("persona").unwrap();
        assert_eq!(block.value, "I am a knowledgeable AI companion.");
    }

    #[tokio::test]
    async fn test_recall_through_registry() {
        // Create a memory and handle
        let memory = Memory::with_owner(&UserId::generate());

        let handle = AgentHandle::test_with_memory(memory.clone());

        // Create and register tools
        let registry = ToolRegistry::new();
        let builtin = BuiltinTools::default_for_agent(handle);
        builtin.register_all(&registry);

        // Test inserting archival memory
        let insert_params = serde_json::json!({
            "operation": "insert",
            "content": "The user mentioned they enjoy hiking in the mountains.",
            "label": "user_hobbies"
        });

        let result = registry.execute("recall", insert_params).await.unwrap();

        assert_eq!(result["success"], true);
        assert!(
            result["message"]
                .as_str()
                .unwrap()
                .contains("Created recall memory")
        );

        // Test appending to archival memory
        let append_params = serde_json::json!({
            "operation": "append",
            "label": "user_hobbies",
            "content": " They also enjoy rock climbing."
        });

        let result = registry.execute("recall", append_params).await.unwrap();

        assert_eq!(result["success"], true);
        assert!(
            result["message"]
                .as_str()
                .unwrap()
                .contains("Appended to recall memory")
        );

        // Verify the append worked by reading
        let read_params = serde_json::json!({
            "operation": "read",
            "label": "user_hobbies"
        });

        let result = registry.execute("recall", read_params).await.unwrap();

        assert_eq!(result["success"], true);
        let results = result["results"].as_array().unwrap();
        assert_eq!(results.len(), 1);
        assert!(results[0]["content"].as_str().unwrap().contains("hiking"));
        assert!(
            results[0]["content"]
                .as_str()
                .unwrap()
                .contains("rock climbing")
        );
    }
}
