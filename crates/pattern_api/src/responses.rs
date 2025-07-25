//! API response types

use crate::{ApiResponse, PaginatedResponse};
use pattern_core::{
    agent::{AgentState, AgentType},
    coordination::{CoordinationPattern, GroupMemberRole},
    id::{AgentId, GroupId, MessageId, UserId},
    message::{ChatRole, Message, MessageContent},
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Authentication response
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AuthResponse {
    /// Access token for API requests
    pub access_token: String,
    /// Refresh token for getting new access tokens
    pub refresh_token: String,
    /// Token type (usually "Bearer")
    pub token_type: String,
    /// Expiration time in seconds
    pub expires_in: u64,
    /// User information
    pub user: UserResponse,
}

/// User response
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct UserResponse {
    pub id: UserId,
    pub username: String,
    pub display_name: Option<String>,
    pub email: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
    pub is_active: bool,
}

/// Agent response
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AgentResponse {
    pub id: AgentId,
    pub name: String,
    pub agent_type: AgentType,
    pub description: Option<String>,
    pub system_prompt: Option<String>,
    pub state: AgentState,
    pub model_provider: String,
    pub model_id: String,
    pub tools: Vec<String>,
    pub owner_id: UserId,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
    pub last_active_at: Option<chrono::DateTime<chrono::Utc>>,
    pub message_count: u64,
    pub metadata: Option<serde_json::Value>,
}

/// Group response
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct GroupResponse {
    pub id: GroupId,
    pub name: String,
    pub description: String,
    pub coordination_pattern: CoordinationPattern,
    pub owner_id: UserId,
    pub member_count: u32,
    pub is_active: bool,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
    pub last_active_at: Option<chrono::DateTime<chrono::Utc>>,
}

/// Group member response
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct GroupMemberResponse {
    pub agent_id: AgentId,
    pub agent_name: String,
    pub role: GroupMemberRole,
    pub capabilities: Vec<String>,
    pub joined_at: chrono::DateTime<chrono::Utc>,
    pub is_active: bool,
    pub metadata: Option<serde_json::Value>,
}

/// Group with members response
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct GroupWithMembersResponse {
    #[serde(flatten)]
    pub group: GroupResponse,
    pub members: Vec<GroupMemberResponse>,
}

/// Message response
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct MessageResponse {
    pub id: MessageId,
    pub agent_id: AgentId,
    pub role: ChatRole,
    pub content: MessageContent,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub tool_calls: Option<Vec<ToolCallResponse>>,
    pub tool_results: Option<Vec<ToolResultResponse>>,
    pub metadata: Option<serde_json::Value>,
}

/// Tool call response
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ToolCallResponse {
    pub id: String,
    pub name: String,
    pub arguments: serde_json::Value,
}

/// Tool result response  
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ToolResultResponse {
    pub tool_call_id: String,
    pub result: serde_json::Value,
    pub is_error: bool,
}

/// Chat response
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ChatResponse {
    pub messages: Vec<MessageResponse>,
    pub usage: Option<UsageInfo>,
}

/// Usage information for model calls
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct UsageInfo {
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub total_tokens: u32,
    pub model: String,
}

/// Memory response
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct MemoryResponse {
    pub agent_id: AgentId,
    pub memory_type: String,
    pub content: serde_json::Value,
    pub updated_at: chrono::DateTime<chrono::Utc>,
    pub version: u32,
}

/// Archival memory item
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ArchivalMemoryItem {
    pub id: String,
    pub label: String,
    pub content: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub metadata: Option<serde_json::Value>,
}

/// Stats response
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct StatsResponse {
    pub total_users: u64,
    pub active_users: u64,
    pub total_agents: u64,
    pub active_agents: u64,
    pub total_groups: u64,
    pub total_messages: u64,
    pub messages_today: u64,
    pub messages_this_week: u64,
    pub messages_this_month: u64,
}

/// Health check response
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct HealthResponse {
    pub status: HealthStatus,
    pub version: String,
    pub uptime_seconds: u64,
    pub database_status: ComponentStatus,
    pub services: Vec<ServiceStatus>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum HealthStatus {
    Healthy,
    Degraded,
    Unhealthy,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum ComponentStatus {
    Ok,
    Warning,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ServiceStatus {
    pub name: String,
    pub status: ComponentStatus,
    pub message: Option<String>,
}

/// Batch operation response
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct BatchResponse<T> {
    pub successful: Vec<BatchResult<T>>,
    pub failed: Vec<BatchError>,
    pub total_operations: u32,
    pub successful_count: u32,
    pub failed_count: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct BatchResult<T> {
    pub index: u32,
    pub result: T,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct BatchError {
    pub index: u32,
    pub error: String,
}
