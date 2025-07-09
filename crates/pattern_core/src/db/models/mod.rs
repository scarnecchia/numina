//! Database model wrappers
//!
//! These types mirror the domain types but use RecordId for database operations.
//! This allows us to maintain type safety in the domain while working with
//! SurrealDB's record ID format.

use chrono::{DateTime, Utc};
use compact_str::ToCompactString;
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, fmt, str::FromStr};
use surrealdb::RecordId;
use uuid::Uuid;

use crate::{
    agent::{AgentState, AgentType, MemoryAccessLevel},
    id::{AgentId, MemoryId, UserId},
    memory::MemoryBlock,
    message::Message,
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

/// Database representation of an Agent with full context
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DbAgent {
    pub id: RecordId,
    pub user_id: RecordId,
    pub agent_type: AgentType,
    pub name: String,
    pub system_prompt: String,
    pub state: AgentState,

    // Memory storage
    #[serde(default)]
    pub memory_blocks: Vec<DbMemoryBlock>,

    // Message history
    #[serde(default)]
    pub messages: Vec<Message>,
    #[serde(default)]
    pub archived_messages: Vec<Message>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message_summary: Option<String>,

    // Context configuration (stored as JSON to avoid SurrealDB enum issues)
    pub context_config: serde_json::Value,
    pub compression_strategy: serde_json::Value,

    // Metadata
    pub created_at: surrealdb::Datetime,
    pub last_active: surrealdb::Datetime,
    #[serde(default)]
    pub total_messages: usize,
    #[serde(default)]
    pub total_tool_calls: usize,
    #[serde(default)]
    pub context_rebuilds: usize,
    #[serde(default)]
    pub compression_events: usize,

    // Legacy fields
    #[serde(default)]
    pub config: serde_json::Value,
    pub updated_at: surrealdb::Datetime,
    #[serde(default = "default_true")]
    pub is_active: bool,
}

/// Database representation of a MemoryBlock
#[derive(Clone, Serialize, Deserialize)]
pub struct DbMemoryBlock {
    pub id: RecordId,
    pub owner_id: RecordId,
    pub label: String,
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub embedding: Option<Vec<f32>>,
    pub embedding_model: Option<String>,
    #[serde(default)]
    pub agents: Vec<RecordId>,
    #[serde(default)]
    pub metadata: serde_json::Value,
    pub created_at: surrealdb::Datetime,
    pub updated_at: surrealdb::Datetime,
}

impl fmt::Debug for DbMemoryBlock {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut debug_struct = f.debug_struct("DbMemoryBlock");
        debug_struct
            .field("id", &self.id)
            .field("owner_id", &self.owner_id)
            .field("label", &self.label)
            .field("content", &self.content)
            .field("description", &self.description);

        // Format embedding nicely
        match &self.embedding {
            Some(arr) => {
                let formatted = crate::utils::debug::EmbeddingDebug(arr);
                debug_struct.field("embedding", &format!("{}", formatted));
            }
            None => {
                debug_struct.field("embedding", &"none");
            }
        }

        debug_struct
            .field("embedding_model", &self.embedding_model)
            .field("agents", &self.agents)
            .field("metadata", &self.metadata)
            .field("created_at", &self.created_at)
            .field("updated_at", &self.updated_at)
            .finish()
    }
}

fn default_true() -> bool {
    true
}

