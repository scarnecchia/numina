//! Built-in tools for agents
//!
//! This module provides the standard tools that all agents have access to,
//! including memory management and inter-agent communication.

mod context;
pub mod data_source;
mod recall;
mod search;
mod send_message;
#[cfg(test)]
mod test_schemas;

use std::fmt::Debug;

pub use context::{ContextInput, ContextOutput, ContextTool, CoreMemoryOperationType};
pub use data_source::{
    DataSourceInput, DataSourceOutput, DataSourceTool, register_data_source_tool,
};
pub use recall::{
    ArchivalMemoryOperationType, ArchivalSearchResult, RecallInput, RecallOutput, RecallTool,
};
use schemars::JsonSchema;
pub use search::{SearchDomain, SearchInput, SearchOutput, SearchTool};

pub use send_message::SendMessageTool;
use serde::{Deserialize, Serialize};

use crate::{
    context::AgentHandle,
    tool::{DynamicTool, DynamicToolAdapter, ToolRegistry},
};

/// Registry specifically for built-in tools
#[derive(Clone)]
pub struct BuiltinTools {
    recall_tool: Box<dyn DynamicTool>,
    context_tool: Box<dyn DynamicTool>,
    search_tool: Box<dyn DynamicTool>,
    send_message_tool: Box<dyn DynamicTool>,
}

impl BuiltinTools {
    /// Create default built-in tools for an agent
    pub fn default_for_agent<C: surrealdb::Connection + Debug + Clone>(
        handle: AgentHandle<C>,
    ) -> Self {
        Self {
            recall_tool: Box::new(DynamicToolAdapter::new(RecallTool {
                handle: handle.clone(),
            })),
            context_tool: Box::new(DynamicToolAdapter::new(ContextTool {
                handle: handle.clone(),
            })),
            search_tool: Box::new(DynamicToolAdapter::new(SearchTool {
                handle: handle.clone(),
            })),

            send_message_tool: Box::new(DynamicToolAdapter::new(SendMessageTool {
                handle: handle.clone(),
            })),
        }
    }

    /// Register all tools to a registry
    pub fn register_all(&self, registry: &ToolRegistry) {
        registry.register_dynamic(self.recall_tool.clone_box());
        registry.register_dynamic(self.context_tool.clone_box());
        registry.register_dynamic(self.search_tool.clone_box());
        registry.register_dynamic(self.send_message_tool.clone_box());

        // Note: DataSourceTool requires external coordinator setup.
        // Use register_data_source_tool() function directly when you have a coordinator.
    }

    /// Builder pattern for customization
    pub fn builder() -> BuiltinToolsBuilder {
        BuiltinToolsBuilder::default()
    }
}

/// Builder for customizing built-in tools
#[derive(Default)]
pub struct BuiltinToolsBuilder {
    recall_tool: Option<Box<dyn DynamicTool>>,
    context_tool: Option<Box<dyn DynamicTool>>,
    search_tool: Option<Box<dyn DynamicTool>>,
    send_message_tool: Option<Box<dyn DynamicTool>>,
}

impl BuiltinToolsBuilder {
    /// Replace the default manage archival memory tool
    pub fn with_recall_tool(mut self, tool: impl DynamicTool + 'static) -> Self {
        self.recall_tool = Some(Box::new(tool));
        self
    }

    /// Replace the default manage core memory tool
    pub fn with_context_tool(mut self, tool: impl DynamicTool + 'static) -> Self {
        self.context_tool = Some(Box::new(tool));
        self
    }

    /// Replace the default search tool
    pub fn with_search_tool(mut self, tool: impl DynamicTool + 'static) -> Self {
        self.search_tool = Some(Box::new(tool));
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
            recall_tool: self.recall_tool.unwrap_or(defaults.recall_tool),
            context_tool: self.context_tool.unwrap_or(defaults.context_tool),
            search_tool: self.search_tool.unwrap_or(defaults.search_tool),
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
    Bluesky,
}

#[cfg(test)]
mod tests;
