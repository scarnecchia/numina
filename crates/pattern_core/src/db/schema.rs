//! Database schema definitions for Pattern

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::{
    agent::{AgentState, AgentType},
    context::{CompressionStrategy, ContextConfig},
    id::{
        AgentId, AgentIdType, ConversationId, IdType, MemoryIdType, MessageId as MessageIdType,
        TaskId, TaskIdType, ToolCallId, ToolCallIdType, UserId, UserIdType,
    },
    memory::MemoryBlock,
    message::Message,
};

/// SQL schema definitions for the database
pub struct Schema;

impl Schema {
    /// Get all table definitions
    pub fn tables() -> Vec<TableDefinition> {
        vec![
            Self::system_metadata(),
            Self::users(),
            Self::agents(),
            Self::memory_blocks(),
            Self::messages(),
            Self::agent_memories(),
            Self::tool_calls(),
            Self::tasks(),
        ]
    }

    /// System metadata table
    pub fn system_metadata() -> TableDefinition {
        TableDefinition {
            name: "system_metadata".to_string(),
            schema: "
                DEFINE TABLE system_metadata SCHEMAFULL;
                DEFINE FIELD embedding_model ON system_metadata TYPE string;
                DEFINE FIELD embedding_dimensions ON system_metadata TYPE int;
                DEFINE FIELD schema_version ON system_metadata TYPE int;
                DEFINE FIELD created_at ON system_metadata TYPE datetime;
                DEFINE FIELD updated_at ON system_metadata TYPE datetime;
            "
            .to_string(),
            indexes: vec![],
        }
    }

    /// Users table
    pub fn users() -> TableDefinition {
        let table_name = UserIdType::PREFIX;
        TableDefinition {
            name: table_name.to_string(),
            schema: format!(
                "
                DEFINE TABLE {table} SCHEMAFULL;
                DEFINE FIELD id ON {table} TYPE record;
                DEFINE FIELD created_at ON {table} TYPE datetime PERMISSIONS FOR select, create FULL, FOR update NONE;
                DEFINE FIELD updated_at ON {table} TYPE datetime;
                DEFINE FIELD settings ON {table} FLEXIBLE TYPE object;
                DEFINE FIELD metadata ON {table} FLEXIBLE TYPE object;
            ",
                table = table_name
            ),
            indexes: vec![format!(
                "DEFINE INDEX {}_id ON {} FIELDS id UNIQUE",
                table_name, table_name
            )],
        }
    }

    /// Agents table with full context storage
    pub fn agents() -> TableDefinition {
        let table_name = AgentIdType::PREFIX;
        TableDefinition {
            name: table_name.to_string(),
            schema: format!(
                "
                DEFINE TABLE {table} SCHEMAFULL;
                DEFINE FIELD id ON {table} TYPE record;
                DEFINE FIELD user_id ON {table} TYPE record<user>;
                DEFINE FIELD agent_type ON {table} TYPE string;
                DEFINE FIELD name ON {table} TYPE string;
                DEFINE FIELD system_prompt ON {table} TYPE string;
                DEFINE FIELD state ON {table} TYPE any;

                DEFINE FIELD memory_blocks ON {table} TYPE array<record<mem>> DEFAULT [];
                DEFINE FIELD messages ON {table} TYPE array<record<msg>> DEFAULT [];
                DEFINE FIELD archived_messages ON {table} TYPE array<record<msg>> DEFAULT [];
                DEFINE FIELD message_summary ON {table} TYPE option<string>;

                DEFINE FIELD context_config ON {table} FLEXIBLE TYPE object;
                DEFINE FIELD compression_strategy ON {table} FLEXIBLE TYPE object;

                -- Metadata
                DEFINE FIELD created_at ON {table} TYPE datetime PERMISSIONS FOR select, create FULL, FOR update NONE;
                DEFINE FIELD last_active ON {table} TYPE datetime;
                DEFINE FIELD total_messages ON {table} TYPE int DEFAULT 0;
                DEFINE FIELD total_tool_calls ON {table} TYPE int DEFAULT 0;
                DEFINE FIELD context_rebuilds ON {table} TYPE int DEFAULT 0;
                DEFINE FIELD compression_events ON {table} TYPE int DEFAULT 0;

                -- Legacy fields for compatibility
                DEFINE FIELD config ON {table} FLEXIBLE TYPE object DEFAULT {{}};
                DEFINE FIELD updated_at ON {table} TYPE datetime;
                DEFINE FIELD is_active ON {table} TYPE bool DEFAULT true;
            ",
                table = table_name
            ),
            indexes: vec![
                format!(
                    "DEFINE INDEX {}_user ON {} FIELDS user_id",
                    table_name, table_name
                ),
                format!(
                    "DEFINE INDEX {}_type ON {} FIELDS agent_type",
                    table_name, table_name
                ),
                format!(
                    "DEFINE INDEX {}_user_type ON {} FIELDS user_id, agent_type UNIQUE",
                    table_name, table_name
                ),
                format!(
                    "DEFINE INDEX {}_last_active ON {} FIELDS last_active",
                    table_name, table_name
                ),
            ],
        }
    }

