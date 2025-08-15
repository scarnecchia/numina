//! Tool discovery for MCP servers

use crate::Result;
use rmcp::service::{Peer, RoleClient};
use serde::{Deserialize, Serialize};

/// Tool discovery interface for MCP servers
pub struct ToolDiscovery;

impl ToolDiscovery {
    /// Discover tools from an MCP server
    pub async fn discover_tools(peer: &Peer<RoleClient>) -> Result<Vec<ToolInfo>> {
        let response = peer
            .list_tools(None)
            .await
            .map_err(|e| crate::error::McpError::transport_init("list_tools", "peer", e))?;

        let tools = response
            .tools
            .into_iter()
            .map(|tool| ToolInfo {
                name: tool.name.to_string(),
                description: tool.description.unwrap_or_default().to_string(),
                input_schema: serde_json::Value::Object((*tool.input_schema).clone()),
            })
            .collect();

        Ok(tools)
    }
}

/// Information about a discovered tool
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolInfo {
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
}
