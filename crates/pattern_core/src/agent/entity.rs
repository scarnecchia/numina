//! Agent entity definition for database persistence
//!
//! This module defines the Agent struct that represents the persistent state
//! of a DatabaseAgent. It includes all fields that need to be stored in the
//! database and can be used to reconstruct a DatabaseAgent instance.

use crate::agent::AgentType;
use crate::context::{CompressionStrategy, ContextConfig};
use crate::id::{AgentId, EventId, MemoryId, RelationId, TaskId, UserId};
use crate::memory::MemoryBlock;
use chrono::{DateTime, Utc};
use ferroid::{Base32SnowExt, SnowflakeGeneratorAsyncTokioExt, SnowflakeMastodonId};
use pattern_macros::Entity;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt;
use std::str::FromStr;
use std::sync::OnceLock;

/// Wrapper type for Snowflake IDs with proper serde support
#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SnowflakePosition(pub SnowflakeMastodonId);

impl SnowflakePosition {
    /// Create a new snowflake position
    pub fn new(id: SnowflakeMastodonId) -> Self {
        Self(id)
    }
}

impl fmt::Display for SnowflakePosition {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Use the efficient base32 encoding via Display
        write!(f, "{}", self.0)
    }
}

impl FromStr for SnowflakePosition {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        // Try parsing as base32 first
        if let Ok(id) = SnowflakeMastodonId::decode(s) {
            return Ok(Self(id));
        }

        // Fall back to parsing as raw u64
        s.parse::<u64>()
            .map(|raw| Self(SnowflakeMastodonId::from_raw(raw)))
            .map_err(|e| format!("Failed to parse snowflake as base32 or u64: {}", e))
    }
}

impl Serialize for SnowflakePosition {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        // Serialize as string using Display
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for SnowflakePosition {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        // Deserialize from string and parse
        let s = String::deserialize(deserializer)?;
        s.parse::<Self>().map_err(serde::de::Error::custom)
    }
}

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

/// Get the next message position synchronously
///
/// This is designed for use in synchronous contexts like Default impls.
/// In practice, we don't generate messages fast enough to hit the sequence
/// limit (65536/ms), so Pending should rarely happen in production.
///
/// When the sequence is exhausted (e.g., in parallel tests), this will block
/// briefly until the next millisecond boundary to get a fresh sequence.
pub fn get_next_message_position_sync() -> SnowflakePosition {
    use ferroid::IdGenStatus;

    let generator = get_position_generator();

    loop {
        match generator.next_id() {
            IdGenStatus::Ready { id } => return SnowflakePosition::new(id),
            IdGenStatus::Pending { yield_for } => {
                // If yield_for is 0, we're at the sequence limit but still in the same millisecond.
                // Wait at least 1ms to roll over to the next millisecond and reset the sequence.
                let wait_ms = yield_for.max(1) as u64;
                std::thread::sleep(std::time::Duration::from_millis(wait_ms));
                // Loop will retry after the wait
            }
        }
    }
}

/// Get the next message position as a Snowflake ID (async version)
pub async fn get_next_message_position() -> SnowflakePosition {
    let id = get_position_generator()
        .try_next_id_async()
        .await
        .expect("for now we are assuming this succeeds");
    SnowflakePosition::new(id)
}

