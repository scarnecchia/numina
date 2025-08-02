//! Agent entity definition for database persistence
//!
//! This module defines the Agent struct that represents the persistent state
//! of a DatabaseAgent. It includes all fields that need to be stored in the
//! database and can be used to reconstruct a DatabaseAgent instance.

use crate::agent::{AgentState, AgentType};
use crate::context::{CompressionStrategy, ContextConfig};
use crate::id::{AgentId, EventId, MemoryId, RelationId, TaskId, UserId};
use crate::memory::MemoryBlock;
use chrono::{DateTime, Utc};
use ferroid::{SnowflakeGeneratorAsyncTokioExt, SnowflakeMastodonId};
use pattern_macros::Entity;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::OnceLock;

/// Type alias for the Snowflake generator we're using
type SnowflakeGen = ferroid::AtomicSnowflakeGenerator<SnowflakeMastodonId, ferroid::MonotonicClock>;

/// Global ID generator for message positions using Snowflake IDs
/// This provides distributed, monotonic IDs that work across processes
static MESSAGE_POSITION_GENERATOR: OnceLock<SnowflakeGen> = OnceLock::new();

pub fn get_position_generator() -> &'static SnowflakeGen {
    MESSAGE_POSITION_GENERATOR.get_or_init(|| {
        // Use machine ID 0 for now - in production this would be configurable
        let clock = ferroid::MonotonicClock::with_epoch(ferroid::TWITTER_EPOCH);
        ferroid::AtomicSnowflakeGenerator::new(0, clock)
    })
}

/// Get the next message position as a Snowflake ID
pub async fn get_next_message_position() -> String {
    get_position_generator()
        .try_next_id_async()
        .await
        .expect("for now we are assuming this succeeds")
        .to_string()
}

/// Agent entity that persists to the database
///
/// This struct contains all the state needed to reconstruct a DatabaseAgent.
/// The runtime components (model provider, tools, etc.) are injected when
/// creating the DatabaseAgent from this persisted state.
#[derive(Debug, Clone, Entity, Serialize, Deserialize)]
#[entity(entity_type = "agent")]
pub struct AgentRecord {
    pub id: AgentId,
    pub name: String,
    pub agent_type: AgentType,
    pub state: AgentState,

    // Model configuration
    pub model_id: Option<String>,
    pub model_config: HashMap<String, serde_json::Value>,

    // Context configuration that gets persisted
    pub base_instructions: String,
    pub max_messages: usize,
    pub max_message_age_hours: i64,
    pub compression_threshold: usize,
    pub memory_char_limit: usize,
    pub enable_thinking: bool,
    #[entity(db_type = "object")]
    pub compression_strategy: CompressionStrategy,

    // Tool execution rules for this agent (serialized as JSON)
    #[serde(default)]
    pub tool_rules: Vec<crate::config::ToolRuleConfig>,

    // Runtime statistics
    pub total_messages: usize,
    pub total_tool_calls: usize,
    pub context_rebuilds: usize,
    pub compression_events: usize,

    // Timestamps
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub last_active: DateTime<Utc>,

    // Relations (using Entity macro features)
    #[entity(relation = "owns", reverse = true)]
    pub owner_id: UserId,

    #[entity(relation = "assigned")]
    pub assigned_task_ids: Vec<TaskId>,

    #[entity(edge_entity = "agent_memories")]
    pub memories: Vec<(MemoryBlock, AgentMemoryRelation)>,

    #[entity(edge_entity = "agent_messages")]
    pub messages: Vec<(
        crate::message::Message,
        crate::message::AgentMessageRelation,
    )>,

    // Optional summary of archived messages for context
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message_summary: Option<String>,

    // TODO: Add MessageThreadId type when message threads are implemented
    // For now, we'll skip conversation IDs
    // #[entity(relation = "participated")]
    // pub conversation_ids: Vec<MessageThreadId>,
    #[entity(relation = "scheduled")]
    pub scheduled_event_ids: Vec<EventId>,
}

