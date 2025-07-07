//! Database schema definitions for Pattern

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

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
            Self::conversations(),
            Self::messages(),
            Self::tool_calls(),
            Self::tasks(),
        ]
    }

    /// System metadata table
    pub fn system_metadata() -> TableDefinition {
        TableDefinition {
            name: "system_metadata".to_string(),
            schema: r#"
                DEFINE TABLE system_metadata SCHEMAFULL;
                DEFINE FIELD embedding_model ON system_metadata TYPE string;
                DEFINE FIELD embedding_dimensions ON system_metadata TYPE int;
                DEFINE FIELD schema_version ON system_metadata TYPE int;
                DEFINE FIELD created_at ON system_metadata TYPE datetime;
                DEFINE FIELD updated_at ON system_metadata TYPE datetime;
            "#
            .to_string(),
            indexes: vec![],
        }
    }

    /// Users table
    pub fn users() -> TableDefinition {
        TableDefinition {
            name: "users".to_string(),
            schema: r#"
                DEFINE TABLE users SCHEMAFULL;
                DEFINE FIELD id ON users TYPE string;
                DEFINE FIELD created_at ON users TYPE datetime;
                DEFINE FIELD updated_at ON users TYPE datetime;
                DEFINE FIELD settings ON users TYPE object;
                DEFINE FIELD metadata ON users TYPE object;
            "#
            .to_string(),
            indexes: vec!["CREATE UNIQUE INDEX users_id ON users (id)".to_string()],
        }
    }

    /// Agents table
    pub fn agents() -> TableDefinition {
        TableDefinition {
            name: "agents".to_string(),
            schema: r#"
                DEFINE TABLE agents SCHEMAFULL;
                DEFINE FIELD id ON agents TYPE string;
                DEFINE FIELD user_id ON agents TYPE string;
                DEFINE FIELD agent_type ON agents TYPE string;
                DEFINE FIELD name ON agents TYPE string;
                DEFINE FIELD system_prompt ON agents TYPE string;
                DEFINE FIELD config ON agents TYPE object;
                DEFINE FIELD state ON agents TYPE object;
                DEFINE FIELD created_at ON agents TYPE datetime;
                DEFINE FIELD updated_at ON agents TYPE datetime;
                DEFINE FIELD is_active ON agents TYPE bool DEFAULT true;
            "#
            .to_string(),
            indexes: vec![
                "CREATE INDEX agents_user ON agents (user_id)".to_string(),
                "CREATE INDEX agents_type ON agents (agent_type)".to_string(),
                "CREATE UNIQUE INDEX agents_user_type ON agents (user_id, agent_type)".to_string(),
            ],
        }
    }

    /// Memory blocks table with embeddings
    pub fn memory_blocks() -> TableDefinition {
        TableDefinition {
            name: "memory_blocks".to_string(),
            schema: r#"
                DEFINE TABLE memory_blocks SCHEMAFULL;
                DEFINE FIELD id ON memory_blocks TYPE string;
                DEFINE FIELD agent_id ON memory_blocks TYPE string;
                DEFINE FIELD label ON memory_blocks TYPE string;
                DEFINE FIELD content ON memory_blocks TYPE string;
                DEFINE FIELD description ON memory_blocks TYPE option<string>;
                DEFINE FIELD embedding ON memory_blocks TYPE array<float>;
                DEFINE FIELD embedding_model ON memory_blocks TYPE string;
                DEFINE FIELD metadata ON memory_blocks TYPE object;
                DEFINE FIELD created_at ON memory_blocks TYPE datetime;
                DEFINE FIELD updated_at ON memory_blocks TYPE datetime;
                DEFINE FIELD is_active ON memory_blocks TYPE bool DEFAULT true;
            "#
            .to_string(),
            indexes: vec![
                "CREATE INDEX memory_agent ON memory_blocks (agent_id)".to_string(),
                "CREATE INDEX memory_label ON memory_blocks (agent_id, label)".to_string(),
            ],
        }
    }

    /// Conversations table
    pub fn conversations() -> TableDefinition {
        TableDefinition {
            name: "conversations".to_string(),
            schema: r#"
                DEFINE TABLE conversations SCHEMAFULL;
                DEFINE FIELD id ON conversations TYPE string;
                DEFINE FIELD user_id ON conversations TYPE string;
                DEFINE FIELD agent_id ON conversations TYPE string;
                DEFINE FIELD title ON conversations TYPE option<string>;
                DEFINE FIELD context ON conversations TYPE object;
                DEFINE FIELD created_at ON conversations TYPE datetime;
                DEFINE FIELD updated_at ON conversations TYPE datetime;
                DEFINE FIELD ended_at ON conversations TYPE option<datetime>;
            "#
            .to_string(),
            indexes: vec![
                "CREATE INDEX conv_user ON conversations (user_id)".to_string(),
                "CREATE INDEX conv_agent ON conversations (agent_id)".to_string(),
                "CREATE INDEX conv_active ON conversations (user_id, ended_at)".to_string(),
            ],
        }
    }

    /// Messages table with embeddings
    pub fn messages() -> TableDefinition {
        TableDefinition {
            name: "messages".to_string(),
            schema: r#"
                DEFINE TABLE messages SCHEMAFULL;
                DEFINE FIELD id ON messages TYPE string;
                DEFINE FIELD conversation_id ON messages TYPE string;
                DEFINE FIELD role ON messages TYPE string;
                DEFINE FIELD content ON messages TYPE string;
                DEFINE FIELD embedding ON messages TYPE option<array<float>>;
                DEFINE FIELD embedding_model ON messages TYPE option<string>;
                DEFINE FIELD metadata ON messages TYPE object;
                DEFINE FIELD tool_calls ON messages TYPE option<array>;
                DEFINE FIELD created_at ON messages TYPE datetime;
            "#
            .to_string(),
            indexes: vec![
                "CREATE INDEX msg_conversation ON messages (conversation_id)".to_string(),
                "CREATE INDEX msg_created ON messages (conversation_id, created_at)".to_string(),
            ],
        }
    }

    /// Tool calls audit table
    pub fn tool_calls() -> TableDefinition {
        TableDefinition {
            name: "tool_calls".to_string(),
            schema: r#"
                DEFINE TABLE tool_calls SCHEMAFULL;
                DEFINE FIELD id ON tool_calls TYPE string;
                DEFINE FIELD agent_id ON tool_calls TYPE string;
                DEFINE FIELD conversation_id ON tool_calls TYPE string;
                DEFINE FIELD tool_name ON tool_calls TYPE string;
                DEFINE FIELD parameters ON tool_calls TYPE object;
                DEFINE FIELD result ON tool_calls TYPE object;
                DEFINE FIELD error ON tool_calls TYPE option<string>;
                DEFINE FIELD duration_ms ON tool_calls TYPE int;
                DEFINE FIELD created_at ON tool_calls TYPE datetime;
            "#
            .to_string(),
            indexes: vec![
                "CREATE INDEX tools_agent ON tool_calls (agent_id)".to_string(),
                "CREATE INDEX tools_name ON tool_calls (tool_name)".to_string(),
                "CREATE INDEX tools_conversation ON tool_calls (conversation_id)".to_string(),
            ],
        }
    }

    /// Tasks table for ADHD support
    pub fn tasks() -> TableDefinition {
        TableDefinition {
            name: "tasks".to_string(),
            schema: r#"
                DEFINE TABLE tasks SCHEMAFULL;
                DEFINE FIELD id ON tasks TYPE string;
                DEFINE FIELD user_id ON tasks TYPE string;
                DEFINE FIELD parent_id ON tasks TYPE option<string>;
                DEFINE FIELD title ON tasks TYPE string;
                DEFINE FIELD description ON tasks TYPE option<string>;
                DEFINE FIELD embedding ON tasks TYPE option<array<float>>;
                DEFINE FIELD embedding_model ON tasks TYPE option<string>;
                DEFINE FIELD status ON tasks TYPE string DEFAULT 'pending';
                DEFINE FIELD priority ON tasks TYPE string DEFAULT 'medium';
                DEFINE FIELD estimated_minutes ON tasks TYPE option<int>;
                DEFINE FIELD actual_minutes ON tasks TYPE option<int>;
                DEFINE FIELD complexity_score ON tasks TYPE option<float>;
                DEFINE FIELD energy_required ON tasks TYPE string DEFAULT 'medium';
                DEFINE FIELD tags ON tasks TYPE array<string> DEFAULT [];
                DEFINE FIELD metadata ON tasks TYPE object;
                DEFINE FIELD created_at ON tasks TYPE datetime;
                DEFINE FIELD updated_at ON tasks TYPE datetime;
                DEFINE FIELD started_at ON tasks TYPE option<datetime>;
                DEFINE FIELD completed_at ON tasks TYPE option<datetime>;
                DEFINE FIELD due_at ON tasks TYPE option<datetime>;
            "#
            .to_string(),
            indexes: vec![
                "CREATE INDEX tasks_user ON tasks (user_id)".to_string(),
                "CREATE INDEX tasks_parent ON tasks (parent_id)".to_string(),
                "CREATE INDEX tasks_status ON tasks (user_id, status)".to_string(),
                "CREATE INDEX tasks_priority ON tasks (user_id, priority, status)".to_string(),
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
    pub id: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
    #[serde(default)]
    pub settings: HashMap<String, serde_json::Value>,
    #[serde(default)]
    pub metadata: HashMap<String, serde_json::Value>,
}

/// Agent model
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Agent {
    pub id: String,
    pub user_id: String,
    pub agent_type: String,
    pub name: String,
    pub system_prompt: String,
    #[serde(default)]
    pub config: serde_json::Value,
    #[serde(default)]
    pub state: serde_json::Value,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
    #[serde(default = "default_true")]
    pub is_active: bool,
}

/// Memory block model
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryBlock {
    pub id: String,
    pub agent_id: String,
    pub label: String,
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub embedding: Vec<f32>,
    pub embedding_model: String,
    #[serde(default)]
    pub metadata: serde_json::Value,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
    #[serde(default = "default_true")]
    pub is_active: bool,
}

/// Conversation model
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Conversation {
    pub id: String,
    pub user_id: String,
    pub agent_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default)]
    pub context: serde_json::Value,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ended_at: Option<chrono::DateTime<chrono::Utc>>,
}

/// Message model
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub id: String,
    pub conversation_id: String,
    pub role: String,
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub embedding: Option<Vec<f32>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub embedding_model: Option<String>,
    #[serde(default)]
    pub metadata: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<serde_json::Value>>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

/// Tool call model
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub agent_id: String,
    pub conversation_id: String,
    pub tool_name: String,
    pub parameters: serde_json::Value,
    pub result: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub duration_ms: i64,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

/// Task model
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    pub id: String,
    pub user_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<String>,
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub embedding: Option<Vec<f32>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub embedding_model: Option<String>,
    #[serde(default = "default_pending")]
    pub status: String,
    #[serde(default = "default_medium")]
    pub priority: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub estimated_minutes: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actual_minutes: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub complexity_score: Option<f32>,
    #[serde(default = "default_medium")]
    pub energy_required: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub metadata: serde_json::Value,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub started_at: Option<chrono::DateTime<chrono::Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<chrono::DateTime<chrono::Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub due_at: Option<chrono::DateTime<chrono::Utc>>,
}

fn default_true() -> bool {
    true
}

fn default_pending() -> String {
    "pending".to_string()
}

fn default_medium() -> String {
    "medium".to_string()
}
