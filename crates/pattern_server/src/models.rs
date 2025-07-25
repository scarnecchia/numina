//! Server-specific data models

use chrono::{DateTime, Utc};
use pattern_core::{
    define_id_type,
    id::{AgentId, EventId, Id, MemoryId, TaskId, UserId},
};
use pattern_macros::Entity;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// Define ID types for server entities
define_id_type!(ApiKeyIdType, "apikey");
define_id_type!(RefreshTokenFamilyIdType, "rtfam");

pub type ApiKeyId = Id<ApiKeyIdType>;
pub type RefreshTokenFamilyId = Id<RefreshTokenFamilyIdType>;

/// Server-side user model with authentication fields
/// Extends pattern_core::User with auth-specific fields
#[derive(Debug, Clone, Entity, Serialize, Deserialize)]
#[entity(entity_type = "user", crate_path = "::pattern_core")]
pub struct ServerUser {
    // Core user fields (from pattern_core::User)
    pub id: UserId,
    pub discord_id: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    #[serde(default)]
    pub settings: HashMap<String, serde_json::Value>,
    #[serde(default)]
    pub metadata: HashMap<String, serde_json::Value>,

    // Server-specific auth fields
    pub username: String,
    pub password_hash: String,
    pub email: Option<String>,
    pub display_name: Option<String>,
    pub is_active: bool,

    // Relations from pattern_core::User
    #[entity(relation = "owns")]
    pub owned_agent_ids: Vec<AgentId>,

    #[entity(relation = "created")]
    pub created_task_ids: Vec<TaskId>,

    #[entity(relation = "remembers")]
    pub memory_ids: Vec<MemoryId>,

    #[entity(relation = "scheduled")]
    pub scheduled_event_ids: Vec<EventId>,

    // Server-specific relations
    #[entity(relation = "owns")]
    pub api_keys: Vec<ApiKeyId>,

    #[entity(relation = "owns")]
    pub refresh_token_families: Vec<RefreshTokenFamilyId>,
}

/// Database record for API keys
#[derive(Debug, Clone, Entity, Serialize, Deserialize)]
#[entity(entity_type = "api_key", crate_path = "::pattern_core")]
pub struct ApiKey {
    pub id: ApiKeyId,
    pub user_id: UserId,
    pub key_hash: String,
    pub name: String,
    pub permissions: Vec<String>,
    pub last_used_at: Option<DateTime<Utc>>,
    pub expires_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub is_active: bool,
}

/// Database record for refresh token families
#[derive(Debug, Clone, Entity, Serialize, Deserialize)]
#[entity(entity_type = "refresh_token_family", crate_path = "::pattern_core")]
pub struct RefreshTokenFamily {
    pub id: RefreshTokenFamilyId,
    pub user_id: UserId,
    pub family_id: uuid::Uuid,
    pub created_at: DateTime<Utc>,
    pub revoked_at: Option<DateTime<Utc>>,
    pub is_active: bool,
}