/// Get the next message position as a String (for database storage)
pub async fn get_next_message_position_string() -> String {
    get_next_message_position().await.to_string()
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
            model_id: None,
            model_config: HashMap::new(),
            base_instructions: String::new(),
            max_messages: 50,
            max_message_age_hours: 24,
            compression_threshold: 30,
            memory_char_limit: 5000,
            enable_thinking: true,
            compression_strategy: CompressionStrategy::Truncate { keep_recent: 100 },
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
    /// Store agent with relations individually to avoid payload size limits
    ///
    /// This is an alternative to store_with_relations() that works better
    /// with remote database instances that have request size limits.
    /// Each memory and message is stored separately rather than in bulk.
    pub async fn store_with_relations_individually<C>(
        &self,
        db: &surrealdb::Surreal<C>,
    ) -> Result<Self, crate::db::DatabaseError>
    where
        C: surrealdb::Connection + Clone,
    {
        // First store the agent itself without relations
        let mut agent_only = self.clone();
        agent_only.memories.clear();
        agent_only.messages.clear();
        agent_only.assigned_task_ids.clear();
        agent_only.scheduled_event_ids.clear();

        let stored_agent = agent_only.store_with_relations(db).await?;

        // Store each memory and its relation individually
        if !self.memories.is_empty() {
            tracing::info!("Storing {} memory blocks...", self.memories.len());
            for (i, (memory, relation)) in self.memories.iter().enumerate() {
                // Store the memory block with its own relations
                memory.store_with_relations(db).await?;
                // Create the agent-memory relation
                crate::db::ops::create_relation_typed(db, relation).await?;

                let stored = i + 1;
                tracing::info!("  Stored {}/{} memories", stored, self.memories.len());
            }
            tracing::info!("Finished storing {} memories", self.memories.len());
        }

        // Store each message and its relation individually
        if !self.messages.is_empty() {
            tracing::info!("Storing {} messages...", self.messages.len());
            for (i, (message, relation)) in self.messages.iter().enumerate() {
                // Store the message with its own relations
                message.store_with_relations(db).await?;
                // Create the agent-message relation
                crate::db::ops::create_relation_typed(db, relation).await?;

                let stored = i + 1;
                if stored % 100 == 0 || stored == self.messages.len() {
                    tracing::info!("  Stored {}/{} messages", stored, self.messages.len());
                }
            }
            tracing::info!("Finished storing {} messages", self.messages.len());
        }

        Ok(stored_agent)
    }

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
            max_context_tokens: None,
            enable_thinking: self.enable_thinking,
            tool_usage_rules: vec![],
            tool_workflow_rules: vec![],
            model_adjustments,
            consent_required_tools: vec![],
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
                r#"SELECT *, out.position as snowflake, batch, sequence_num, out.created_at AS msg_created FROM agent_messages
                   WHERE in = $agent_id AND batch IS NOT NULL
                   ORDER BY batch NUMERIC ASC, sequence_num NUMERIC ASC, snowflake NUMERIC ASC, msg_created ASC"#
            )
        } else {
            format!(
                r#"SELECT *, out.position as snowflake, batch, sequence_num, message_type, out.created_at AS msg_created FROM agent_messages
                   WHERE in = $agent_id AND message_type = 'active' AND batch IS NOT NULL
                   ORDER BY batch NUMERIC ASC, sequence_num NUMERIC ASC, snowflake NUMERIC ASC, msg_created ASC"#
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
                "Loading message for relation: message_id={:?}, type={:?}, position={:?}",
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
            "Total messages loaded for agent {}: {} (sorted by message created_at in query)",
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
        // Generate relation position at attach time
        let position = get_position_generator()
            .try_next_id_async()
            .await
            .expect("for now we are assuming this succeeds");

        // Attempt to load the message to copy batch sequencing metadata
        let (batch, sequence_num, batch_type) = if let Some(msg) =
            crate::message::Message::load_with_relations(db, message_id).await?
        {
            (msg.batch, msg.sequence_num, msg.batch_type)
        } else {
            (None, None, None)
        };

        // Create the relation using the edge entity
        let relation = crate::message::AgentMessageRelation {
            id: RelationId::nil(),
            in_id: self.id.clone(),
            out_id: message_id.clone(),
            message_type,
            position: Some(SnowflakePosition(position)),
            added_at: chrono::Utc::now(),
            batch,
            sequence_num,
            batch_type,
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
    }

    #[tokio::test]
    async fn test_agent_message_relationships() {
        use crate::db::client;
        use crate::message::{Message, MessageRelationType};
        use crate::test_helpers::messages::simple_user_assistant_batch;

        let db = client::create_test_db().await.unwrap();

        // Create an agent
        let agent = AgentRecord {
            name: "Test Agent".to_string(),
            agent_type: AgentType::Generic,
            owner_id: UserId::generate(),
            ..Default::default()
        };
        let stored_agent = agent.store_with_relations(&db).await.unwrap();

        // Create messages within a batch so relations get batch/sequence
        let (msg1, mut msg2, _batch_id) =
            simple_user_assistant_batch("First message", "Agent response");
        // Keep msg3 archived and separate (in its own batch)
        let separate_batch = crate::agent::get_next_message_position_sync();
        let msg3 = Message::user_in_batch(separate_batch, 0, "Another message");

        // Ensure batch_type is set for assistant if needed
        if msg2.batch_type.is_none() {
            msg2.batch_type = Some(crate::message::BatchType::UserRequest);
        }

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
