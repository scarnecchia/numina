//! Base entity implementations using the derive macro system
//!
//! These implementations provide the core entities that all deployments need,
//! using the derive macro for minimal boilerplate and proper type separation.

use crate::agent::AgentState;
use crate::id::{AgentId, EventId, MemoryId, TaskId, TaskIdType, UserId};
use chrono::{DateTime, Utc};
use pattern_macros::Entity;
use serde::{Deserialize, Serialize};

// ============================================================================
// Base User Implementation
// ============================================================================

/// Base user entity with no extensions
#[derive(Debug, Clone, Entity, Serialize, Deserialize)]
#[entity(entity_type = "user")]
pub struct BaseUser {
    pub id: UserId,
    pub discord_id: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,

    // Relations
    #[entity(relation = "owns")]
    pub owned_agent_ids: Vec<AgentId>,

    #[entity(relation = "created")]
    pub created_task_ids: Vec<TaskId>,

    #[entity(relation = "remembers")]
    pub memory_ids: Vec<MemoryId>,

    #[entity(relation = "scheduled")]
    pub scheduled_event_ids: Vec<EventId>,
}

impl Default for BaseUser {
    fn default() -> Self {
        let now = Utc::now();
        Self {
            id: UserId::generate(),
            discord_id: None,
            created_at: now,
            updated_at: now,
            owned_agent_ids: Vec::new(),
            created_task_ids: Vec::new(),
            memory_ids: Vec::new(),
            scheduled_event_ids: Vec::new(),
        }
    }
}

// ============================================================================
// Base Agent Implementation
// ============================================================================

/// Base agent type enum
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum BaseAgentType {
    /// Generic assistant agent
    Assistant,
    /// Custom agent with a string identifier
    Custom(String),
}

impl Serialize for BaseAgentType {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        match self {
            BaseAgentType::Assistant => serializer.serialize_str("assistant"),
            BaseAgentType::Custom(s) => serializer.serialize_str(s),
        }
    }
}

impl<'de> Deserialize<'de> for BaseAgentType {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        match s.as_str() {
            "assistant" => Ok(BaseAgentType::Assistant),
            custom => Ok(BaseAgentType::Custom(custom.to_string())),
        }
    }
}

/// Base agent entity
#[derive(Debug, Clone, Entity, Serialize, Deserialize)]
#[entity(entity_type = "agent")]
pub struct BaseAgent {
    pub id: AgentId,
    pub name: String,
    pub agent_type: BaseAgentType,
    pub state: AgentState,
    pub model_id: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,

    // Owner reference (not a relation to avoid circular dependency)
    pub owner_id: UserId,

    #[entity(relation = "assigned")]
    pub assigned_task_ids: Vec<TaskId>,

    #[entity(relation = "has_memory")]
    pub memory_ids: Vec<MemoryId>,
}

impl Default for BaseAgent {
    fn default() -> Self {
        let now = Utc::now();
        Self {
            id: AgentId::generate(),
            name: String::new(),
            agent_type: BaseAgentType::Assistant,
            state: AgentState::default(),
            model_id: None,
            created_at: now,
            updated_at: now,
            owner_id: UserId::nil(),
            assigned_task_ids: Vec::new(),
            memory_ids: Vec::new(),
        }
    }
}

// ============================================================================
// Base Task Implementation
// ============================================================================

/// Base task status
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BaseTaskStatus {
    Pending,
    InProgress,
    Completed,
    Cancelled,
}

/// Base task priority
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BaseTaskPriority {
    Low,
    Medium,
    High,
    Critical,
}

/// Base task entity
#[derive(Debug, Clone, Entity, Serialize, Deserialize)]
#[entity(entity_type = "task")]
pub struct BaseTask {
    pub id: TaskId,
    pub title: String,
    pub description: Option<String>,
    pub status: BaseTaskStatus,
    pub priority: BaseTaskPriority,
    pub due_date: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,

    // Foreign key references (not relations to avoid circular dependencies)
    pub creator_id: UserId,
    pub assigned_agent_id: Option<AgentId>,
    pub parent_task_id: Option<TaskId>,

    #[entity(relation = "has_subtask")]
    pub subtask_ids: Vec<TaskId>,
}

impl Default for BaseTask {
    fn default() -> Self {
        let now = Utc::now();
        Self {
            id: TaskId::generate(),
            title: String::new(),
            description: None,
            status: BaseTaskStatus::Pending,
            priority: BaseTaskPriority::Medium,
            due_date: None,
            completed_at: None,
            created_at: now,
            updated_at: now,
            creator_id: UserId::nil(),
            assigned_agent_id: None,
            parent_task_id: None,
            subtask_ids: Vec::new(),
        }
    }
}

// ============================================================================
// Base Event Implementation
// ============================================================================

