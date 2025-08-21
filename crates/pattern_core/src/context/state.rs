//! Agent state management for building stateful agents on stateless protocols
//!
//! This module provides the infrastructure for managing agent state between
//! conversations, including message history, memory persistence, and context rebuilding.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use surrealdb::RecordId;
use tokio::sync::{RwLock, watch};

use std::sync::Arc;

use crate::{
    AgentId, AgentState, AgentType, CoreError, IdType, ModelProvider, Result,
    db::{DatabaseError, DbEntity},
    id::MessageId,
    memory::{Memory, MemoryBlock, MemoryPermission, MemoryType},
    message::{Message, MessageContent, ToolCall, ToolResponse},
    tool::ToolRegistry,
};

use super::{
    CompressionResult, CompressionStrategy, ContextBuilder, ContextConfig, MemoryContext,
    MessageCompressor,
};

/// Cheap handle to agent internals that built-in tools can hold
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentHandle {
    /// The agent's display name
    pub name: String,
    /// Unique identifier for this agent
    pub agent_id: AgentId,
    /// Type of agent (e.g., memgpt_agent, custom)
    pub agent_type: AgentType,

    /// The agent's memory system (already cheap to clone via Arc<DashMap>)
    pub memory: Memory,
    /// The agent's current state
    pub state: AgentState,

    /// Watch channel for state changes
    #[serde(skip)]
    pub(crate) state_watch: Option<Arc<(watch::Sender<AgentState>, watch::Receiver<AgentState>)>>,

    /// Private database connection for controlled access
    #[serde(skip)]
    db: Option<surrealdb::Surreal<surrealdb::engine::any::Any>>,

    /// Message router for sending messages to various targets
    #[serde(skip)]
    pub(crate) message_router: Option<super::message_router::AgentMessageRouter>,
}

impl AgentHandle {
    /// Get a watch receiver for state changes
    pub fn state_receiver(&self) -> Option<watch::Receiver<AgentState>> {
        self.state_watch.as_ref().map(|arc| arc.1.clone())
    }

    /// Update the state and notify watchers
    pub(crate) fn update_state(&mut self, new_state: AgentState) {
        self.state = new_state.clone();
        if let Some(arc) = &self.state_watch {
            let _ = arc.0.send(new_state);
        }
    }

    /// Create a new handle with a database connection
    pub fn with_db(mut self, db: surrealdb::Surreal<surrealdb::engine::any::Any>) -> Self {
        self.db = Some(db);
        self
    }

    /// Check if this handle has a database connection
    pub fn has_db_connection(&self) -> bool {
        self.db.is_some()
    }

    /// Set the message router for this handle
    pub fn with_message_router(
        mut self,
        router: super::message_router::AgentMessageRouter,
    ) -> Self {
        self.message_router = Some(router);
        self
    }

    /// Get the message router for this handle
    pub fn message_router(&self) -> Option<&super::message_router::AgentMessageRouter> {
        self.message_router.as_ref()
    }

    /// Search archival memories directly from the database
    /// This avoids loading all archival memories into RAM
    pub async fn search_archival_memories(
        &self,
        query: &str,
        limit: usize,
    ) -> Result<Vec<MemoryBlock>> {
        let db = self.db.as_ref().ok_or_else(|| {
            crate::db::DatabaseError::QueryFailed(surrealdb::Error::Api(
                surrealdb::error::Api::InvalidParams(
                    "No database connection available for archival search".into(),
                ),
            ))
        })?;

        // Single-step query using graph traversal
        // Need to construct the full record reference inline
        let sql = format!(
            r#"
            SELECT * FROM mem
            WHERE (<-agent_memories<-agent:⟨{}⟩..)
            AND memory_type = 'archival'
            AND value @@ $search_term
            LIMIT $limit
        "#,
            self.agent_id.to_key()
        );

        tracing::debug!(
            "Executing search with query='{}' for agent={}",
            query,
            self.agent_id
        );

        let mut result = db
            .query(&sql)
            .bind(("search_term", query.to_string()))
            .bind(("limit", limit))
            .await
            .map_err(|e| {
                crate::log_error!("Search query failed", e);
                crate::db::DatabaseError::QueryFailed(e)
            })?;

        tracing::debug!("search results: {:#?}", result);

        let blocks: Vec<<MemoryBlock as DbEntity>::DbModel> =
            result.take(0).map_err(DatabaseError::from)?;

        let blocks: Vec<MemoryBlock> = blocks
            .into_iter()
            .map(|b| MemoryBlock::from_db_model(b).expect("model type should convert"))
            .collect();

        Ok(blocks)
    }

    /// Insert a new archival memory to in-memory storage
    /// Database persistence happens automatically via persist_memory_changes
    pub async fn insert_archival_memory(&self, label: &str, content: &str) -> Result<MemoryBlock> {
        // Create the memory block in the DashMap
        self.memory.create_block(label, content)?;

        // Update it to be archival type
        if let Some(mut block) = self.memory.get_block_mut(label) {
            block.memory_type = MemoryType::Archival;
            block.permission = MemoryPermission::ReadWrite;
        }

        // Get the created block
        let block = self
            .memory
            .get_block(label)
            .ok_or_else(|| crate::CoreError::MemoryNotFound {
                agent_id: self.agent_id.to_string(),
                block_name: label.to_string(),
                available_blocks: self.memory.list_blocks(),
            })?
            .clone();

        Ok(block)
    }

    /// Insert a new working memory to in-memory storage
    /// Database persistence happens automatically via persist_memory_changes
    pub async fn insert_working_memory(&self, label: &str, content: &str) -> Result<MemoryBlock> {
        // Create the memory block in the DashMap
        self.memory.create_block(label, content)?;

        // Update it to be working type
        if let Some(mut block) = self.memory.get_block_mut(label) {
            block.memory_type = MemoryType::Working;
            block.permission = MemoryPermission::ReadWrite;
        }

        // Get the created block
        let block = self
            .memory
            .get_block(label)
            .ok_or_else(|| crate::CoreError::MemoryNotFound {
                agent_id: self.agent_id.to_string(),
                block_name: label.to_string(),
                available_blocks: self.memory.list_blocks(),
            })?
            .clone();

        Ok(block)
    }