    /// Memory blocks table with embeddings
    pub fn memory_blocks() -> TableDefinition {
        let table_name = MemoryIdType::PREFIX;
        TableDefinition {
            name: table_name.to_string(),
            schema: format!(
                "
                DEFINE TABLE {table} SCHEMAFULL;
                DEFINE FIELD id ON {table} TYPE record;
                DEFINE FIELD owner_id ON {table} TYPE record<user>;
                DEFINE FIELD label ON {table} TYPE string;
                DEFINE FIELD content ON {table} TYPE string;
                DEFINE FIELD description ON {table} TYPE option<string>;
                DEFINE FIELD embedding ON {table} TYPE array<float>;
                DEFINE FIELD embedding_model ON {table} TYPE string;
                DEFINE FIELD agents ON {table} TYPE array<record<agent>> DEFAULT [];
                DEFINE FIELD metadata ON {table} FLEXIBLE TYPE object;
                DEFINE FIELD created_at ON {table} TYPE datetime PERMISSIONS FOR select, create FULL, FOR update NONE;
                DEFINE FIELD updated_at ON {table} TYPE datetime;
                DEFINE FIELD is_active ON {table} TYPE bool DEFAULT true;
            ",
                table = table_name
            ),
            indexes: vec![
                format!(
                    "DEFINE INDEX {}_owner ON {} FIELDS owner_id",
                    table_name, table_name
                ),
                format!(
                    "DEFINE INDEX {}_label ON {} FIELDS label",
                    table_name, table_name
                ),
                format!(
                    "DEFINE INDEX {}_agents ON {} FIELDS agents",
                    table_name, table_name
                ),
            ],
        }
    }

    /// Agent-Memory relationship table (many-to-many)
    pub fn agent_memories() -> TableDefinition {
        TableDefinition {
            name: "agent_memories".to_string(),
            schema: r#"
                DEFINE TABLE agent_memories SCHEMAFULL TYPE RELATION IN agent OUT mem ENFORCED;
                DEFINE FIELD access_level ON agent_memories TYPE string DEFAULT read ASSERT $value INSIDE ['read', 'write', 'admin'] PERMISSIONS FULL;
                DEFINE FIELD created_at ON agent_memories TYPE datetime PERMISSIONS FOR select, create FULL, FOR update NONE;
                DEFINE FIELD in ON agent_memories TYPE record<agent> PERMISSIONS FULL;
                DEFINE FIELD out ON agent_memories TYPE record<mem> PERMISSIONS FULL;

            "#.to_string(),
            indexes: vec![
                "
                DEFINE INDEX unique_relationships
                    ON TABLE agent_memories
                    COLUMNS in, out UNIQUE;
                ".to_string()
            ],
        }
    }

    /// Messages table with embeddings
    pub fn messages() -> TableDefinition {
        let table_name = MessageIdType::PREFIX;
        TableDefinition {
            name: table_name.to_string(),
            schema: format!(
                "
                DEFINE TABLE {table} SCHEMAFULL;
                DEFINE FIELD id ON {table} TYPE record;
                DEFINE FIELD agent_id ON {table} TYPE record<agent>;
                DEFINE FIELD role ON {table} TYPE string;
                DEFINE FIELD content ON {table} TYPE string;
                DEFINE FIELD embedding ON {table} TYPE option<array<float>>;
                DEFINE FIELD embedding_model ON {table} TYPE option<string>;
                DEFINE FIELD metadata ON {table} FLEXIBLE TYPE object;
                DEFINE FIELD tool_calls ON {table} TYPE option<array>;
                DEFINE FIELD created_at ON {table} TYPE datetime PERMISSIONS FOR select, create FULL, FOR update NONE;
            ",
                table = table_name
            ),
            indexes: vec![
                format!(
                    "DEFINE INDEX {}_conversation ON {} FIELDS conversation_id",
                    table_name, table_name
                ),
                format!(
                    "DEFINE INDEX {}_created ON {} FIELDS conversation_id, created_at",
                    table_name, table_name
                ),
            ],
        }
    }