/// Base event entity
#[derive(Debug, Clone, Entity, Serialize, Deserialize)]
#[entity(entity_type = "event")]
pub struct BaseEvent {
    pub id: EventId,
    pub title: String,
    pub description: Option<String>,
    pub event_type: String,
    pub scheduled_for: DateTime<Utc>,
    pub duration_minutes: Option<i32>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,

    // Creator reference (not a relation to avoid circular dependency)
    pub creator_id: UserId,
}

impl Default for BaseEvent {
    fn default() -> Self {
        let now = Utc::now();
        Self {
            id: EventId::generate(),
            title: String::new(),
            description: None,
            event_type: "general".to_string(),
            scheduled_for: now,
            duration_minutes: None,
            created_at: now,
            updated_at: now,
            creator_id: UserId::nil(),
        }
    }
}

// ============================================================================
// AgentMemoryRelation - Edge Entity for Agent-Memory Relationships
// ============================================================================

use crate::agent::MemoryAccessLevel;
use crate::id::{RelationId, RelationIdType};

/// Edge entity for agent-memory relationships with access levels
#[derive(Debug, Clone, Entity, Serialize, Deserialize)]
#[entity(entity_type = "agent_memories")]
pub struct AgentMemoryRelation {
    pub id: RelationId,
    pub in_id: AgentId,
    pub out_id: MemoryId,
    pub access_level: MemoryAccessLevel,
    pub created_at: DateTime<Utc>,
}

impl Default for AgentMemoryRelation {
    fn default() -> Self {
        Self {
            id: RelationId::generate(),
            in_id: AgentId::nil(),
            out_id: MemoryId::nil(),
            access_level: MemoryAccessLevel::Read,
            created_at: Utc::now(),
        }
    }
}

// ============================================================================
// RELATE Helper Functions (Deprecated - Use entity.store_with_relations() instead)
// ============================================================================

// Note: These functions are deprecated. The entity system now handles relations
// automatically through the store_with_relations() and load_with_relations() methods.
// They are kept here for backward compatibility but will be removed in a future version.

// ============================================================================
// Query Helper Functions (Updated to use entity system)
// ============================================================================

/// Get all agents owned by a user
pub async fn get_user_agents<C: surrealdb::Connection>(
    db: &surrealdb::Surreal<C>,
    user_id: &UserId,
) -> Result<Vec<AgentId>, crate::db::DatabaseError> {
    let user = BaseUser::load_with_relations(db, *user_id).await?;
    Ok(user.map(|u| u.owned_agent_ids).unwrap_or_default())
}

/// Get all tasks created by a user
pub async fn get_user_tasks<C: surrealdb::Connection>(
    db: &surrealdb::Surreal<C>,
    user_id: &UserId,
) -> Result<Vec<TaskId>, crate::db::DatabaseError> {
    let user = BaseUser::load_with_relations(db, *user_id).await?;
    Ok(user.map(|u| u.created_task_ids).unwrap_or_default())
}

/// Get all tasks assigned to an agent
pub async fn get_agent_tasks<C: surrealdb::Connection>(
    db: &surrealdb::Surreal<C>,
    agent_id: &AgentId,
) -> Result<Vec<TaskId>, crate::db::DatabaseError> {
    let agent = BaseAgent::load_with_relations(db, *agent_id).await?;
    Ok(agent.map(|a| a.assigned_task_ids).unwrap_or_default())
}

/// Get the owner of a task
/// Get the creator of a task
pub async fn get_task_owner<C: surrealdb::Connection>(
    db: &surrealdb::Surreal<C>,
    task_id: &TaskId,
) -> Result<Option<UserId>, crate::db::DatabaseError> {
    let task = BaseTask::load_with_relations(db, *task_id).await?;
    Ok(task.map(|t| t.creator_id))
}

/// Get all subtasks of a task
pub async fn get_task_subtasks<C: surrealdb::Connection>(
    db: &surrealdb::Surreal<C>,
    parent_id: &TaskId,
) -> Result<Vec<TaskId>, crate::db::DatabaseError> {
    let task = BaseTask::load_with_relations(db, *parent_id).await?;
    Ok(task.map(|t| t.subtask_ids).unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_base_user_creation() {
        let user = BaseUser::default();
        assert!(user.discord_id.is_none());
        assert!(user.created_at <= Utc::now());
    }

    #[test]
    fn test_base_agent_creation() {
        let agent = BaseAgent {
            name: "Test Agent".to_string(),
            agent_type: BaseAgentType::Custom("test".to_string()),
            ..Default::default()
        };
        assert_eq!(agent.name, "Test Agent");
        assert!(matches!(agent.agent_type, BaseAgentType::Custom(ref s) if s == "test"));
    }

    #[test]
    fn test_base_task_creation() {
        let task = BaseTask {
            title: "Test Task".to_string(),
            priority: BaseTaskPriority::High,
            ..Default::default()
        };
        assert_eq!(task.title, "Test Task");
        assert_eq!(task.priority, BaseTaskPriority::High);
        assert_eq!(task.status, BaseTaskStatus::Pending);
    }
}