    /// Get an archival memory by exact label from the database
    pub async fn get_archival_memory_by_label(&self, label: &str) -> Result<Option<MemoryBlock>> {
        let db = self.db.as_ref().ok_or_else(|| {
            crate::db::DatabaseError::QueryFailed(surrealdb::Error::Api(
                surrealdb::error::Api::InvalidParams(
                    "No database connection available for archival get".into(),
                ),
            ))
        })?;

        crate::db::ops::get_memory_by_label(db, self.agent_id.clone(), label)
            .await
            .map_err(|e| e.into())
    }

    /// Delete an archival memory from the database
    pub async fn delete_archival_memory(&self, label: &str) -> Result<()> {
        let db = self.db.as_ref().ok_or_else(|| {
            crate::db::DatabaseError::QueryFailed(surrealdb::Error::Api(
                surrealdb::error::Api::InvalidParams(
                    "No database connection available for archival delete".into(),
                ),
            ))
        })?;

        // First find the memory with this label
        let sql = r#"
            DELETE FROM mem
            WHERE owner_id = $owner_id
            AND label = $label
            AND memory_type = $memory_type
        "#;

        db.query(sql)
            .bind(("owner_id", surrealdb::RecordId::from(&self.memory.owner_id)))
            .bind(("label", label.to_string()))
            .bind(("memory_type", "archival"))
            .await
            .map_err(|e| crate::db::DatabaseError::QueryFailed(e))?;

        Ok(())
    }

    /// Archive messages in the database and store the summary
    pub async fn archive_messages(
        &self,
        message_ids: Vec<crate::MessageId>,
        summary: Option<String>,
    ) -> Result<()> {
        let db = self.db.as_ref().ok_or_else(|| {
            crate::db::DatabaseError::QueryFailed(surrealdb::Error::Api(
                surrealdb::error::Api::InvalidParams(
                    "No database connection available for archiving messages".into(),
                ),
            ))
        })?;

        if !message_ids.is_empty() {
            // Update the agent_messages relations to mark messages as archived
            // We need to update the edge records, not the messages themselves
            let sql = r#"
                UPDATE agent_messages
                SET message_type = 'archived'
                WHERE in = $agent_id
                AND out IN $message_ids
            "#;

            db.query(sql)
                .bind(("agent_id", surrealdb::RecordId::from(self.agent_id.clone())))
                .bind((
                    "message_ids",
                    message_ids
                        .iter()
                        .map(|id| surrealdb::RecordId::from(id))
                        .collect::<Vec<_>>(),
                ))
                .await
                .map_err(|e| crate::db::DatabaseError::QueryFailed(e))?;

            tracing::info!(
                "Archived {} messages for agent {} in database",
                message_ids.len(),
                self.agent_id
            );
        }

        // Store the summary if provided
        if let Some(summary_text) = summary {
            // Update agent record with the archive summary
            // The field is called message_summary in AgentRecord
            let sql = r#"
                UPDATE agent
                SET message_summary = $summary,
                    updated_at = time::now()
                WHERE id = $id
            "#;

            db.query(sql)
                .bind(("id", surrealdb::RecordId::from(self.agent_id.clone())))
                .bind(("summary", summary_text))
                .await
                .map_err(|e| crate::db::DatabaseError::QueryFailed(e))?;

            tracing::info!("Stored archive summary for agent {}", self.agent_id);
        }

        Ok(())
    }

    /// Count archival memories for this agent
    pub async fn count_archival_memories(&self) -> Result<usize> {
        let db = self.db.as_ref().ok_or_else(|| {
            crate::db::DatabaseError::QueryFailed(surrealdb::Error::Api(
                surrealdb::error::Api::InvalidParams("No database connection available".into()),
            ))
        })?;

        let sql = r#"
            SELECT count() as count FROM mem
            WHERE owner_id = $owner_id
            AND memory_type = $memory_type
            GROUP ALL
        "#;

        let mut result = db
            .query(sql)
            .bind(("owner_id", surrealdb::RecordId::from(&self.memory.owner_id)))
            .bind(("memory_type", "archival"))
            .await
            .map_err(|e| crate::db::DatabaseError::QueryFailed(e))?;

        let count_result: Option<serde_json::Value> = result
            .take("count")
            .map_err(|e| crate::db::DatabaseError::QueryFailed(e))?;

        match count_result {
            Some(serde_json::Value::Number(n)) => Ok(n.as_u64().unwrap_or(0) as usize),
            _ => Ok(0),
        }
    }

