//! Pattern Core - Agent Framework and Memory System
//!
//! This crate provides the core agent framework, memory management,
//! and tool execution system that powers Pattern's multi-agent
//! cognitive support system.

#![recursion_limit = "256"]

pub mod agent;
pub mod atproto_identity;
pub mod config;
pub mod constellation_memory;
pub mod context;
pub mod coordination;
pub mod data_source;
pub mod db;
pub mod discord_identity;
pub mod embeddings;
pub mod error;
pub mod export;
pub mod id;
pub mod memory;
pub mod message;
pub mod message_queue;
pub mod model;
pub mod oauth;
pub mod prompt_template;
pub mod realtime;
pub mod tool;
pub mod users;
pub mod utils;

// Macros are automatically available at crate root due to #[macro_export]

pub use crate::agent::SnowflakePosition;
pub use agent::{Agent, AgentState, AgentType};
pub use context::{
    AgentContext, AgentContextBuilder, CompressionStrategy, ContextBuilder, ContextConfig,
    MemoryContext, MessageCompressor,
};
pub use coordination::{AgentGroup, Constellation, CoordinationPattern};
pub use error::{CoreError, Result};
pub use id::{
    AgentId, AtprotoIdentityId, ConversationId, Did, IdType, MemoryId, MessageId, ModelId,
    OAuthTokenId, QueuedMessageId, RequestId, SessionId, TaskId, ToolCallId, UserId, WakeupId,
};
pub use memory::{Memory, MemoryBlock};
pub use message_queue::{QueuedMessage, ScheduledWakeup};
pub use model::ModelCapability;
pub use model::ModelProvider;
pub use tool::{AiTool, DynamicTool, ToolRegistry, ToolResult};
/// Re-export commonly used types
pub mod prelude {
    pub use crate::{
        Agent, AgentContext, AgentContextBuilder, AgentId, AgentState, AgentType, AiTool,
        CompressionStrategy, Constellation, ContextBuilder, ContextConfig, CoordinationPattern,
        CoreError, DynamicTool, IdType, Memory, MemoryBlock, MemoryContext, MessageCompressor,
        ModelCapability, ModelProvider, Result, ToolRegistry, ToolResult,
    };
}

#[cfg(test)]
mod tests {

    #[test]
    fn it_works() {
        // Basic smoke test
        assert_eq!(2 + 2, 4);
    }
}
