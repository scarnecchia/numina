//! Pattern Core - Agent Framework and Memory System
//!
//! This crate provides the core agent framework, memory management,
//! and tool execution system that powers Pattern's multi-agent
//! cognitive support system.

pub mod agent;
pub mod config;
pub mod context;
pub mod coordination;
pub mod db;
pub mod embeddings;
pub mod error;
//pub mod export;
pub mod id;
pub mod memory;
pub mod message;
pub mod model;
pub mod realtime;
pub mod tool;
pub mod users;
pub mod utils;

// Macros are automatically available at crate root due to #[macro_export]

pub use agent::{Agent, AgentState, AgentType};
pub use context::{
    AgentContext, AgentContextBuilder, CompressionStrategy, ContextBuilder, ContextConfig,
    MemoryContext, MessageCompressor,
};
pub use coordination::{AgentGroup, Constellation, CoordinationPattern};
pub use error::{CoreError, Result};
pub use id::{
    AgentId, ConversationId, Id, IdType, MemoryId, MessageId, MessageIdType, ModelId, RequestId,
    SessionId, TaskId, ToolCallId, UserId,
};
pub use memory::{Memory, MemoryBlock};
pub use model::ModelCapability;
pub use model::ModelProvider;
pub use tool::{AiTool, DynamicTool, ToolRegistry, ToolResult};

/// Re-export commonly used types
pub mod prelude {
    pub use crate::{
        Agent, AgentContext, AgentContextBuilder, AgentId, AgentState, AgentType, AiTool,
        CompressionStrategy, Constellation, ContextBuilder, ContextConfig, CoordinationPattern,
        CoreError, DynamicTool, Id, IdType, Memory, MemoryBlock, MemoryContext, MessageCompressor,
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