    /// Search conversation messages with filters
    pub async fn search_conversations(
        &self,
        query: Option<&str>,
        role_filter: Option<crate::message::ChatRole>,
        start_time: Option<DateTime<Utc>>,
        end_time: Option<DateTime<Utc>>,
        limit: usize,
    ) -> Result<Vec<crate::message::Message>> {
        let db = self.db.as_ref().ok_or_else(|| {
            crate::db::DatabaseError::QueryFailed(surrealdb::Error::Api(
                surrealdb::error::Api::InvalidParams(
                    "No database connection available for conversation search".into(),
                ),
            ))
        })?;

        // If we have a content search, we need to do two queries:
        // 1. Get all message IDs belonging to this agent
        // 2. Query msg table directly with content search
        // Then filter in code
        let (sql, _needs_messages_extraction, _needs_agent_filtering) = if query.is_some() {
            // Build conditions for msg table query (without agent filter)
            let mut conditions = vec!["content @@ $search_query".to_string()];

            if role_filter.is_some() {
                conditions.push("role = $role".to_string());
            }
            if start_time.is_some() {
                conditions.push("created_at >= $start_time".to_string());
            }
            if end_time.is_some() {
                conditions.push("created_at <= $end_time".to_string());
            }

            // Query msg table directly with content search (we'll filter by agent after)
            let sql = format!(
                "SELECT * FROM msg WHERE {} ORDER BY batch NUMERIC DESC, sequence_num NUMERIC DESC, position NUMERIC DESC, created_at DESC LIMIT {}",
                conditions.join(" AND "),
                limit * 10 // Get more results since we'll filter some out
            );
            (sql, false, true)
        } else {
            // No content search - use existing graph traversal approach
            let mut conditions = vec![];
            if role_filter.is_some() {
                conditions.push("role = $role");
            }
            if start_time.is_some() {
                conditions.push("created_at >= $start_time");
            }
            if end_time.is_some() {
                conditions.push("created_at <= $end_time");
            }

            let where_clause = if conditions.is_empty() {
                String::new()
            } else {
                format!(" WHERE {}", conditions.join(" AND "))
            };

            // Query the agent_messages relation table directly
            let sql = format!(
                "SELECT position, batch, sequence_num, ->(msg{}) AS messages FROM agent_messages WHERE (in = agent:{} AND out IS NOT NULL) ORDER BY batch NUMERIC DESC, sequence_num NUMERIC DESC, position NUMERIC DESC LIMIT $limit FETCH messages",
                where_clause,
                self.agent_id.to_key()
            );
            (sql, true, false)
        };

        // Build query and bind all parameters
        let mut query_builder = db.query(&sql).bind(("limit", limit));

        if let Some(search_query) = query {
            query_builder = query_builder.bind(("search_query", search_query.to_string()));
        }

        if let Some(role) = &role_filter {
            query_builder = query_builder.bind(("role", role.to_string()));
        }

        if let Some(start) = start_time {
            query_builder =
                query_builder.bind(("start_time", surrealdb::sql::Datetime::from(start)));
        }

        if let Some(end) = end_time {
            query_builder = query_builder.bind(("end_time", surrealdb::sql::Datetime::from(end)));
        }

        // Execute the query
        let mut result = query_builder.await.map_err(DatabaseError::from)?;

        // Extract messages based on query type
        let mut messages: Vec<Message> = if _needs_messages_extraction {
            // Graph traversal query - messages are nested under "messages" field
            let db_messages: Vec<Vec<<Message as DbEntity>::DbModel>> =
                result.take("messages").map_err(DatabaseError::from)?;

            db_messages
                .into_iter()
                .flatten()
                .map(|m| Message::from_db_model(m).expect("message should convert from db model"))
                .collect()
        } else {
            // Direct msg table query - messages are at the top level
            let db_messages: Vec<<Message as DbEntity>::DbModel> =
                result.take(0).map_err(DatabaseError::from)?;

            let mut converted_messages: Vec<Message> = db_messages
                .into_iter()
                .map(|m| Message::from_db_model(m).expect("message should convert from db model"))
                .collect();

            // If we did a content search, we need to filter by agent
            if _needs_agent_filtering {
                // Get all message IDs belonging to this agent
                let agent_record_id: RecordId = self.agent_id.clone().into();
                let agent_msg_sql = format!(
                    "SELECT out FROM agent_messages WHERE in = {} AND out IS NOT NULL",
                    agent_record_id
                );

                let mut agent_msg_result = db
                    .query(&agent_msg_sql)
                    .await
                    .map_err(DatabaseError::from)?;

                // Debug: print the raw result
                //tracing::info!("Agent messages query raw result: {:?}", agent_msg_result);

                // Extract the "out" field from each result object
                #[derive(serde::Deserialize)]
                struct OutRecord {
                    out: RecordId,
                }

                let out_records: Vec<OutRecord> =
                    agent_msg_result.take(0).map_err(DatabaseError::from)?;

                let agent_msg_ids: Vec<RecordId> = out_records.into_iter().map(|r| r.out).collect();

                let agent_msg_id_set: std::collections::HashSet<RecordId> =
                    agent_msg_ids.into_iter().collect();

                // Filter messages to only those belonging to this agent
                converted_messages.retain(|msg| {
                    let msg_record_id = RecordId::from((MessageId::PREFIX, msg.id.to_key()));
                    agent_msg_id_set.contains(&msg_record_id)
                });
            }

            converted_messages
        };

        // Apply limit in application code since we may get more than limit from the query
        messages.truncate(limit);

        Ok(messages)
    }

    /// Search messages from all agents in the same constellation
    pub async fn search_constellation_messages(
        &self,
        query: Option<&str>,
        role_filter: Option<crate::message::ChatRole>,
        start_time: Option<DateTime<Utc>>,
        end_time: Option<DateTime<Utc>>,
        limit: usize,
    ) -> Result<Vec<(String, crate::message::Message)>> {
        let db = self.db.as_ref().ok_or_else(|| {
            crate::db::DatabaseError::QueryFailed(surrealdb::Error::Api(
                surrealdb::error::Api::InvalidParams(
                    "No database connection available for constellation message search".into(),
                ),
            ))
        })?;

        // First, find the constellation this agent belongs to
        let constellation_query = format!(
            "SELECT VALUE <-constellation_agents<-constellation FROM agent:{} LIMIT 1",
            self.agent_id.to_key()
        );

        let mut result = db
            .query(&constellation_query)
            .await
            .map_err(DatabaseError::from)?;
        let constellation_ids: Vec<surrealdb::RecordId> =
            result.take(0).map_err(DatabaseError::from)?;

        if constellation_ids.is_empty() {
            // Agent not in a constellation, return empty
            return Ok(Vec::new());
        }

        let constellation_id = &constellation_ids[0];

        // First, get all agents in the constellation with their names
        let agents_query = format!(
            r#"SELECT id, name FROM agent WHERE id IN (
                -- Direct agents in the constellation
                SELECT VALUE out FROM constellation_agents
                WHERE in = {}
                -- UNION with agents from groups in the constellation
                UNION
                SELECT VALUE in FROM group_members
                WHERE out IN (
                    SELECT VALUE out FROM composed_of
                    WHERE in = {}
                )
            )"#,
            constellation_id, constellation_id
        );

        let mut agents_result = db.query(&agents_query).await.map_err(DatabaseError::from)?;
        let agents_data: Vec<serde_json::Value> =
            agents_result.take(0).map_err(DatabaseError::from)?;

        // Build a map of agent ID to name
        let mut agent_names = std::collections::HashMap::new();
        for agent in agents_data {
            if let (Some(id), Some(name)) = (
                agent.get("id").and_then(|v| v.as_str()),
                agent.get("name").and_then(|v| v.as_str()),
            ) {
                agent_names.insert(id.to_string(), name.to_string());
            }
        }

        // Build conditions for msg filtering
        let mut conditions = vec![];
        if query.is_some() {
            conditions.push("content @@ $search_query");
        }
        if role_filter.is_some() {
            conditions.push("role = $role");
        }
        if start_time.is_some() {
            conditions.push("created_at >= $start_time");
        }
        if end_time.is_some() {
            conditions.push("created_at <= $end_time");
        }

        // Build WHERE clause for msg filtering
        let where_clause = if conditions.is_empty() {
            String::new()
        } else {
            format!(" WHERE {}", conditions.join(" AND "))
        };

