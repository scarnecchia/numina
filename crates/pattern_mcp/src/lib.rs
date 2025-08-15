//! Pattern MCP - Model Context Protocol Client and Server
//!
//! This crate provides both MCP client and server implementations:
//! - Client: Connect to external MCP servers and consume their tools
//! - Server: Expose Pattern's agent capabilities through MCP

pub mod client;
pub mod error;
pub mod registry;
pub mod server;
pub mod transport;

pub use error::{McpError, Result};
pub use registry::{ToolRegistry, ToolRegistryBuilder};
pub use server::{McpServer, McpServerBuilder};
pub use transport::{Transport, TransportType};

// Client exports
pub use client::{
    AuthConfig, McpClientService, McpServerConfig, McpToolWrapper, ToolRequest, ToolResponse,
    TransportConfig,
};

/// Re-export commonly used types
pub mod prelude {
    pub use crate::{
        // Client types
        AuthConfig,
        McpClientService,
        // Server types
        McpServer,
        McpServerBuilder,
        McpServerConfig,
        McpToolWrapper,
        ToolRegistry,
        ToolRegistryBuilder,
        Transport,
        TransportConfig,
        TransportType,
        // Common types
        error::{McpError, Result},
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