impl Default for AgentRecord {
    fn default() -> Self {
        let now = Utc::now();
        Self {
            id: AgentId::generate(),
            name: String::new(),
            agent_type: AgentType::Generic,
            state: AgentState::Ready,
            model_id: None,
            model_config: HashMap::new(),
            base_instructions: String::new(),
            max_messages: 50,
            max_message_age_hours: 24,
            compression_threshold: 30,
            memory_char_limit: 5000,
            enable_thinking: true,
            compression_strategy: CompressionStrategy::Truncate { keep_recent: 20 },
            tool_rules: Vec::new(),
            total_messages: 0,
            total_tool_calls: 0,
            context_rebuilds: 0,
            compression_events: 0,
            created_at: now,
            updated_at: now,
            last_active: now,
            owner_id: UserId::nil(),
            assigned_task_ids: Vec::new(),
            memories: Vec::new(),
            messages: Vec::new(),
            message_summary: None,
            scheduled_event_ids: Vec::new(),
        }
    }
}

/// Extension methods for AgentRecord
impl AgentRecord {
    /// Create a ContextConfig from the agent's stored configuration
    pub fn to_context_config(&self) -> ContextConfig {
        // Create model adjustments based on compression settings
        let model_adjustments = crate::context::ModelAdjustments {
            // We'll keep defaults for now, but could be enhanced later
            ..Default::default()
        };

        ContextConfig {
            base_instructions: self.base_instructions.clone(),
            memory_char_limit: self.memory_char_limit,
            max_context_messages: self.max_messages,
            enable_thinking: self.enable_thinking,
            tool_usage_rules: vec![],
            tool_workflow_rules: vec![],
            model_adjustments,
        }
    }

    /// Update the agent from a DatabaseAgent's current state
    pub fn update_from_runtime(
        &mut self,
        total_messages: usize,
        total_tool_calls: usize,
        context_rebuilds: usize,
        compression_events: usize,
    ) {
        self.total_messages = total_messages;
        self.total_tool_calls = total_tool_calls;
        self.context_rebuilds = context_rebuilds;
        self.compression_events = compression_events;
        self.last_active = Utc::now();
        self.updated_at = Utc::now();
    }

    /// Load the agent's message history (active and/or archived)
    ///
    /// This loads messages via the agent_messages edge entity, respecting
    /// the message type (active, archived, shared) and ordering.
    pub async fn load_message_history<C: surrealdb::Connection>(
        &self,
        db: &surrealdb::Surreal<C>,
        include_archived: bool,
    ) -> Result<
        Vec<(
            crate::message::Message,
            crate::message::AgentMessageRelation,
        )>,
        crate::db::DatabaseError,
    > {
        let query = if include_archived {
            format!(
                r#"SELECT * FROM agent_messages
                   WHERE in = $agent_id
                   ORDER BY position ASC"#
            )
        } else {
            format!(
                r#"SELECT * FROM agent_messages
                   WHERE in = $agent_id AND message_type = "active"
                   ORDER BY position ASC"#
            )
        };

        tracing::debug!(
            "Loading message history for agent {}: query={}",
            self.id,
            query
        );

        let mut result = db
            .query(&query)
            .bind(("agent_id", surrealdb::RecordId::from(&self.id)))
            .await
            .map_err(crate::db::DatabaseError::QueryFailed)?;

        tracing::trace!("Message history query result: {:?}", result);

        // First get the DB models (which have the serde rename attributes)
        use crate::db::entity::DbEntity;
        let relation_db_models: Vec<<crate::message::AgentMessageRelation as DbEntity>::DbModel> =
            result
                .take(0)
                .map_err(crate::db::DatabaseError::QueryFailed)?;

        tracing::debug!(
            "Found {} agent_messages relations for agent {}",
            relation_db_models.len(),
            self.id
        );

        // Convert DB models to domain types
        let relations: Vec<crate::message::AgentMessageRelation> = relation_db_models
            .into_iter()
            .map(|db_model| {
                crate::message::AgentMessageRelation::from_db_model(db_model)
                    .map_err(crate::db::DatabaseError::from)
            })
            .collect::<Result<Vec<_>, _>>()?;

        // Now load the messages for each relation
        let mut messages_with_relations = Vec::new();

        for relation in relations {
            tracing::trace!(
                "Loading message for relation: message_id={:?}, type={:?}, position={}",
                relation.out_id,
                relation.message_type,
                relation.position
            );

            if let Some(message) =
                crate::message::Message::load_with_relations(db, &relation.out_id).await?
            {
                tracing::trace!(
                    "Loaded message: id={:?}, role={:?}, content_len={}",
                    message.id,
                    message.role,
                    message.text_content().map(|s| s.len()).unwrap_or(0)
                );
                messages_with_relations.push((message, relation));
            } else {
                tracing::warn!("Message {:?} not found in database", relation.out_id);
            }
        }

        tracing::debug!(
            "Total messages loaded for agent {}: {}",
            self.id,
            messages_with_relations.len()
        );

        Ok(messages_with_relations)
    }