    /// Tool calls audit table
    pub fn tool_calls() -> TableDefinition {
        let table_name = ToolCallIdType::PREFIX;
        TableDefinition {
            name: table_name.to_string(),
            schema: format!(
                "
                DEFINE TABLE {table} SCHEMAFULL;
                DEFINE FIELD id ON {table} TYPE record;
                DEFINE FIELD agent_id ON {table} TYPE record<agent>;
                DEFINE FIELD tool_name ON {table} TYPE string;
                DEFINE FIELD parameters ON {table} TYPE object;
                DEFINE FIELD result ON {table} TYPE object;
                DEFINE FIELD error ON {table} TYPE option<string>;
                DEFINE FIELD duration_ms ON {table} TYPE int;
                DEFINE FIELD created_at ON {table} TYPE datetime PERMISSIONS FOR select, create FULL, FOR update NONE;
            ",
                table = table_name
            ),
            indexes: vec![
                format!("DEFINE INDEX tools_agent ON {} FIELDS agent_id", table_name),
                format!("DEFINE INDEX tools_name ON {} FIELDS tool_name", table_name),
                format!(
                    "DEFINE INDEX tools_conversation ON {} FIELDS conversation_id",
                    table_name
                ),
            ],
        }
    }

    /// Tasks table for ADHD support
    pub fn tasks() -> TableDefinition {
        let table_name = TaskIdType::PREFIX;
        TableDefinition {
            name: table_name.to_string(),
            schema: format!(
                "
                DEFINE TABLE {table} SCHEMAFULL;
                DEFINE FIELD id ON {table} TYPE record;
                DEFINE FIELD user_id ON {table} TYPE record<user>;
                DEFINE FIELD parent_id ON {table} TYPE option<record>;
                DEFINE FIELD title ON {table} TYPE string;
                DEFINE FIELD description ON {table} TYPE option<string>;
                DEFINE FIELD embedding ON {table} TYPE option<array<float>>;
                DEFINE FIELD embedding_model ON {table} TYPE option<string>;
                DEFINE FIELD status ON {table} TYPE string DEFAULT 'pending';
                DEFINE FIELD priority ON {table} TYPE string DEFAULT 'medium';
                DEFINE FIELD estimated_minutes ON {table} TYPE option<int>;
                DEFINE FIELD actual_minutes ON {table} TYPE option<int>;
                DEFINE FIELD complexity_score ON {table} TYPE option<float>;
                DEFINE FIELD energy_required ON {table} TYPE string DEFAULT 'medium';
                DEFINE FIELD tags ON {table} TYPE array<string> DEFAULT [];
                DEFINE FIELD metadata ON {table} TYPE object;
                DEFINE FIELD created_at ON {table} TYPE datetime PERMISSIONS FOR select, create FULL, FOR update NONE;
                DEFINE FIELD updated_at ON {table} TYPE datetime;
                DEFINE FIELD started_at ON {table} TYPE option<datetime>;
                DEFINE FIELD completed_at ON {table} TYPE option<datetime>;
                DEFINE FIELD due_at ON {table} TYPE option<datetime>;
            ",
                table = table_name
            ),
            indexes: vec![
                format!("DEFINE INDEX tasks_user ON {} FIELDS user_id", table_name),
                format!(
                    "DEFINE INDEX tasks_parent ON {} FIELDS parent_id",
                    table_name
                ),
                format!(
                    "DEFINE INDEX tasks_status ON {} FIELDS user_id, status",
                    table_name
                ),
                format!(
                    "DEFINE INDEX tasks_priority ON {} FIELDS user_id, priority, status",
                    table_name
                ),
            ],
        }
    }

