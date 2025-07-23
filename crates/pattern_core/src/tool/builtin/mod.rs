//! Built-in tools for agents
//!
//! This module provides the standard tools that all agents have access to,
//! including memory management and inter-agent communication.

mod manage_archival_memory;
mod manage_core_memory;
mod search_conversations;
mod send_message;
#[cfg(test)]
mod test_schemas;

use std::fmt::Debug;

pub use manage_archival_memory::{
    ArchivalMemoryOperationType, ArchivalSearchResult, ManageArchivalMemoryInput,
    ManageArchivalMemoryOutput, ManageArchivalMemoryTool,
};
pub use manage_core_memory::{
    CoreMemoryOperationType, ManageCoreMemoryInput, ManageCoreMemoryOutput, ManageCoreMemoryTool,
};
use schemars::JsonSchema;
pub use search_conversations::{
    ConversationResult, MessageSummary, SearchConversationsInput, SearchConversationsOutput,
    SearchConversationsTool,
};
pub use send_message::SendMessageTool;
use serde::{Deserialize, Serialize};

use crate::{
    context::AgentHandle,
    tool::{DynamicTool, DynamicToolAdapter, ToolRegistry},
};

/// Registry specifically for built-in tools
#[derive(Clone)]
pub struct BuiltinTools {
    manage_archival_memory_tool: Box<dyn DynamicTool>,
    manage_core_memory_tool: Box<dyn DynamicTool>,
    search_conversations_tool: Box<dyn DynamicTool>,
    send_message_tool: Box<dyn DynamicTool>,
}

impl BuiltinTools {
    /// Create default built-in tools for an agent
    pub fn default_for_agent<C: surrealdb::Connection + Debug + Clone>(
        handle: AgentHandle<C>,
    ) -> Self {
        Self {
            manage_archival_memory_tool: Box::new(DynamicToolAdapter::new(
                ManageArchivalMemoryTool {
                    handle: handle.clone(),
                },
            )),
            manage_core_memory_tool: Box::new(DynamicToolAdapter::new(ManageCoreMemoryTool {
                handle: handle.clone(),
            })),
            search_conversations_tool: Box::new(DynamicToolAdapter::new(SearchConversationsTool {
                handle: handle.clone(),
            })),
            send_message_tool: Box::new(DynamicToolAdapter::new(SendMessageTool {
                handle: handle.clone(),
            })),
        }
    }

    /// Register all tools to a registry
    pub fn register_all(&self, registry: &ToolRegistry) {
        registry.register_dynamic(self.manage_archival_memory_tool.clone_box());
        registry.register_dynamic(self.manage_core_memory_tool.clone_box());
        registry.register_dynamic(self.search_conversations_tool.clone_box());
        registry.register_dynamic(self.send_message_tool.clone_box());
    }

    /// Builder pattern for customization
    pub fn builder() -> BuiltinToolsBuilder {
        BuiltinToolsBuilder::default()
    }
}

/// Builder for customizing built-in tools
#[derive(Default)]
pub struct BuiltinToolsBuilder {
    manage_archival_memory_tool: Option<Box<dyn DynamicTool>>,
    manage_core_memory_tool: Option<Box<dyn DynamicTool>>,
    search_conversations_tool: Option<Box<dyn DynamicTool>>,
    send_message_tool: Option<Box<dyn DynamicTool>>,
}

impl BuiltinToolsBuilder {
    /// Replace the default manage archival memory tool
    pub fn with_manage_archival_memory_tool(mut self, tool: impl DynamicTool + 'static) -> Self {
        self.manage_archival_memory_tool = Some(Box::new(tool));
        self
    }

    /// Replace the default manage core memory tool
    pub fn with_manage_core_memory_tool(mut self, tool: impl DynamicTool + 'static) -> Self {
        self.manage_core_memory_tool = Some(Box::new(tool));
        self
    }

    /// Replace the default send_message tool
    pub fn with_send_message_tool(mut self, tool: impl DynamicTool + 'static) -> Self {
        self.send_message_tool = Some(Box::new(tool));
        self
    }

    /// Build the tools for a specific agent
    pub fn build_for_agent<C: surrealdb::Connection + Debug + Clone>(
        self,
        handle: AgentHandle<C>,
    ) -> BuiltinTools {
        let defaults = BuiltinTools::default_for_agent(handle);
        BuiltinTools {
            manage_archival_memory_tool: self
                .manage_archival_memory_tool
                .unwrap_or(defaults.manage_archival_memory_tool),
            manage_core_memory_tool: self
                .manage_core_memory_tool
                .unwrap_or(defaults.manage_core_memory_tool),
            search_conversations_tool: self
                .search_conversations_tool
                .unwrap_or(defaults.search_conversations_tool),
            send_message_tool: self.send_message_tool.unwrap_or(defaults.send_message_tool),
        }
    }
}

/// Trait for custom memory backends
pub trait MemoryBackend: Send + Sync {
    /// Update a memory block's value
    fn update_block(&self, label: &str, value: String) -> crate::Result<()>;

    /// Get a memory block's value
    fn get_block(&self, label: &str) -> crate::Result<Option<String>>;

    /// List all memory block labels
    fn list_blocks(&self) -> Vec<String>;
}

/// Trait for custom message sending backends
#[async_trait::async_trait]
pub trait MessageSender: Send + Sync {
    /// Send a message to a target
    async fn send_message(&self, target: MessageTarget, content: String) -> crate::Result<()>;
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[schemars(inline)]
pub struct MessageTarget {
    pub target_type: TargetType,
    #[schemars(default, with = "String")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[schemars(inline)]
pub enum TargetType {
    User,
    Agent,
    Group,
    Channel,
}

#[cfg(test)]
mod tests;
