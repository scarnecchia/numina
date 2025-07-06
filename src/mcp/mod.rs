//! MCP server and tools

use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

pub mod core_tools;
pub mod discord_tools;
pub mod knowledge_tools;
pub mod server;
pub mod sleeptime_tools;

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
    pub user_id: String,
    pub message: String,
    pub agent_id: Option<String>,
    /// Whether to request another agent turn after this tool completes
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_heartbeat: Option<bool>,
}

#[derive(Debug, Deserialize, Serialize, schemars::JsonSchema)]
pub struct GetAgentMemoryRequest {
    pub user_id: String,
    pub agent_id: Option<String>,
    /// Whether to request another agent turn after this tool completes
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_heartbeat: Option<bool>,
}

#[derive(Debug, Deserialize, Serialize, schemars::JsonSchema)]
pub struct UpdateAgentMemoryRequest {
    pub user_id: String,
    pub memory_json: String,
    pub agent_id: Option<String>,
    /// Whether to request another agent turn after this tool completes
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_heartbeat: Option<bool>,
}

#[derive(Debug, Deserialize, Serialize, schemars::JsonSchema)]
pub struct ScheduleEventRequest {
    pub user_id: String,
    pub title: String,
    pub start_time: String,
    pub end_time: String,
    pub description: Option<String>,
    pub location: Option<String>,
    /// Whether to request another agent turn after this tool completes
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_heartbeat: Option<bool>,
}

#[derive(Debug, Deserialize, Serialize, schemars::JsonSchema)]
pub struct UpdateAgentModelRequest {
    pub user_id: String,
    pub agent_id: String,
    /// Model capability level: "routine", "interactive", "investigative", or "critical"
    pub capability: String,
    /// Whether to request another agent turn after this tool completes
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_heartbeat: Option<bool>,
}

#[derive(Debug, Deserialize, Serialize, schemars::JsonSchema)]
pub struct RecordEnergyStateRequest {
    pub user_id: String,
    pub energy_level: i32,       // 1-10
    pub attention_state: String, // focused, scattered, hyperfocus, etc
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mood: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_break_minutes: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    /// Whether to request another agent turn after this tool completes
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_heartbeat: Option<bool>,
}

// Group-related request types
#[derive(Debug, Deserialize, Serialize, schemars::JsonSchema)]
pub struct SendGroupMessageRequest {
    pub user_id: String,
    pub group_name: String,
    pub message: String,
    /// Whether to request another agent turn after this tool completes
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_heartbeat: Option<bool>,
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

impl FromStr for MessageDestinationType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "agent" => Ok(MessageDestinationType::Agent),
            "group" => Ok(MessageDestinationType::Group),
            #[cfg(feature = "discord")]
            "discord_channel" => Ok(MessageDestinationType::DiscordChannel),
            #[cfg(feature = "discord")]
            "discord_dm" => Ok(MessageDestinationType::DiscordDm),
            _ => Err(format!("Unknown destination type: {}", s)),
        }
    }
}

#[derive(Debug, Deserialize, Serialize, schemars::JsonSchema)]
pub struct SendMessageRequest {
    pub user_id: String,
    /// Destination type: "agent", "group", "discord_channel", or "discord_dm"
    pub destination_type: String,
    pub destination: String,
    pub message: String,
    // Optional fields for Discord embeds
    #[serde(skip_serializing_if = "Option::is_none")]
    pub embed_title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub embed_color: Option<i64>,
    // Note: embed_fields removed to avoid JSON Schema $ref issues with Gemini
    /// Whether to request another agent turn after this tool completes
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_heartbeat: Option<bool>,
}

/// Empty request type for tools that don't take parameters
#[derive(Debug, Deserialize, Serialize, schemars::JsonSchema)]
pub struct EmptyRequest {
    /// Whether to request another agent turn after this tool completes
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_heartbeat: Option<bool>,
}