    /// Create vector index query for a table
    pub fn vector_index(table: &str, field: &str, dimensions: usize) -> String {
        format!(
            "DEFINE INDEX {}_vector_idx ON {} FIELDS {} HNSW DIMENSION {} DIST COSINE",
            table, table, field, dimensions
        )
    }
}

/// Table definition with schema and indexes
#[derive(Debug, Clone)]
pub struct TableDefinition {
    pub name: String,
    pub schema: String,
    pub indexes: Vec<String>,
}

/// Database models

/// User model
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct User {
    pub id: UserId,

    pub created_at: chrono::DateTime<chrono::Utc>,

    pub updated_at: chrono::DateTime<chrono::Utc>,
    #[serde(default)]
    pub settings: HashMap<String, serde_json::Value>,
    #[serde(default)]
    pub metadata: HashMap<String, serde_json::Value>,
}

/// Agent model with full context
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Agent {
    pub id: AgentId,
    pub user_id: UserId,
    pub agent_type: AgentType,
    pub name: String,
    pub system_prompt: String,
    pub state: AgentState,

    // Memory storage
    #[serde(default)]
    pub memory_blocks: Vec<MemoryBlock>,

    // Message history
    #[serde(default)]
    pub messages: Vec<Message>,
    #[serde(default)]
    pub archived_messages: Vec<Message>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message_summary: Option<String>,

    // Context configuration
    pub context_config: ContextConfig,
    pub compression_strategy: CompressionStrategy,

    // Metadata
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub last_active: chrono::DateTime<chrono::Utc>,
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
    pub updated_at: chrono::DateTime<chrono::Utc>,
    pub is_active: bool,
}

/// Tool call model
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: ToolCallId,

    pub agent_id: AgentId,
    pub tool_name: String,
    pub parameters: serde_json::Value,
    pub result: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub duration_ms: i64,

    pub created_at: chrono::DateTime<chrono::Utc>,
}

/// Task status enum
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TaskStatus {
    Pending,
    InProgress,
    Completed,
    Cancelled,
    Blocked,
}

/// Task priority enum
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TaskPriority {
    Low,
    Medium,
    High,
    Critical,
}

/// Energy level required for a task
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EnergyLevel {
    Low,
    Medium,
    High,
}

/// Task model
#[derive(Clone, Serialize, Deserialize)]
pub struct Task {
    pub id: TaskId,

    pub user_id: UserId,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub parent_id: Option<TaskId>,
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub embedding: Option<Vec<f32>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub embedding_model: Option<String>,
    #[serde(default = "default_pending")]
    pub status: TaskStatus,
    #[serde(default = "default_medium_priority")]
    pub priority: TaskPriority,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub estimated_minutes: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actual_minutes: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub complexity_score: Option<f32>,
    #[serde(default = "default_medium_energy")]
    pub energy_required: EnergyLevel,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub metadata: serde_json::Value,

    pub created_at: chrono::DateTime<chrono::Utc>,

    pub updated_at: chrono::DateTime<chrono::Utc>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub started_at: Option<chrono::DateTime<chrono::Utc>>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub completed_at: Option<chrono::DateTime<chrono::Utc>>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub due_at: Option<chrono::DateTime<chrono::Utc>>,
}

impl std::fmt::Debug for Task {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut debug_struct = f.debug_struct("Task");
        debug_struct
            .field("id", &self.id)
            .field("user_id", &self.user_id)
            .field("parent_id", &self.parent_id)
            .field("title", &self.title)
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
            .field("status", &self.status)
            .field("priority", &self.priority)
            .field("estimated_minutes", &self.estimated_minutes)
            .field("actual_minutes", &self.actual_minutes)
            .field("complexity_score", &self.complexity_score)
            .field("energy_required", &self.energy_required)
            .field("tags", &self.tags)
            .field("metadata", &self.metadata)
            .field("created_at", &self.created_at)
            .field("updated_at", &self.updated_at)
            .field("started_at", &self.started_at)
            .field("completed_at", &self.completed_at)
            .field("due_at", &self.due_at)
            .finish()
    }
}

fn default_pending() -> TaskStatus {
    TaskStatus::Pending
}

fn default_medium_priority() -> TaskPriority {
    TaskPriority::Medium
}

fn default_medium_energy() -> EnergyLevel {
    EnergyLevel::Medium
}
