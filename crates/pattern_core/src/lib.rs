//! Pattern Core - Agent Framework and Memory System
//!
//! This crate provides the core agent framework, memory management,
//! and tool execution system that powers Pattern's multi-agent
//! cognitive support system.

pub mod agent;
pub mod context;
pub mod coordination;
pub mod db;
pub mod error;
pub mod memory;
pub mod model;
pub mod realtime;
pub mod tool;
pub mod utils;

pub use agent::{Agent, AgentBuilder, AgentId, AgentType};
pub use coordination::{AgentGroup, Constellation, CoordinationPattern};
pub use error::{CoreError, Result};
pub use memory::{Memory, MemoryBlock, MemoryStore};
pub use model::{ModelCapability, ModelProvider};
pub use tool::{AiTool, DynamicTool, ToolRegistry, ToolResult};

/// Re-export commonly used types
pub mod prelude {
    pub use crate::{
        Agent, AgentBuilder, AgentId, AgentType, AiTool, Constellation, CoordinationPattern,
        CoreError, DynamicTool, Memory, MemoryBlock, ModelCapability, ModelProvider, Result,
        ToolRegistry, ToolResult,
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_works() {
        // Basic smoke test
        assert_eq!(2 + 2, 4);
    }
}