    /// Attach a message to this agent with the specified relationship type
    ///
    /// This creates an agent_messages edge between the agent and message,
    /// tracking the relationship type and position in the history.
    pub async fn attach_message<C: surrealdb::Connection>(
        &self,
        db: &surrealdb::Surreal<C>,
        message_id: &crate::MessageId,
        message_type: crate::message::MessageRelationType,
    ) -> Result<(), crate::db::DatabaseError> {
        let position = get_position_generator()
            .try_next_id_async()
            .await
            .expect("for now we are assuming this succeeds");

        // Create the relation using the edge entity
        let relation = crate::message::AgentMessageRelation {
            id: RelationId::nil(),
            in_id: self.id.clone(),
            out_id: message_id.clone(),
            message_type,
            position: position.to_string(),
            added_at: chrono::Utc::now(),
        };

        // Use create_relation_typed to store the edge entity
        crate::db::ops::create_relation_typed(&db, &relation).await?;

        Ok(())
    }
}

/// Edge entity for agent-memory relationships
///
/// This is already defined in base.rs but included here for reference.
/// It stores metadata about how an agent relates to a memory block.
use crate::memory::MemoryPermission;

#[derive(Debug, Clone, Entity, Serialize, Deserialize)]
#[entity(entity_type = "agent_memories", edge = true)]
pub struct AgentMemoryRelation {
    pub id: RelationId,
    pub in_id: AgentId,
    pub out_id: MemoryId,
    pub access_level: MemoryPermission,
    pub created_at: DateTime<Utc>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::entity::DbEntity;

    #[test]
    fn test_agent_record_schema() {
        let schema = AgentRecord::schema();
        println!("AgentRecord schema:\n{}", schema.schema);

        // Also check the storage type to debug
        let storage_type = std::any::type_name::<CompressionStrategy>();
        println!("CompressionStrategy type name: {}", storage_type);
    }

    #[test]
    fn test_agent_record_creation() {
        let agent = AgentRecord {
            name: "Test Agent".to_string(),
            agent_type: AgentType::Custom("test".to_string()),
            owner_id: UserId::generate(),
            ..Default::default()
        };
        assert_eq!(agent.name, "Test Agent");
        assert!(matches!(agent.agent_type, AgentType::Custom(ref s) if s == "test"));
        assert_eq!(agent.state, AgentState::Ready);
    }