        // Build query to get messages from all constellation agents using efficient traversal
        let sql = format!(
            r#"SELECT position, in AS agent_id, ->(msg{}) AS messages FROM agent_messages
            WHERE in IN (
                -- Direct agents in the constellation
                SELECT VALUE out FROM constellation_agents
                WHERE in = {}
                -- UNION with agents from groups in the constellation
                UNION
                SELECT VALUE in FROM group_members
                WHERE out IN (
                    SELECT VALUE out FROM composed_of
                    WHERE in = {}
                )
            ) AND out IS NOT NULL
            ORDER BY position DESC
            LIMIT $limit
            FETCH messages"#,
            where_clause, constellation_id, constellation_id
        );

        // Build query and bind all parameters
        let mut query_builder = db.query(&sql).bind(("limit", limit));

        if let Some(search_query) = query {
            query_builder = query_builder.bind(("search_query", search_query.to_string()));
        }

        if let Some(role) = &role_filter {
            query_builder = query_builder.bind(("role", role.to_string()));
        }

        if let Some(start) = start_time {
            query_builder =
                query_builder.bind(("start_time", surrealdb::sql::Datetime::from(start)));
        }

        if let Some(end) = end_time {
            query_builder = query_builder.bind(("end_time", surrealdb::sql::Datetime::from(end)));
        }

        // Execute query and get results with agent names
        let results: Vec<serde_json::Value> = query_builder
            .await
            .map_err(DatabaseError::from)?
            .take(0)
            .map_err(DatabaseError::from)?;

