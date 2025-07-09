//! Built-in tools for agents
//!
//! This module provides the standard tools that all agents have access to,
//! including memory management and inter-agent communication.

mod memory;
mod send_message;

pub use memory::UpdateMemoryTool;
pub use send_message::SendMessageTool;

use crate::{
    context::AgentHandle,
    tool::{DynamicTool, DynamicToolAdapter, ToolRegistry},
};

/// Registry specifically for built-in tools
#[derive(Clone)]
pub struct BuiltinTools {
    memory_tool: Box<dyn DynamicTool>,
    send_message_tool: Box<dyn DynamicTool>,
}

impl BuiltinTools {
    /// Create default built-in tools for an agent
    pub fn default_for_agent(handle: AgentHandle) -> Self {
        Self {
            memory_tool: Box::new(DynamicToolAdapter::new(UpdateMemoryTool {
                handle: handle.clone(),
            })),
            send_message_tool: Box::new(DynamicToolAdapter::new(SendMessageTool {
                handle: handle.clone(),
            })),
        }
    }

    /// Register all tools to a registry
    pub fn register_all(&self, registry: &ToolRegistry) {
        registry.register_dynamic(self.memory_tool.clone_box());
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
    memory_tool: Option<Box<dyn DynamicTool>>,
    send_message_tool: Option<Box<dyn DynamicTool>>,
}

impl BuiltinToolsBuilder {
    /// Replace the default memory tool
    pub fn with_memory_tool(mut self, tool: impl DynamicTool + 'static) -> Self {
        self.memory_tool = Some(Box::new(tool));
        self
    }

    /// Replace the default send_message tool
    pub fn with_send_message_tool(mut self, tool: impl DynamicTool + 'static) -> Self {
        self.send_message_tool = Some(Box::new(tool));
        self
    }

    /// Build the tools for a specific agent
    pub fn build_for_agent(self, handle: AgentHandle) -> BuiltinTools {
        let defaults = BuiltinTools::default_for_agent(handle);
        BuiltinTools {
            memory_tool: self.memory_tool.unwrap_or(defaults.memory_tool),
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

/// Target for sending messages
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum MessageTarget {
    /// Send to the user
    User,

    /// Send to another agent
    Agent { agent_id: String },

    /// Send to a group of agents
    Group { group_id: String },

    /// Send to a specific channel/platform
    Channel { channel_id: String },
}

#[cfg(test)]
mod tests;
