//! Example entity implementations showing all field attribute types
//!
//! This module demonstrates how to use the Entity derive macro with various
//! field attributes for different storage strategies.

use crate::id::{AgentId, MemoryId, TaskId, TaskIdType, UserId};
use chrono::{DateTime, Utc};
use pattern_macros::Entity;
use serde::{Deserialize, Serialize};

// ============================================================================
// Example: Advanced Agent with Relations
// ============================================================================

/// An advanced agent that demonstrates all field types
#[derive(Debug, Clone, Entity, Serialize, Deserialize)]
#[entity(entity_type = "agent")]
pub struct AdvancedAgent {
    // Regular fields with automatic type conversion
    pub id: AgentId,
    pub name: String,
    pub model_id: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,

    // Skip field - computed at runtime, not stored
    #[entity(skip)]
    pub is_active: bool,

    // Skip field - derived from other data
    #[entity(skip)]
    pub age_days: i64,

    // Relation fields - stored in separate tables with RELATE
    #[entity(relation = "owns")]
    pub owner: UserId,

    #[entity(relation = "assigned")]
    pub assigned_tasks: Vec<TaskId>,

    #[entity(relation = "remembers")]
    pub memory_blocks: Vec<MemoryId>,
}

impl Default for AdvancedAgent {
    fn default() -> Self {
        let now = Utc::now();
        Self {
            id: AgentId::generate(),
            name: String::new(),
            model_id: None,
            created_at: now,
            updated_at: now,
            is_active: true,
            age_days: 0,
            owner: UserId::nil(),
            assigned_tasks: Vec::new(),
            memory_blocks: Vec::new(),
        }
    }
}

impl AdvancedAgent {
    /// Compute skip fields from stored data
    #[allow(dead_code)]
    pub fn compute_skip_fields(&mut self) {
        // Compute age in days
        self.age_days = (Utc::now() - self.created_at).num_days();

        // Determine if active (updated within last 7 days)
        self.is_active = (Utc::now() - self.updated_at).num_days() < 7;
    }
}

// ============================================================================
// Example: Custom Storage Types
// ============================================================================

/// Example showing custom storage type mapping
#[derive(Debug, Clone, Entity, Serialize, Deserialize)]
#[entity(entity_type = "task")]
pub struct CustomStorageTask {
    pub id: TaskId,
    pub title: String,

    // Store as flexible JSON in database
    pub metadata: serde_json::Value,

    // Store as comma-separated string
    #[entity(db_type = "String")]
    pub tags: Vec<String>,

    pub created_at: DateTime<Utc>,
}

// Custom conversion would be needed for the db_type fields
impl CustomStorageTask {
    #[allow(dead_code)]
    pub fn tags_to_string(&self) -> String {
        self.tags.join(",")
    }

    #[allow(dead_code)]
    pub fn tags_from_string(s: &str) -> Vec<String> {
        s.split(',').map(|s| s.trim().to_string()).collect()
    }
}

// ============================================================================
// Usage Examples
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_advanced_agent_relations() {
        let mut agent = AdvancedAgent {
            name: "Helper".to_string(),
            ..Default::default()
        };

        // Skip fields are computed, not stored
        agent.compute_skip_fields();
        assert_eq!(agent.age_days, 0);
        assert!(agent.is_active);

        // Relations would be loaded separately
        // agent.load_relations(&db).await?;
    }

    #[test]
    fn test_custom_storage_conversions() {
        let task = CustomStorageTask {
            id: TaskId::generate(),
            title: "Test Task".to_string(),
            metadata: serde_json::json!({
                "priority": "high",
                "category": "development"
            }),
            tags: vec!["rust".to_string(), "async".to_string()],
            created_at: Utc::now(),
        };

        // Tags would be converted to/from string for storage
        assert_eq!(task.tags_to_string(), "rust,async");
    }
}