        // Parse results to extract agent ID and message ID, then fetch messages
        let mut messages_with_agents = Vec::new();
        for result in results {
            if let (Some(agent_id_val), Some(message_id_val)) =
                (result.get("agent_id"), result.get("message"))
            {
                // Extract the agent ID string
                let agent_id = if let Some(obj) = agent_id_val.as_object() {
                    if let Some(id_str) = obj.get("id").and_then(|v| v.as_str()) {
                        id_str
                    } else {
                        continue;
                    }
                } else if let Some(id_str) = agent_id_val.as_str() {
                    id_str
                } else {
                    continue;
                };

                // Look up agent name from our map
                let agent_name = agent_names
                    .get(agent_id)
                    .cloned()
                    .unwrap_or_else(|| format!("Unknown ({})", agent_id));

                // Extract message ID and fetch the message
                let message_id = if let Some(obj) = message_id_val.as_object() {
                    if let (Some(tb), Some(id)) = (
                        obj.get("tb").and_then(|v| v.as_str()),
                        obj.get("id").and_then(|v| v.as_str()),
                    ) {
                        format!("{}:{}", tb, id)
                    } else {
                        continue;
                    }
                } else if let Some(id_str) = message_id_val.as_str() {
                    id_str.to_string()
                } else {
                    continue;
                };

                // Fetch the actual message
                let msg_query = format!("SELECT * FROM {}", message_id);
                if let Ok(mut msg_result) = db.query(&msg_query).await {
                    if let Ok(msg_data) = msg_result.take::<Vec<serde_json::Value>>(0) {
                        if let Some(msg_val) = msg_data.into_iter().next() {
                            // Convert to Message
                            if let Ok(db_model) =
                                serde_json::from_value::<<Message as DbEntity>::DbModel>(msg_val)
                            {
                                if let Ok(message) = Message::from_db_model(db_model) {
                                    // Apply filters
                                    if let Some(q) = query {
                                        if let Some(text) = message.text_content() {
                                            if !text.to_lowercase().contains(&q.to_lowercase()) {
                                                continue;
                                            }
                                        } else {
                                            continue;
                                        }
                                    }

                                    if let Some(role) = &role_filter {
                                        if message.role != *role {
                                            continue;
                                        }
                                    }

                                    if let Some(start) = start_time {
                                        if message.created_at < start {
                                            continue;
                                        }
                                    }

                                    if let Some(end) = end_time {
                                        if message.created_at > end {
                                            continue;
                                        }
                                    }

                                    messages_with_agents.push((agent_name, message));

                                    // Check if we've reached the limit
                                    if messages_with_agents.len() >= limit {
                                        break;
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        Ok(messages_with_agents)
    }
}

impl Default for AgentHandle {
    fn default() -> Self {
        let state = AgentState::Ready;
        let (tx, rx) = watch::channel(state.clone());
        Self {
            name: "".to_string(),
            agent_id: AgentId::generate(),
            memory: Memory::new(),
            state,
            agent_type: AgentType::Generic,
            state_watch: Some(Arc::new((tx, rx))),
            db: None,
            message_router: None,
        }
    }
}

impl AgentHandle {
    /// Create a test handle with custom memory
    #[cfg(test)]
    pub fn test_with_memory(memory: Memory) -> Self {
        Self {
            name: "test_agent".to_string(),
            agent_id: AgentId::generate(),
            memory,
            state: AgentState::Ready,
            agent_type: AgentType::Generic,
            state_watch: None,
            db: None,
            message_router: None,
        }
    }
}

/// Message history that needs locking
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MessageHistory {
    /// Active batches in the current context window
    pub batches: Vec<crate::message::MessageBatch>,
    /// Batches that have been compressed/archived to save context space
    pub archived_batches: Vec<crate::message::MessageBatch>,
    /// Optional summary of archived batches
    #[serde(skip_serializing_if = "Option::is_none")]
    pub archive_summary: Option<String>,
    /// Strategy used for compressing messages when context is full
    pub compression_strategy: CompressionStrategy,
    /// When compression was last performed
    pub last_compression: DateTime<Utc>,
}

impl MessageHistory {
    /// Get the total count of messages across all batches
    pub fn total_message_count(&self) -> usize {
        self.batches.iter().map(|b| b.messages.len()).sum()
    }

    /// Create a new message history with the specified compression strategy
    pub fn new(compression_strategy: CompressionStrategy) -> Self {
        Self {
            batches: Vec::new(),
            archived_batches: Vec::new(),
            archive_summary: None,
            compression_strategy,
            last_compression: Utc::now(),
        }
    }

    /// Add a message to its batch (uses message.batch if present)
    pub fn add_message(&mut self, message: Message) -> Message {
        let batch_id = message
            .batch
            .unwrap_or_else(crate::agent::get_next_message_position_sync);
        self.add_message_to_batch(batch_id, message)
    }

    /// Add a message to a specific batch
    pub fn add_message_to_batch(
        &mut self,
        batch_id: crate::agent::SnowflakePosition,
        mut message: Message,
    ) -> Message {
        use crate::message::{BatchType, MessageBatch};

        // Validate/set batch_id
        if let Some(msg_batch_id) = message.batch {
            assert_eq!(msg_batch_id, batch_id, "Message batch_id doesn't match");
        } else {
            message.batch = Some(batch_id);
        }

        // Find or create batch
        if let Some(batch) = self.batches.iter_mut().find(|b| b.id == batch_id) {
            batch.add_message(message)
        } else {
            // Create new batch - infer type from message role
            let batch_type = message.batch_type.unwrap_or_else(|| {
                match message.role {
                    crate::message::ChatRole::User => BatchType::UserRequest,
                    crate::message::ChatRole::System => BatchType::SystemTrigger,
                    _ => BatchType::UserRequest, // Default
                }
            });

            // Clone the message since from_messages consumes it
            let message_clone = message.clone();
            let batch = MessageBatch::from_messages(batch_id, batch_type, vec![message]);
            self.batches.push(batch);
            message_clone
        }
    }

    /// Add an entire batch
    pub fn add_batch(&mut self, batch: crate::message::MessageBatch) {
        // Could check if batch.id already exists and merge, or just add
        self.batches.push(batch);
    }
}

/// Represents the complete state of an agent
#[derive(Clone)]
pub struct AgentContext {
    /// Cheap, frequently accessed stuff
    pub handle: AgentHandle,

    /// Tools available to this agent
    pub tools: ToolRegistry,

    /// Configuration for context building
    pub context_config: ContextConfig,

    /// Metadata about the agent state
    pub metadata: Arc<RwLock<AgentContextMetadata>>,

    /// The big stuff in its own lock
    pub history: Arc<RwLock<MessageHistory>>,

    /// Optional constellation activity tracker for shared context
    pub constellation_tracker:
        Option<Arc<crate::constellation_memory::ConstellationActivityTracker>>,

    /// Model provider for compression strategies that need it
    pub(crate) model_provider: Option<Arc<dyn ModelProvider>>,
}

/// Metadata about the agent state
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentContextMetadata {
    pub created_at: DateTime<Utc>,
    pub last_active: DateTime<Utc>,
    pub total_messages: usize,
    pub total_tool_calls: usize,
    pub context_rebuilds: usize,
    pub compression_events: usize,
}

impl Default for AgentContextMetadata {
    fn default() -> Self {
        Self {
            created_at: Utc::now(),
            last_active: Utc::now(),
            total_messages: 0,
            total_tool_calls: 0,
            context_rebuilds: 0,
            compression_events: 0,
        }
    }
}

impl AgentContext {
    /// Create a new agent state
    pub fn new(
        agent_id: AgentId,
        name: String,
        agent_type: AgentType,
        memory: Memory,
        tools: ToolRegistry,
        context_config: ContextConfig,
    ) -> Self {
        let state = AgentState::Ready;
        let (tx, rx) = watch::channel(state.clone());
        let handle = AgentHandle {
            agent_id,
            memory,
            name,
            agent_type,
            state,
            state_watch: Some(Arc::new((tx, rx))),
            db: None,
            message_router: None,
        };

        Self {
            handle,
            tools,
            context_config,
            metadata: Arc::new(RwLock::new(AgentContextMetadata::default())),
            history: Arc::new(RwLock::new(MessageHistory::new(
                CompressionStrategy::default(),
            ))),
            constellation_tracker: None,
            model_provider: None,
        }
    }

    /// Get a cheap handle to agent internals
    pub fn handle(&self) -> AgentHandle {
        self.handle.clone()
    }

    /// Set the constellation activity tracker
    pub fn set_constellation_tracker(
        &mut self,
        tracker: Arc<crate::constellation_memory::ConstellationActivityTracker>,
    ) {
        self.constellation_tracker = Some(tracker);
    }

    /// Add tool workflow rules to the context configuration (merges with existing rules)
    pub fn add_tool_rules(&mut self, mut tool_rules: Vec<crate::context::ToolRule>) {
        self.context_config
            .tool_workflow_rules
            .append(&mut tool_rules);
    }

    /// Build the current context for this agent
    ///
    /// # Arguments
    /// * `current_batch_id` - The batch currently being processed (if any).
    ///   This ensures incomplete batches that are actively being processed are included.
    pub async fn build_context(
        &self,
        current_batch_id: Option<crate::agent::SnowflakePosition>,
    ) -> Result<MemoryContext> {
        {
            let mut metadata = self.metadata.write().await;
            metadata.last_active = Utc::now();
            metadata.context_rebuilds += 1;
        }

        // Check if we need to compress batches (by message count OR token count)
        {
            let history = self.history.read().await;
            let total_messages: usize = history.batches.iter().map(|b| b.len()).sum();

            // Check message count
            let needs_message_compression =
                total_messages > self.context_config.max_context_messages;

            // Check token count if we have a limit
            let needs_token_compression =
                if let Some(max_tokens) = self.context_config.max_context_tokens {
                    // Estimate current token usage
                    let memory_blocks = self.handle.memory.get_all_blocks();
                    let system_tokens = self.estimate_system_prompt_tokens(&memory_blocks);
                    let message_tokens: usize = history
                        .batches
                        .iter()
                        .flat_map(|b| &b.messages)
                        .map(|m| m.estimate_tokens())
                        .sum();
                    let total_tokens = system_tokens + message_tokens;

                    // Leave some buffer (use 80% of limit to trigger compression)
                    total_tokens > (max_tokens * 4 / 5)
                } else {
                    false
                };

            if needs_message_compression || needs_token_compression {
                drop(history); // release read lock
                tracing::info!(
                    "Triggering compression: messages={} (limit={}), tokens_exceeded={}",
                    total_messages,
                    self.context_config.max_context_messages,
                    needs_token_compression
                );
                self.compress_messages().await?;
            }
        }

        // Get memory blocks
        let memory_blocks = self.handle.memory.get_all_blocks();

        // Build context with read lock
        let history = self.history.read().await;
        let total_messages: usize = history.batches.iter().map(|b| b.len()).sum();
        tracing::debug!(
            "Building context for agent {}: {} messages across {} batches, max_context_messages={}",
            self.handle.agent_id,
            total_messages,
            history.batches.len(),
            self.context_config.max_context_messages
        );

        // Count complete vs incomplete batches
        let complete_count = history.batches.iter().filter(|b| b.is_complete).count();
        let incomplete_count = history.batches.len() - complete_count;

        tracing::debug!(
            "Context state for agent {}: {} batches ({} complete, {} incomplete), current_batch_id={:?}",
            self.handle.agent_id,
            history.batches.len(),
            complete_count,
            incomplete_count,
            current_batch_id
        );

        // Sort batches by ID (oldest to newest) before building context
        let mut sorted_batches = history.batches.clone();
        sorted_batches.sort_by_key(|b| b.id);

        let context =
            ContextBuilder::new(self.handle.agent_id.clone(), self.context_config.clone())
                .with_memory_blocks(memory_blocks)
                .with_tools_from_registry(&self.tools)
                .with_batches(sorted_batches)
                .with_archive_summary(history.archive_summary.clone())
                .build(current_batch_id)
                .await?;

        tracing::debug!(
            "Built context with {} messages, system_prompt length={} chars",
            context.len(),
            context.system_prompt.len()
        );
        for msg in &context.messages() {
            tracing::debug!("{:?}", msg);
        }

        Ok(context)
    }

    /// Add a new message to the state
    pub async fn add_message(&self, message: Message) -> Message {
        // Count tool calls for metadata
        if message.role.is_assistant() {
            if let MessageContent::ToolCalls(calls) = &message.content {
                let mut metadata = self.metadata.write().await;
                metadata.total_tool_calls += calls.len();
            } else if let MessageContent::Blocks(blocks) = &message.content {
                let mut metadata = self.metadata.write().await;
                for block in blocks {
                    if matches!(block, crate::message::ContentBlock::ToolUse { .. }) {
                        metadata.total_tool_calls += 1;
                    }
                }
            }
        }

        // Add message to history - batches handle duplicate detection and sequencing
        let mut history = self.history.write().await;
        let updated_message = history.add_message(message);

        // Update metadata
        let mut metadata = self.metadata.write().await;
        metadata.total_messages += 1;
        metadata.last_active = Utc::now();

        updated_message
    }

    /// Add a message to a specific batch
    pub async fn add_message_to_batch(
        &self,
        batch_id: crate::agent::SnowflakePosition,
        message: Message,
    ) {
        let mut history = self.history.write().await;
        history.add_message_to_batch(batch_id, message);

        let mut metadata = self.metadata.write().await;
        metadata.total_messages += 1;
        metadata.last_active = Utc::now();
    }

    /// Add an entire batch
    pub async fn add_batch(&self, batch: crate::message::MessageBatch) {
        let message_count = batch.len();
        let mut history = self.history.write().await;
        history.add_batch(batch);

        let mut metadata = self.metadata.write().await;
        metadata.total_messages += message_count;
        metadata.last_active = Utc::now();
    }

    /// Clean up errors in batches - adds failure responses then runs finalize without marking complete
    /// This is used when exiting process_message_stream early due to errors
    pub async fn cleanup_errors(
        &self,
        current_batch_id: Option<crate::agent::SnowflakePosition>,
        error_message: &str,
    ) {
        let mut history = self.history.write().await;

        // Find and clean up the current batch if specified
        if let Some(batch_id) = current_batch_id {
            if let Some(batch) = history.batches.iter_mut().find(|b| b.id == batch_id) {
                // First, add failure responses for any pending tool calls
                let pending_calls = batch.get_pending_tool_calls();
                if !pending_calls.is_empty() {
                    tracing::info!(
                        "Adding {} failure responses for pending tool calls in batch {}",
                        pending_calls.len(),
                        batch_id
                    );
                    for call_id in pending_calls {
                        let error_response = crate::message::ToolResponse {
                            call_id: call_id.clone(),
                            content: format!("Error: {}", error_message),
                        };
                        batch.add_tool_response_with_sequencing(error_response);
                    }
                }

                // Then finalize to clean up any remaining issues
                let removed = batch.finalize();
                if !removed.is_empty() {
                    tracing::warn!(
                        "Cleaned up {} unpaired tool calls in current batch {} during error handling",
                        removed.len(),
                        batch_id
                    );
                }
                // Don't mark complete - leave that decision to the caller
            }
        }

        // Also clean up any other incomplete batches (defensive)
        for batch in history.batches.iter_mut() {
            if !batch.is_complete && batch.has_pending_tool_calls() {
                // Add failure responses for pending calls
                let pending_calls = batch.get_pending_tool_calls();
                for call_id in pending_calls {
                    let error_response = crate::message::ToolResponse {
                        call_id: call_id.clone(),
                        content: format!("Error: {}", error_message),
                    };
                    batch.add_tool_response_with_sequencing(error_response);
                }

                let removed = batch.finalize();
                if !removed.is_empty() {
                    tracing::warn!(
                        "Cleaned up {} unpaired tool calls in incomplete batch {} during error handling",
                        removed.len(),
                        batch.id
                    );
                }
                // Don't mark complete - these may be resumed later
            }
        }
    }

    /// Process a single tool call and return the response
    pub async fn process_tool_call(&self, call: &ToolCall) -> Result<Option<ToolResponse>> {
        // No duplicate checking needed - batches handle this
        tracing::debug!(
            "Executing tool: {} with args: {:?}",
            call.fn_name,
            call.fn_arguments
        );

        match self
            .tools
            .execute(&call.fn_name, call.fn_arguments.clone())
            .await
        {
            Ok(tool_response) => {
                tracing::debug!("✅ Tool {} executed successfully", call.fn_name);

                let response_json = serde_json::to_string_pretty(&tool_response)
                    .unwrap_or_else(|_| "Error serializing response".to_string());

                Ok(Some(ToolResponse {
                    call_id: call.call_id.clone(),
                    content: response_json,
                }))
            }
            Err(e) => {
                tracing::warn!("❌ Tool {} failed: {}", call.fn_name, e);

                Ok(Some(ToolResponse {
                    call_id: call.call_id.clone(),
                    content: format!("Error: {:?}", e),
                }))
            }
        }
    }

    /// Update a memory block
    pub async fn update_memory_block(&self, label: &str, new_value: String) -> Result<()> {
        self.handle.memory.update_block_value(label, new_value)?;
        self.metadata.write().await.last_active = Utc::now();
        Ok(())
    }

    /// Append to a memory block
    pub async fn append_to_memory_block(&self, label: &str, content: &str) -> Result<()> {
        let current = self.handle.memory.get_block(label).ok_or_else(|| {
            CoreError::memory_not_found(
                &self.handle.agent_id,
                label,
                self.handle.memory.list_blocks(),
            )
        })?;

        let new_value = format!("{}\n{}", current.value, content);
        self.handle.memory.update_block_value(label, new_value)?;
        self.metadata.write().await.last_active = Utc::now();
        Ok(())
    }

    /// Replace content in a memory block
    pub async fn replace_in_memory_block(
        &self,
        label: &str,
        old_content: &str,
        new_content: &str,
    ) -> Result<()> {
        let current = self.handle.memory.get_block(label).ok_or_else(|| {
            CoreError::memory_not_found(
                &self.handle.agent_id,
                label,
                self.handle.memory.list_blocks(),
            )
        })?;

        let new_value = current.value.replace(old_content, new_content);
        self.handle.memory.update_block_value(label, new_value)?;
        self.metadata.write().await.last_active = Utc::now();
        Ok(())
    }

    /// Estimate the token count for system prompt and memory blocks
    fn estimate_system_prompt_tokens(&self, memory_blocks: &[MemoryBlock]) -> usize {
        // Build a rough approximation of the system prompt
        let mut prompt_text = String::new();

        // Add base instructions
        prompt_text.push_str(&self.context_config.base_instructions);
        prompt_text.push_str("\n\n");

        // Add metadata section (rough approximation)
        prompt_text.push_str("System Metadata:\n");
        prompt_text.push_str("Current time: ");
        prompt_text.push_str(&chrono::Utc::now().to_rfc3339());
        prompt_text.push_str("\n\n");

        // Add memory blocks
        if !memory_blocks.is_empty() {
            prompt_text.push_str("Memory Blocks:\n");
            for block in memory_blocks {
                prompt_text.push_str(&format!("[{}] ", block.label));
                prompt_text.push_str(&block.value);
                prompt_text.push_str("\n");
            }
            prompt_text.push_str("\n");
        }

        // Add tool usage rules if configured
        for rule in &self.context_config.tool_usage_rules {
            prompt_text.push_str(&rule.rule);
            prompt_text.push_str("\n");
        }

        // Estimate tokens using the same formula as Message::estimate_tokens()
        // ~4 characters per token, with a multiplier for safety
        let char_count = prompt_text.len();
        let base_tokens = char_count / 4;

        // Apply the token multiplier from model adjustments if available
        let multiplier = self.context_config.model_adjustments.token_multiplier;
        (base_tokens as f32 * multiplier) as usize
    }

    /// Force compression regardless of current limits
    /// Used when we get a "prompt too long" error from the model
    pub async fn force_compression(&self) -> Result<()> {
        let mut history = self.history.write().await;

        // Calculate system prompt tokens (including memory blocks)
        let memory_blocks = self.handle.memory.get_all_blocks();
        let system_prompt_tokens = self.estimate_system_prompt_tokens(&memory_blocks);

        let mut compressor = MessageCompressor::new(history.compression_strategy.clone())
            .with_system_prompt_tokens(system_prompt_tokens)
            .with_existing_summary(history.archive_summary.clone());

        // Add model provider if available
        if let Some(ref provider) = self.model_provider {
            compressor = compressor.with_model_provider(provider.clone());
        }

        // Sort batches by ID (oldest to newest) before compression
        history.batches.sort_by_key(|b| b.id);

        let batch_count_before = history.batches.len();
        let message_count_before: usize = history.batches.iter().map(|b| b.len()).sum();

        tracing::info!(
            "FORCING compression due to prompt too long: {} batches with {} total messages",
            batch_count_before,
            message_count_before
        );

        // Force compression by using very aggressive limits
        // Use 50% of normal limits to ensure we compress enough
        let forced_message_limit = self.context_config.max_context_messages / 2;
        let forced_token_limit = self.context_config.max_context_tokens.map(|t| t / 2);

        let result = compressor
            .compress(
                history.batches.clone(),
                forced_message_limit,
                forced_token_limit,
            )
            .await?;

        // Update history with compressed batches
        history.batches = result.active_batches;

        // Store compression metadata
        if let Some(summary) = result.summary {
            history.archive_summary = Some(summary);
        }

        history.last_compression = chrono::Utc::now();

        let batch_count_after = history.batches.len();
        let message_count_after: usize = history.batches.iter().map(|b| b.len()).sum();

        tracing::info!(
            "Force compression complete: {} -> {} batches, {} -> {} messages (removed {})",
            batch_count_before,
            batch_count_after,
            message_count_before,
            message_count_after,
            message_count_before - message_count_after
        );

        Ok(())
    }

    /// Compress messages using the configured strategy
    async fn compress_messages(&self) -> Result<()> {
        let mut history = self.history.write().await;

        // Calculate system prompt tokens (including memory blocks)
        let memory_blocks = self.handle.memory.get_all_blocks();
        let system_prompt_tokens = self.estimate_system_prompt_tokens(&memory_blocks);

        let mut compressor = MessageCompressor::new(history.compression_strategy.clone())
            .with_system_prompt_tokens(system_prompt_tokens)
            .with_existing_summary(history.archive_summary.clone());

        // Add model provider if available
        if let Some(ref provider) = self.model_provider {
            compressor = compressor.with_model_provider(provider.clone());
        }

        // Sort batches by ID (oldest to newest) before compression
        history.batches.sort_by_key(|b| b.id);

        let batch_count_before = history.batches.len();
        let message_count_before: usize = history.batches.iter().map(|b| b.len()).sum();

        tracing::debug!(
            "Starting compression: {} batches with {} total messages",
            batch_count_before,
            message_count_before
        );

        let result = compressor
            .compress(
                history.batches.clone(),
                self.context_config.max_context_messages,
                self.context_config.max_context_tokens,
            )
            .await?;

        let active_message_count: usize = result.active_batches.iter().map(|b| b.len()).sum();
        let archived_message_count: usize = result.archived_batches.iter().map(|b| b.len()).sum();

        tracing::info!(
            "Compression result: {} active batches ({} msgs), {} archived batches ({} msgs)",
            result.active_batches.len(),
            active_message_count,
            result.archived_batches.len(),
            archived_message_count
        );

        let archived_ids = self.apply_compression_result(&mut history, result)?;
        let archive_summary = history.archive_summary.clone();
        history.last_compression = Utc::now();
        self.metadata.write().await.compression_events += 1;

        // Archive the messages in the database and store the summary
        if !archived_ids.is_empty() || archive_summary.is_some() {
            if let Err(e) = self
                .handle
                .archive_messages(archived_ids.clone(), archive_summary)
                .await
            {
                tracing::error!("Failed to archive messages in database: {:?}", e);
            } else {
                tracing::info!(
                    "Successfully archived {} messages for agent {}",
                    archived_ids.len(),
                    self.handle.agent_id
                );
            }
        }

        Ok(())
    }

    /// Apply compression result to state and return archived message IDs
    fn apply_compression_result(
        &self,
        history: &mut MessageHistory,
        result: CompressionResult,
    ) -> Result<Vec<crate::MessageId>> {
        // Collect message IDs that will be archived
        let archived_ids: Vec<crate::MessageId> = result
            .archived_batches
            .iter()
            .flat_map(|batch| batch.messages.iter().map(|msg| msg.id.clone()))
            .collect();

        // Move compressed batches to archive
        history.archived_batches.extend(result.archived_batches);

        // Update active batches
        history.batches = result.active_batches;

        // Update or append to summary
        if let Some(new_summary) = result.summary {
            if let Some(existing_summary) = &mut history.archive_summary {
                *existing_summary = format!("{}\n\n{}", existing_summary, new_summary);
            } else {
                history.archive_summary = Some(new_summary);
            }
        }

        // Return the archived message IDs for the caller to persist
        Ok(archived_ids)
    }

    /// Get agent statistics
    pub async fn get_stats(&self) -> AgentStats {
        let history = self.history.read().await;
        let metadata = self.metadata.read().await;
        AgentStats {
            total_messages: metadata.total_messages,
            active_messages: history.batches.iter().map(|b| b.len()).sum(),
            archived_messages: history.archived_batches.iter().map(|b| b.len()).sum(),
            total_tool_calls: metadata.total_tool_calls,
            memory_blocks: self.handle.memory.list_blocks().len(),
            compression_events: metadata.compression_events,
            uptime: Utc::now() - metadata.created_at,
            last_active: metadata.last_active,
        }
    }

    /// Create a checkpoint of the current state
    pub async fn checkpoint(&self) -> StateCheckpoint {
        let history = self.history.read().await;
        StateCheckpoint {
            agent_id: self.handle.agent_id.clone(),
            timestamp: Utc::now(),
            batches: history.batches.clone(),
            memory_snapshot: self.handle.memory.clone(),
            metadata: self.metadata.read().await.clone(),
        }
    }

    /// Restore from a checkpoint
    pub async fn restore_from_checkpoint(&self, checkpoint: StateCheckpoint) -> Result<()> {
        if checkpoint.agent_id != self.handle.agent_id {
            return Err(CoreError::AgentInitFailed {
                agent_type: format!("{:?}", self.handle.agent_type),
                cause: "Checkpoint is for a different agent".to_string(),
            });
        }

        let mut history = self.history.write().await;
        history.batches = checkpoint.batches;
        // Note: memory is shared via handle, so we can't restore it here
        // This might need a different approach
        *self.metadata.write().await = checkpoint.metadata;
        Ok(())
    }
}

/// Statistics about an agent
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentStats {
    pub total_messages: usize,
    pub active_messages: usize,
    pub archived_messages: usize,
    pub total_tool_calls: usize,
    pub memory_blocks: usize,
    pub compression_events: usize,
    pub uptime: chrono::Duration,
    pub last_active: DateTime<Utc>,
}

/// A checkpoint of agent state
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StateCheckpoint {
    pub agent_id: AgentId,
    pub timestamp: DateTime<Utc>,
    pub batches: Vec<crate::message::MessageBatch>,
    pub memory_snapshot: Memory,
    pub metadata: AgentContextMetadata,
}

/// Builder for creating agent states
pub struct AgentContextBuilder {
    agent_id: AgentId,
    agent_type: AgentType,
    memory_blocks: Vec<(String, String, Option<String>)>,
    tools: Option<ToolRegistry>,
    context_config: Option<ContextConfig>,
    compression_strategy: Option<CompressionStrategy>,
    initial_messages: Vec<Message>,
}

impl AgentContextBuilder {
    /// Create a new agent state builder
    pub fn new(agent_id: AgentId, agent_type: AgentType) -> Self {
        Self {
            agent_id,
            agent_type,
            memory_blocks: Vec::new(),
            tools: None,
            context_config: None,
            compression_strategy: None,
            initial_messages: Vec::new(),
        }
    }

    /// Add a memory block
    pub fn with_memory_block(
        mut self,
        label: impl Into<String>,
        value: impl Into<String>,
        description: Option<impl Into<String>>,
    ) -> Self {
        self.memory_blocks
            .push((label.into(), value.into(), description.map(|d| d.into())));
        self
    }

    /// Set the tool registry
    pub fn with_tools(mut self, tools: ToolRegistry) -> Self {
        self.tools = Some(tools);
        self
    }

    /// Set the context configuration
    pub fn with_context_config(mut self, config: ContextConfig) -> Self {
        self.context_config = Some(config);
        self
    }

    /// Set the compression strategy
    pub fn with_compression_strategy(mut self, strategy: CompressionStrategy) -> Self {
        self.compression_strategy = Some(strategy);
        self
    }

    /// Add initial messages
    pub fn with_initial_messages(mut self, messages: Vec<Message>) -> Self {
        self.initial_messages = messages;
        self
    }

    /// Build the agent state
    pub async fn build(self) -> Result<AgentContext> {
        // Create memory
        let memory = Memory::new();
        for (label, value, description) in self.memory_blocks {
            memory.create_block(&label, &value)?;
            if let Some(desc) = description {
                if let Some(mut block) = memory.get_block_mut(&label) {
                    block.description = Some(desc);
                }
            }
        }

        // Get or create default tools
        let tools = self.tools.unwrap_or_else(|| ToolRegistry::new());

        // Get or create default context config
        let context_config = self.context_config.unwrap_or_default();

        // Create state
        let state = AgentContext::new(
            self.agent_id,
            "test_agent".into(),
            self.agent_type,
            memory,
            tools,
            context_config,
        );

        // Set compression strategy if provided
        if let Some(strategy) = self.compression_strategy {
            let mut history = state.history.write().await;
            history.compression_strategy = strategy;
        }

        // Add initial messages
        for message in self.initial_messages {
            state.add_message(message).await;
        }

        Ok(state)
    }
}