    #[tokio::test]
    async fn test_agent_message_relationships() {
        use crate::db::client;
        use crate::message::{Message, MessageRelationType};

        let db = client::create_test_db().await.unwrap();

        // Create an agent
        let agent = AgentRecord {
            name: "Test Agent".to_string(),
            agent_type: AgentType::Generic,
            owner_id: UserId::generate(),
            ..Default::default()
        };
        let stored_agent = agent.store_with_relations(&db).await.unwrap();

        // Create some messages
        let msg1 = Message::user("First message");
        let msg2 = Message::agent("Agent response");
        let msg3 = Message::user("Another message");

        let stored_msg1 = msg1.store_with_relations(&db).await.unwrap();
        let stored_msg2 = msg2.store_with_relations(&db).await.unwrap();
        let stored_msg3 = msg3.store_with_relations(&db).await.unwrap();

        // Attach messages to agent
        stored_agent
            .attach_message(&db, &stored_msg1.id, MessageRelationType::Active)
            .await
            .unwrap();
        stored_agent
            .attach_message(&db, &stored_msg2.id, MessageRelationType::Active)
            .await
            .unwrap();
        stored_agent
            .attach_message(&db, &stored_msg3.id, MessageRelationType::Archived)
            .await
            .unwrap();

        // Load active messages only
        let active_messages = stored_agent.load_message_history(&db, false).await.unwrap();
        assert_eq!(active_messages.len(), 2);
        assert_eq!(
            active_messages[0].0.text_content(),
            Some("First message".to_string())
        );
        assert_eq!(
            active_messages[1].0.text_content(),
            Some("Agent response".to_string())
        );

        // Verify positions are ordered correctly (timestamp-based)
        assert!(active_messages[0].1.position < active_messages[1].1.position);

        // Load all messages including archived
        let all_messages = stored_agent.load_message_history(&db, true).await.unwrap();
        assert_eq!(all_messages.len(), 3);
        assert_eq!(
            all_messages[2].0.text_content(),
            Some("Another message".to_string())
        );
        assert_eq!(
            all_messages[2].1.message_type,
            MessageRelationType::Archived
        );

        // Verify all positions are ordered
        assert!(all_messages[0].1.position < all_messages[1].1.position);
        assert!(all_messages[1].1.position < all_messages[2].1.position);
    }
}

/// Conversion implementations
impl AgentRecord {
    /// Create an AgentRecord from a DatabaseAgent's current state
    ///
    /// Note: This creates a snapshot of the current agent state.
    /// Some fields like relations, creation time, and detailed config are not
    /// accessible through the public API and would need to be loaded separately.

    pub async fn from_database_agent<C, M, E>(
        agent: &crate::agent::DatabaseAgent<C, M, E>,
        owner_id: UserId,
        base_instructions: String,
    ) -> Self
    where
        C: surrealdb::Connection + Clone + std::fmt::Debug + 'static,
        M: crate::ModelProvider + 'static,
        E: crate::embeddings::EmbeddingProvider + 'static,
    {
        use crate::agent::Agent;

        let now = Utc::now();

        Self {
            id: agent.id(),
            name: agent.name().to_string(),
            agent_type: agent.agent_type(),
            state: agent.state().await,
            model_id: None,               // TODO: Get from model provider
            model_config: HashMap::new(), // TODO: Get from model provider config
            base_instructions,
            max_messages: 50,          // TODO: Make configurable
            max_message_age_hours: 24, // TODO: Make configurable
            compression_threshold: 30, // TODO: Make configurable
            memory_char_limit: 5000,   // TODO: Make configurable
            enable_thinking: true,     // TODO: Make configurable
            compression_strategy: CompressionStrategy::Truncate { keep_recent: 20 }, // TODO: Make configurable
            tool_rules: Vec::new(), // TODO: Get from agent's tool rule engine
            // Stats would need to be exposed through the Agent trait or stored externally
            total_messages: 0,
            total_tool_calls: 0,
            context_rebuilds: 0,
            compression_events: 0,
            created_at: now, // Would need to be stored when agent is first created
            updated_at: now,
            last_active: now,
            owner_id,
            // Relations would need to be loaded from the database
            assigned_task_ids: Vec::new(),
            memories: Vec::new(),
            messages: Vec::new(),
            message_summary: None,
            scheduled_event_ids: Vec::new(),
        }
    }
}