pub fn from_surreal_datetime(dt: surrealdb::Datetime) -> DateTime<Utc> {
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
            state: agent.state,
            memory_blocks: agent.memory_blocks.into_iter().map(Into::into).collect(),
            messages: agent.messages,
            archived_messages: agent.archived_messages,
            message_summary: agent.message_summary,
            context_config: serde_json::to_value(agent.context_config)
                .expect("ContextConfig should serialize"),
            compression_strategy: serde_json::to_value(agent.compression_strategy)
                .expect("CompressionStrategy should serialize"),
            created_at: agent.created_at.into(),
            last_active: agent.last_active.into(),
            total_messages: agent.total_messages,
            total_tool_calls: agent.total_tool_calls,
            context_rebuilds: agent.context_rebuilds,
            compression_events: agent.compression_events,
            config: agent.config,
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

        println!("context config: {:?}", db_agent.context_config);
        println!("compression_strategy: {:?}", db_agent.compression_strategy);

        let user_id = UserId::from_uuid(
            Uuid::from_str(strip_brackets(&db_agent.user_id.key().to_string())).unwrap(),
        );

        Ok(Self {
            id: agent_id,
            user_id,
            agent_type: db_agent.agent_type,
            name: db_agent.name,
            system_prompt: db_agent.system_prompt,
            state: db_agent.state,
            memory_blocks: db_agent
                .memory_blocks
                .into_iter()
                .map(|mb| MemoryBlock::try_from(mb).expect("should deserialize memories"))
                .collect(),
            messages: db_agent.messages,
            archived_messages: db_agent.archived_messages,
            message_summary: db_agent.message_summary,
            context_config: serde_json::from_value(db_agent.context_config).map_err(|e| {
                crate::id::IdError::InvalidFormat(format!("Invalid context_config: {}", e))
            })?,
            compression_strategy: serde_json::from_value(db_agent.compression_strategy).map_err(
                |e| {
                    crate::id::IdError::InvalidFormat(format!(
                        "Invalid compression_strategy: {}",
                        e
                    ))
                },
            )?,
            created_at: from_surreal_datetime(db_agent.created_at),
            last_active: from_surreal_datetime(db_agent.last_active),
            total_messages: db_agent.total_messages,
            total_tool_calls: db_agent.total_tool_calls,
            context_rebuilds: db_agent.context_rebuilds,
            compression_events: db_agent.compression_events,
            config: db_agent.config,
            updated_at: from_surreal_datetime(db_agent.updated_at),
            is_active: db_agent.is_active,
        })
    }
}

impl From<crate::memory::MemoryBlock> for DbMemoryBlock {
    fn from(memory: crate::memory::MemoryBlock) -> Self {
        Self {
            id: RecordId::from(memory.id),
            owner_id: memory.owner_id.into(),
            label: memory.label.to_string(),
            content: memory.content,
            description: memory.description,
            embedding: None, // Core MemoryBlock doesn't have embedding
            embedding_model: memory.embedding_model,
            agents: Vec::new(), // Will be populated from junction table
            metadata: memory.metadata,
            created_at: memory.created_at.into(),
            updated_at: memory.updated_at.into(),
        }
    }
}

impl TryFrom<DbMemoryBlock> for crate::memory::MemoryBlock {
    type Error = crate::id::IdError;

    fn try_from(db_memory: DbMemoryBlock) -> Result<Self, Self::Error> {
        // Extract UUIDs from record IDs
        let memory_id = MemoryId::from_uuid(
            Uuid::from_str(strip_brackets(&db_memory.id.key().to_string())).unwrap(),
        );

        let owner_id = UserId::from_uuid(
            Uuid::from_str(strip_brackets(&db_memory.owner_id.key().to_string())).unwrap(),
        );

        Ok(Self {
            id: memory_id,
            owner_id,
            label: db_memory.label.try_to_compact_string().unwrap(),
            content: db_memory.content,
            description: db_memory.description,
            embedding_model: db_memory.embedding_model,
            metadata: db_memory.metadata,
            is_active: true,
            created_at: from_surreal_datetime(db_memory.created_at),
            updated_at: from_surreal_datetime(db_memory.updated_at),
        })
    }
}

/// Database representation of an Agent-Memory relationship
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DbAgentMemory {
    pub id: RecordId,
    pub r#in: RecordId,
    pub out: RecordId,
    pub access_level: MemoryAccessLevel,
    pub created_at: surrealdb::Datetime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccessBlock {
    pub access_level: MemoryAccessLevel,
    pub memory_data: DbMemoryBlock,
}
