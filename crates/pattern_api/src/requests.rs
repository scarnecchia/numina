//! API request types

use crate::PaginationParams;
use pattern_core::{
    agent::{AgentState, AgentType},
    coordination::{CoordinationPattern, GroupMemberRole},
    id::{AgentId, GroupId, MessageId, UserId},
    message::{ChatRole, MessageContent, MessageOptions},
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Authentication request
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
pub enum AuthRequest {
    /// Login with username/password
    Password { username: String, password: String },
    /// Login with API key
    ApiKey { api_key: String },
    /// Refresh access token
    RefreshToken { refresh_token: String },
}

/// User creation request
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CreateUserRequest {
    pub username: String,
    pub password: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
}

/// User update request
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct UpdateUserRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub password: Option<String>,
}

/// Agent creation request
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CreateAgentRequest {
    pub name: String,
    pub agent_type: AgentType,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system_prompt: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_provider: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
}

/// Agent update request
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct UpdateAgentRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system_prompt: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state: Option<AgentState>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_provider: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
}

/// Group creation request
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CreateGroupRequest {
    pub name: String,
    pub description: String,
    pub coordination_pattern: CoordinationPattern,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub members: Option<Vec<GroupMemberRequest>>,
}

/// Group member addition request
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct GroupMemberRequest {
    pub agent_id: AgentId,
    pub role: GroupMemberRole,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub capabilities: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
}

/// Group update request
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct UpdateGroupRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub coordination_pattern: Option<CoordinationPattern>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_active: Option<bool>,
}

/// Chat message request
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SendMessageRequest {
    /// Target agent or group ID
    pub target_id: String,
    /// Whether target is a group (true) or agent (false)
    pub is_group: bool,
    /// Message content
    pub content: MessageContent,
    /// Optional message options
    #[serde(skip_serializing_if = "Option::is_none")]
    pub options: Option<MessageOptions>,
}

/// Message search request
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SearchMessagesRequest {
    /// Text to search for
    #[serde(skip_serializing_if = "Option::is_none")]
    pub query: Option<String>,
    /// Filter by agent ID
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<AgentId>,
    /// Filter by role
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<ChatRole>,
    /// Filter by date range (from)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from_date: Option<chrono::DateTime<chrono::Utc>>,
    /// Filter by date range (to)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub to_date: Option<chrono::DateTime<chrono::Utc>>,
    /// Include tool calls in results
    #[serde(default)]
    pub include_tool_calls: bool,
    /// Pagination
    #[serde(flatten)]
    pub pagination: PaginationParams,
}

/// Agent memory update request
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct UpdateMemoryRequest {
    pub memory_key: String,
    pub operation: MemoryOperation,
}

/// Memory operation types
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum MemoryOperation {
    /// Append content to memory
    Append { content: String },
    /// Replace memory content
    Replace {
        old_content: String,
        new_content: String,
    },
    /// Archive memory
    Archive {
        #[serde(skip_serializing_if = "Option::is_none")]
        label: Option<String>,
    },
    /// Load from archival
    LoadFromArchival { label: String },
    /// Swap memories
    Swap {
        archive_key: String,
        load_label: String,
    },
}

/// Batch operation request
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct BatchRequest<T> {
    pub operations: Vec<T>,
    /// Whether to stop on first error
    #[serde(default)]
    pub stop_on_error: bool,
}

/// List query parameters
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ListQueryParams {
    /// Filter by active/inactive status
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_active: Option<bool>,
    /// Sort field
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sort_by: Option<String>,
    /// Sort direction
    #[serde(default)]
    pub sort_desc: bool,
    /// Pagination
    #[serde(flatten)]
    pub pagination: PaginationParams,
}
