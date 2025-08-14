//! MCP Client implementation for consuming external tools
//!
//! This module provides a client that can connect to MCP servers,
//! discover their tools, and expose them as Pattern DynamicTools.

mod discovery;
mod service;
mod tool_wrapper;
mod transport;

pub use discovery::ToolDiscovery;
pub use service::{McpClientService, McpServerConfig};
pub use tool_wrapper::McpToolWrapper;
pub use transport::{AuthConfig, ClientTransport, TransportConfig};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Request sent from tool wrapper to client service
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolRequest {
    /// Unique request identifier
    pub id: String,
    /// Name of the tool to invoke
    pub tool: String,
    /// Parameters for the tool
    pub params: serde_json::Value,
}

impl ToolRequest {
    /// Create a new tool request with a unique ID
    pub fn new(tool: String, params: serde_json::Value) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            tool,
            params,
        }
    }
}

/// Response sent from client service to tool wrappers
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResponse {
    /// Request ID this response is for
    pub request_id: String,
    /// Result of the tool invocation
    pub result: std::result::Result<serde_json::Value, String>, // String for serializable errors
}

impl ToolResponse {
    /// Create a successful response
    pub fn success(request_id: String, value: serde_json::Value) -> Self {
        Self {
            request_id,
            result: Ok(value),
        }
    }

    /// Create an error response
    pub fn error(request_id: String, error: String) -> Self {
        Self {
            request_id,
            result: Err(error),
        }
    }
}