//! Pattern Core - Agent Framework and Memory System
//!
//! This crate provides the core agent framework, memory management,
//! and tool execution system that powers Pattern's multi-agent
//! cognitive support system.

pub mod agent;
pub mod context;
pub mod coordination;
pub mod db;
pub mod embeddings;
pub mod error;
pub mod id;
pub mod memory;
pub mod model;
pub mod realtime;
pub mod tool;
pub mod utils;

pub use agent::{Agent, AgentBuilder, AgentType};
pub use context::{
    AgentContext, AgentState, AgentStateBuilder, CompressionStrategy, ContextBuilder,
    ContextConfig, MessageCompressor,
};
pub use coordination::{AgentGroup, Constellation, CoordinationPattern};
pub use error::{CoreError, Result};
pub use id::{
    AgentId, ConversationId, Id, IdType, MemoryId, MessageId, SessionId, TaskId, ToolCallId, UserId,
};
pub use memory::{Memory, MemoryBlock};
pub use model::{ModelCapability, ModelProvider};
pub use tool::{AiTool, DynamicTool, ToolRegistry, ToolResult};

/// Re-export commonly used types
pub mod prelude {
    pub use crate::{
        Agent, AgentBuilder, AgentContext, AgentId, AgentState, AgentStateBuilder, AgentType,
        AiTool, CompressionStrategy, Constellation, ContextBuilder, ContextConfig,
        CoordinationPattern, CoreError, DynamicTool, Id, IdType, Memory, MemoryBlock,
        MessageCompressor, ModelCapability, ModelProvider, Result, ToolRegistry, ToolResult,
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
