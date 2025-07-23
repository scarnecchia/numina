//! Pattern MCP - Model Context Protocol Server
//!
//! This crate provides the MCP server implementation that exposes
//! Pattern's agent capabilities through the Model Context Protocol.

pub mod error;
pub mod registry;
pub mod server;
pub mod transport;

pub use error::{McpError, Result};
pub use registry::{ToolRegistry, ToolRegistryBuilder};
pub use server::{McpServer, McpServerBuilder};
pub use transport::{Transport, TransportType};

/// Re-export commonly used types
pub mod prelude {
    pub use crate::{
        McpServer, McpServerBuilder, ToolRegistry, ToolRegistryBuilder, Transport, TransportType,
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
