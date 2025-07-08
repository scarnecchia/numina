//! Database model wrappers
//!
//! These types mirror the domain types but use RecordId for database operations.
//! This allows us to maintain type safety in the domain while working with
//! SurrealDB's record ID format.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, str::FromStr};
use surrealdb::RecordId;
use uuid::Uuid;

use crate::{
    agent::{AgentState, AgentType},
    id::{AgentId, MemoryId, UserId},
};

/// Database representation of a User
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DbUser {
    pub id: RecordId,
    pub created_at: surrealdb::Datetime,
    pub updated_at: surrealdb::Datetime,
    #[serde(default)]
    pub settings: HashMap<String, serde_json::Value>,
    #[serde(default)]
    pub metadata: HashMap<String, serde_json::Value>,
}

/// Database representation of an Agent
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DbAgent {
    pub id: RecordId,
    pub user_id: RecordId,
    pub agent_type: AgentType,
    pub name: String,
    pub system_prompt: String,
    #[serde(default)]
    pub config: serde_json::Value,
    pub state: AgentState,
    pub created_at: surrealdb::Datetime,
    pub updated_at: surrealdb::Datetime,
    #[serde(default = "default_true")]
    pub is_active: bool,
}

/// Database representation of a MemoryBlock
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DbMemoryBlock {
    pub id: RecordId,
    pub agent_id: RecordId,
    pub label: String,
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub embedding: Option<Vec<f32>>,
    pub embedding_model: Option<String>,
    #[serde(default)]
    pub metadata: serde_json::Value,
    pub created_at: surrealdb::Datetime,
    pub updated_at: surrealdb::Datetime,
}

fn default_true() -> bool {
    true
}

pub fn from_surreal_datetime(dt: surrealdb::Datetime) -> DateTime<Utc> {
    println!("datetime: {}", strip_dt(&dt.to_string()));
    let datetime = chrono::NaiveDateTime::parse_from_str(&dt.to_string(), "d'%FT%T%.6fZ'")
        .expect("should be valid ISO-8601");

    datetime.and_utc()
}

// Conversion implementations

impl From<crate::db::schema::User> for DbUser {
    fn from(user: crate::db::schema::User) -> Self {
        Self {
            id: RecordId::from(user.id),
            created_at: user.created_at.into(),
            updated_at: user.updated_at.into(),
            settings: user.settings,
            metadata: user.metadata,
        }
    }
}

impl TryFrom<DbUser> for crate::db::schema::User {
    type Error = crate::id::IdError;

    fn try_from(db_user: DbUser) -> Result<Self, Self::Error> {
        let user_id = UserId::from_uuid(
            Uuid::from_str(strip_brackets(&db_user.id.key().to_string())).unwrap(),
        );

        Ok(Self {
            id: user_id,
            created_at: from_surreal_datetime(db_user.created_at),
            updated_at: from_surreal_datetime(db_user.updated_at),
            settings: db_user.settings,
            metadata: db_user.metadata,
        })
    }
}

impl From<crate::db::schema::Agent> for DbAgent {
    fn from(agent: crate::db::schema::Agent) -> Self {
        Self {
            id: RecordId::from(agent.id),
            user_id: agent.user_id.into(),
            agent_type: agent.agent_type,
            name: agent.name,
            system_prompt: agent.system_prompt,
            config: agent.config,
            state: agent.state,
            created_at: agent.created_at.into(),
            updated_at: agent.updated_at.into(),
            is_active: agent.is_active,
        }
    }
}

pub fn strip_brackets(s: &str) -> &str {
    s.strip_prefix('⟨')
        .and_then(|s| s.strip_suffix('⟩'))
        .unwrap_or(s)
}

pub fn strip_dt(s: &str) -> &str {
    s.strip_prefix("d'")
        .and_then(|s| s.strip_suffix('\''))
        .unwrap_or(s)
}

impl TryFrom<DbAgent> for crate::db::schema::Agent {
    type Error = crate::id::IdError;

    fn try_from(db_agent: DbAgent) -> Result<Self, Self::Error> {
        // Extract the UUID from the record ID's key
        let agent_id = AgentId::from_uuid(
            Uuid::from_str(strip_brackets(&db_agent.id.key().to_string())).unwrap(),
        );

        let user_id = UserId::from_uuid(
            Uuid::from_str(strip_brackets(&db_agent.user_id.key().to_string())).unwrap(),
        );

        Ok(Self {
            id: agent_id,
            user_id,
            agent_type: db_agent.agent_type,
            name: db_agent.name,
            system_prompt: db_agent.system_prompt,
            config: db_agent.config,
            state: db_agent.state,
            created_at: from_surreal_datetime(db_agent.created_at),
            updated_at: from_surreal_datetime(db_agent.updated_at),
            is_active: db_agent.is_active,
        })
    }
}

impl From<crate::db::schema::MemoryBlock> for DbMemoryBlock {
    fn from(memory: crate::db::schema::MemoryBlock) -> Self {
        Self {
            id: RecordId::from(memory.id),
            agent_id: memory.agent_id.into(),
            label: memory.label,
            content: memory.content,
            description: memory.description,
            embedding: Some(memory.embedding),
            embedding_model: Some(memory.embedding_model),
            metadata: memory.metadata,
            created_at: memory.created_at.into(),
            updated_at: memory.updated_at.into(),
        }
    }
}

impl TryFrom<DbMemoryBlock> for crate::db::schema::MemoryBlock {
    type Error = crate::id::IdError;

    fn try_from(db_memory: DbMemoryBlock) -> Result<Self, Self::Error> {
        // Extract UUIDs from record IDs
        let memory_id = MemoryId::from_uuid(
            Uuid::from_str(strip_brackets(&db_memory.id.key().to_string())).unwrap(),
        );

        let agent_id = AgentId::from_uuid(
            Uuid::from_str(strip_brackets(&db_memory.agent_id.key().to_string())).unwrap(),
        );

        Ok(Self {
            id: memory_id,
            agent_id,
            label: db_memory.label,
            content: db_memory.content,
            description: db_memory.description,
            embedding: db_memory.embedding.unwrap_or_default(),
            embedding_model: db_memory.embedding_model.unwrap_or_default(),
            metadata: db_memory.metadata,
            is_active: true,
            created_at: from_surreal_datetime(db_memory.created_at),
            updated_at: from_surreal_datetime(db_memory.updated_at),
        })
    }
}
