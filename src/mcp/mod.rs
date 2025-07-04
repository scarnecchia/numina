//! MCP server and tools

use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

pub mod core_tools;
pub mod discord_tools;
pub mod server;

/// MCP transport type
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum McpTransport {
    Stdio,
    Sse,
    #[serde(rename = "http")]
    Http,
}

impl fmt::Display for McpTransport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            McpTransport::Stdio => write!(f, "stdio"),
            McpTransport::Sse => write!(f, "sse"),
            McpTransport::Http => write!(f, "http"),
        }
    }
}

impl FromStr for McpTransport {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "stdio" => Ok(McpTransport::Stdio),
            "sse" => Ok(McpTransport::Sse),
            "http" => Ok(McpTransport::Http),
            _ => Err(format!("Unknown MCP transport: {}", s)),
        }
    }
}

// MCP request types
#[derive(Debug, Deserialize, Serialize, schemars::JsonSchema)]
pub struct ChatWithAgentRequest {
    pub user_id: i64,
    pub message: String,
    pub agent_id: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, schemars::JsonSchema)]
pub struct GetAgentMemoryRequest {
    pub user_id: i64,
    pub agent_id: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, schemars::JsonSchema)]
pub struct UpdateAgentMemoryRequest {
    pub user_id: i64,
    pub memory_json: String,
    pub agent_id: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, schemars::JsonSchema)]
pub struct ScheduleEventRequest {
    pub user_id: i64,
    pub title: String,
    pub start_time: String,
    pub description: Option<String>,
    pub duration_minutes: Option<u32>,
}

#[derive(Debug, Deserialize, Serialize, schemars::JsonSchema)]
pub struct UpdateAgentModelRequest {
    pub user_id: i64,
    pub agent_id: String,
    pub capability: crate::agent::ModelCapability,
}

// Group-related request types
#[derive(Debug, Deserialize, Serialize, schemars::JsonSchema)]
pub struct SendGroupMessageRequest {
    pub user_id: i64,
    pub group_name: String,
    pub message: String,
}

// Unified message sending
#[derive(Debug, Deserialize, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum MessageDestinationType {
    Agent,
    Group,
    #[cfg(feature = "discord")]
    DiscordChannel,
    #[cfg(feature = "discord")]
    DiscordDm,
}

#[derive(Debug, Deserialize, Serialize, schemars::JsonSchema)]
pub struct SendMessageRequest {
    pub user_id: i64,
    pub destination_type: MessageDestinationType,
    pub destination: String,
    pub message: String,
    // Optional fields for Discord embeds
    #[serde(skip_serializing_if = "Option::is_none")]
    pub embed_title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub embed_color: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub embed_fields: Option<Vec<discord_tools::EmbedField>>,
}
