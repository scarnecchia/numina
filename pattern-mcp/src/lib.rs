//! Pattern MCP - Model Context Protocol Server
//!
//! This crate provides the MCP server implementation that exposes
//! Pattern's agent capabilities through the Model Context Protocol.

pub mod registry;
pub mod server;
pub mod transport;

pub use registry::{ToolRegistry, ToolRegistryBuilder};
pub use server::{McpServer, McpServerBuilder};
pub use transport::{Transport, TransportType};

/// MCP-specific error types
pub mod error {
    use thiserror::Error;

    #[derive(Error, Debug)]
    pub enum McpError {
        #[error("Transport error: {0}")]
        Transport(String),

        #[error("Protocol error: {0}")]
        Protocol(String),

        #[error("Tool execution error: {0}")]
        ToolExecution(String),

        #[error("Serialization error: {0}")]
        Serialization(#[from] serde_json::Error),

        #[error("Server error: {0}")]
        Server(String),
    }

    pub type Result<T> = std::result::Result<T, McpError>;
}

/// Re-export commonly used types
pub mod prelude {
    pub use crate::{
        error::{McpError, Result},
        McpServer, McpServerBuilder, ToolRegistry, ToolRegistryBuilder, Transport, TransportType,
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
