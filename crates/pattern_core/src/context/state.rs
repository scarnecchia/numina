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
    message::{Message, MessageContent, MessageRelationType, ToolCall, ToolResponse},
    tool::ToolRegistry,
};

/// Search result with relevance score
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScoredMessage {
    pub message: Message,
    pub score: f32,
}

/// Search result for memory blocks with relevance score
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScoredMemoryBlock {
    pub block: MemoryBlock,
    pub score: f32,
}

/// Search result for constellation messages with relevance score
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScoredConstellationMessage {
    pub agent_name: String,
    pub message: Message,
    pub score: f32,
}

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
        let scored = self
            .search_archival_memories_with_options(query, limit, None)
            .await?;
        Ok(scored.into_iter().map(|sm| sm.block).collect())
    }

    /// Search archival memories with fuzzy search options (returns scored results)
    pub async fn search_archival_memories_with_options(
        &self,
        query: &str,
        limit: usize,
        _fuzzy_level: Option<u8>,
    ) -> Result<Vec<ScoredMemoryBlock>> {
        let db = self.db.as_ref().ok_or_else(|| {
            CoreError::database_query_error(
                "archival_search_init",
                "mem",
                surrealdb::Error::Api(surrealdb::error::Api::InvalidParams(
                    "No database connection available for archival search".into(),
                )),
            )
        })?;

        // Use @1@ to identify this search for scoring functions
        // TODO: Implement actual fuzzy search when SurrealDB supports it
        let _fuzzy = _fuzzy_level.unwrap_or(0) > 0; // Currently unused

        // Direct query to mem table with BM25 scoring, filter by owner after
        let sql = r#"
            SELECT *, value, search::score(1) AS relevance_score FROM mem
            WHERE memory_type = 'archival'
            AND value @1@ $search_term
            ORDER BY relevance_score DESC
            LIMIT $limit_multiplier
        "#;

        tracing::debug!(
            "Executing search with query='{}' for agent={}",
            query,
            self.agent_id
        );

        let mut result = db
            .query(sql)
            .bind(("search_term", query.to_string()))
            .bind(("limit_multiplier", limit * 10)) // Get more results since we'll filter
            .await
            .map_err(|e| CoreError::database_query_error("archival_search_mem", "mem", e))?;

        tracing::debug!("search results: {:#?}", result);

        // Extract results with scores - mirror the full structure
        #[derive(serde::Deserialize)]
        struct ScoredMemResult {
            id: surrealdb::RecordId,
            owner_id: surrealdb::RecordId,
            label: String,
            value: String,
            description: Option<String>,
            memory_type: crate::memory::MemoryType,
            permission: crate::memory::MemoryPermission,
            metadata: serde_json::Value,
            embedding_model: Option<String>,
            embedding: Option<Vec<f32>>,
            created_at: surrealdb::Datetime,
            updated_at: surrealdb::Datetime,
            is_active: bool,
            pinned: bool,
            relevance_score: Option<f32>,
        }

        let mut scored_results: Vec<ScoredMemResult> =
            result.take(0).map_err(DatabaseError::from)?;

        // Extract the "out" field from each result object
        #[derive(serde::Deserialize)]
        struct OutRecord {
            out: RecordId,
        }

        let mut agent_mem_id_set: std::collections::HashSet<RecordId> =
            std::collections::HashSet::new();

        let agent_mem_sql = format!(
            "SELECT out FROM agent_memories WHERE in = {} AND out IS NOT NULL",
            self.agent_id.to_string()
        );

        let mut agent_mem_result = db
            .query(&agent_mem_sql)
            .await
            .map_err(|e| CoreError::database_query_error(&agent_mem_sql, "agent_memories", e))?;

        let out_records: Vec<OutRecord> = agent_mem_result.take(0).map_err(DatabaseError::from)?;

        for record in out_records.into_iter() {
            agent_mem_id_set.insert(record.out.clone());
        }

        // Filter messages to only those belonging to this agent
        scored_results.retain(|sm| agent_mem_id_set.contains(&sm.id));

        let scored_blocks: Vec<ScoredMemoryBlock> = scored_results
            .into_iter()
            .map(|sr| {
                // Reconstruct the DbModel from our query results
                use crate::memory::MemoryBlockDbModel;
                let db_model = MemoryBlockDbModel {
                    id: sr.id,
                    owner_id: sr.owner_id,
                    label: sr.label,
                    value: sr.value,
                    description: sr.description,
                    memory_type: sr.memory_type,
                    permission: sr.permission,
                    metadata: sr.metadata,
                    embedding_model: sr.embedding_model,
                    embedding: sr.embedding,
                    created_at: sr.created_at,
                    updated_at: sr.updated_at,
                    is_active: sr.is_active,
                    pinned: sr.pinned,
                };

                let block = MemoryBlock::from_db_model(db_model)
                    .expect("memory should convert from db model");

                ScoredMemoryBlock {
                    block,
                    score: sr.relevance_score.unwrap_or(1.0),
                }
            })
            .collect();

        Ok(scored_blocks)
    }

    /// Search archival memories across all agents in the same groups as this agent
    pub async fn search_group_archival_memories_with_options(
        &self,
        query: &str,
        limit: usize,
        _fuzzy_level: Option<u8>,
    ) -> Result<Vec<ScoredMemoryBlock>> {
        let db = self.db.as_ref().ok_or_else(|| {
            crate::db::DatabaseError::QueryFailed(surrealdb::Error::Api(
                surrealdb::error::Api::InvalidParams(
                    "No database connection available for group archival search".into(),
                ),
            ))
        })?;

        let mut agents_result = db
            .query("SELECT ->group_members->group<-group_members<-agent AS agents FROM $agent_id")
            .bind(("agent_id", RecordId::from(&self.agent_id)))
            .await
            .map_err(|e| {
                crate::log_error!("Failed to get group members", e);
                crate::db::DatabaseError::QueryFailed(e)
            })?;

        #[derive(serde::Deserialize, Debug)]
        struct GroupResult {
            agents: Vec<RecordId>,
        }

        let result: Vec<GroupResult> = agents_result.take(0).map_err(DatabaseError::from)?;
        let agents = result
            .into_iter()
            .next()
            .map(|r| r.agents)
            .unwrap_or_default();

        if agents.is_empty() {
            return Ok(Vec::new());
        }

        tracing::debug!(
            "Executing group archival search with query='{}' for {} agents",
            query,
            agents.len()
        );

        // Search all archival memories then filter by owner
        let sql = r#"
            SELECT *, value, search::score(1) AS relevance_score FROM mem
            WHERE  value @1@ $search_term
            ORDER BY relevance_score DESC
            LIMIT $limit_multiplier
        "#;

        let mut result = db
            .query(sql)
            .bind(("search_term", query.to_string()))
            .bind(("limit_multiplier", limit * 10)) // Get more since we'll filter
            .await
            .map_err(|e| {
                crate::log_error!("Group archival search query failed", e);
                crate::db::DatabaseError::QueryFailed(e)
            })?;

        tracing::debug!("search results: {:#?}", result);

        // Extract results with scores - use same struct as single-agent search
        #[derive(serde::Deserialize)]
        struct ScoredMemResult {
            id: surrealdb::RecordId,
            owner_id: surrealdb::RecordId,
            label: String,
            value: String,
            description: Option<String>,
            memory_type: crate::memory::MemoryType,
            permission: crate::memory::MemoryPermission,
            metadata: serde_json::Value,
            embedding_model: Option<String>,
            embedding: Option<Vec<f32>>,
            created_at: surrealdb::Datetime,
            updated_at: surrealdb::Datetime,
            is_active: bool,
            pinned: bool,
            relevance_score: Option<f32>,
        }

        let mut scored_results: Vec<ScoredMemResult> =
            result.take(0).map_err(DatabaseError::from)?;

        // Extract the "out" field from each result object
        #[derive(serde::Deserialize)]
        struct OutRecord {
            out: RecordId,
        }

        let mut agent_mem_id_set: std::collections::HashSet<RecordId> =
            std::collections::HashSet::new();

        for agent_record in agents {
            let agent_id = crate::AgentId::from_record(agent_record);
            let agent_mem_sql = format!(
                "SELECT out FROM agent_memories WHERE in = {} AND out IS NOT NULL",
                agent_id.to_string()
            );

            let mut agent_mem_result = db
                .query(&agent_mem_sql)
                .await
                .map_err(DatabaseError::from)?;

            let out_records: Vec<OutRecord> =
                agent_mem_result.take(0).map_err(DatabaseError::from)?;

            for record in out_records.into_iter() {
                agent_mem_id_set.insert(record.out.clone());
            }
        }

        // Filter messages to only those belonging to this agent
        scored_results.retain(|sm| agent_mem_id_set.contains(&sm.id));

        // Filter by group members and convert to domain objects
        let mut all_scored_blocks: Vec<ScoredMemoryBlock> = scored_results
            .into_iter()
            .map(|sr| {
                // Reconstruct the DbModel from our query results
                use crate::memory::MemoryBlockDbModel;
                let db_model = MemoryBlockDbModel {
                    id: sr.id,
                    owner_id: sr.owner_id,
                    label: sr.label,
                    value: sr.value,
                    description: sr.description,
                    memory_type: sr.memory_type,
                    permission: sr.permission,
                    metadata: sr.metadata,
                    embedding_model: sr.embedding_model,
                    embedding: sr.embedding,
                    created_at: sr.created_at,
                    updated_at: sr.updated_at,
                    is_active: sr.is_active,
                    pinned: sr.pinned,
                };

                let block = MemoryBlock::from_db_model(db_model)
                    .expect("memory should convert from db model");

                ScoredMemoryBlock {
                    block,
                    score: sr.relevance_score.unwrap_or(1.0),
                }
            })
            .collect();

        // Sort all results by relevance score and limit
        all_scored_blocks.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        all_scored_blocks.truncate(limit);

        let scored_blocks = all_scored_blocks;

        Ok(scored_blocks)
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
        // Check if block already exists to avoid unnecessary work
        if let Some(existing_block) = self.memory.get_block(label) {
            // If it's already a working memory block or core memory, just return it
            if existing_block.memory_type == MemoryType::Working
                || existing_block.memory_type == MemoryType::Core
            {
                return Ok(existing_block.clone());
            }
        }

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
        if let Some(mem) = self.memory.get_block(label).map(|b| b.clone()) {
            Ok(Some(mem))
        } else {
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
        let scored = self
            .search_conversations_with_options(
                query,
                role_filter,
                start_time,
                end_time,
                limit,
                None, // Use default (exact) search
            )
            .await?;
        Ok(scored.into_iter().map(|sm| sm.message).collect())
    }

    /// Execute a content search query against the msg table
    async fn execute_content_search(
        &self,
        search_query: &str,
        role_filter: Option<crate::message::ChatRole>,
        start_time: Option<DateTime<Utc>>,
        end_time: Option<DateTime<Utc>>,
        result_multiplier: usize,
    ) -> Result<Vec<ScoredMessage>> {
        let db = self.db.as_ref().ok_or_else(|| {
            crate::db::DatabaseError::QueryFailed(surrealdb::Error::Api(
                surrealdb::error::Api::InvalidParams(
                    "No database connection available for content search".into(),
                ),
            ))
        })?;
        // Build conditions for msg table query
        let mut conditions = vec!["content @1@ $search_query".to_string()];

        if role_filter.is_some() {
            conditions.push("role = $role".to_string());
        }
        if start_time.is_some() {
            conditions.push("created_at >= $start_time".to_string());
        }
        if end_time.is_some() {
            conditions.push("created_at <= $end_time".to_string());
        }

        // Query msg table directly with content search and BM25 scoring
        let sql = format!(
            "SELECT *, content, search::score(1) AS relevance_score FROM msg WHERE {} ORDER BY relevance_score DESC, created_at DESC LIMIT {}",
            conditions.join(" AND "),
            result_multiplier
        );

        // Build query and bind parameters
        let mut query_builder = db
            .query(&sql)
            .bind(("search_query", search_query.to_string()));

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

        // Execute query
        let mut result = query_builder.await.map_err(DatabaseError::from)?;

        // Extract messages with scores
        // Define a struct that matches the exact query result structure from SurrealDB
        // with the relevance_score added from our SELECT
        #[derive(serde::Deserialize)]
        struct ScoredMsgResult {
            // Fields from MessageDbModel
            id: surrealdb::RecordId,
            role: crate::message::ChatRole,
            owner_id: Option<crate::id::UserId>,
            content: serde_json::Value,  // DB stores as JSON value
            metadata: serde_json::Value, // DB stores as JSON value
            options: serde_json::Value,  // DB stores as JSON value
            has_tool_calls: bool,
            word_count: u32,
            created_at: surrealdb::Datetime,
            position: Option<String>, // DB stores snowflake as string
            batch: Option<String>,    // DB stores snowflake as string
            sequence_num: Option<u32>,
            batch_type: Option<crate::message::BatchType>,
            embedding: Option<Vec<f32>>,
            embedding_model: Option<String>,
            // Added by our query
            relevance_score: Option<f32>,
        }

        let scored_results: Vec<ScoredMsgResult> = result.take(0).map_err(DatabaseError::from)?;

        let scored_msgs: Vec<ScoredMessage> = scored_results
            .into_iter()
            .map(|sr| {
                // Convert from DB model format to domain Message
                // We need to reconstruct the MessageDbModel from our query results
                use crate::message::MessageDbModel;

                let db_model = MessageDbModel {
                    id: sr.id,
                    role: sr.role,
                    owner_id: sr.owner_id,
                    content: sr.content,
                    metadata: sr.metadata,
                    options: sr.options,
                    has_tool_calls: sr.has_tool_calls,
                    word_count: sr.word_count,
                    created_at: sr.created_at,
                    position: sr.position,
                    batch: sr.batch,
                    sequence_num: sr.sequence_num,
                    batch_type: sr.batch_type,
                    embedding: sr.embedding,
                    embedding_model: sr.embedding_model,
                };

                let msg =
                    Message::from_db_model(db_model).expect("message should convert from db model");

                ScoredMessage {
                    message: msg,
                    score: sr.relevance_score.unwrap_or(1.0),
                }
            })
            .collect();

        Ok(scored_msgs)
    }

    /// Parse time filter from query string (e.g., "search term > 5 days")
    fn parse_time_filter(query: &str) -> (String, Option<DateTime<Utc>>, Option<DateTime<Utc>>) {
        use chrono::Duration;
        use regex::Regex;

        // Check if the potential time expression is quoted - if so, it's part of the search query
        // This allows searching for literal time expressions like: search for "5 days"
        let quote_check = Regex::new(r#"["']\s*(?:[<>]\s*)?\d+(?:\s*-\s*\d+)?\s*(?:days?|hours?|weeks?|months?)\s*(?:old|ago)?\s*["']$"#).unwrap();
        if quote_check.is_match(query) {
            // Time expression is quoted, don't parse it
            return (query.to_string(), None, None);
        }

        // Match patterns like "> 5 days", "< 2 hours", "3-7 days"
        // Regex breakdown:
        // (?i)                     - case insensitive
        // \s*                      - optional whitespace
        // (?:([<>])\s*)?           - optional: capture group 1 for < or > with optional space after
        // (\d+)                    - capture group 2: first number (required)
        // (?:\s*-\s*(\d+))?        - optional: dash and capture group 3 for second number (ranges)
        // \s*                      - optional whitespace
        // (days?|hours?|weeks?|months?) - capture group 4: time unit (? makes 's' optional for singular/plural)
        // \s*                      - optional whitespace
        // (?:old|ago)?             - optional: "old" or "ago" (not captured, just matched)
        // $                        - must be at end of string
        let time_pattern = Regex::new(r"(?i)\s*(?:([<>])\s*)?(\d+)(?:\s*-\s*(\d+))?\s*(days?|hours?|weeks?|months?)\s*(?:old|ago)?$").unwrap();

        if let Some(captures) = time_pattern.captures(query) {
            let cleaned_query = query[..captures.get(0).unwrap().start()].trim().to_string();
            let now = Utc::now();

            let operator = captures.get(1).map(|m| m.as_str());
            let num1: i64 = captures
                .get(2)
                .and_then(|m| m.as_str().parse().ok())
                .unwrap_or(0);
            let num2: Option<i64> = captures.get(3).and_then(|m| m.as_str().parse().ok());
            let unit = captures
                .get(4)
                .map(|m| m.as_str().to_lowercase())
                .unwrap_or_default();

            // Convert to duration
            let duration1 = match unit.as_str() {
                s if s.starts_with("hour") => Duration::hours(num1),
                s if s.starts_with("day") => Duration::days(num1),
                s if s.starts_with("week") => Duration::weeks(num1),
                s if s.starts_with("month") => Duration::days(num1 * 30), // Approximate
                _ => Duration::days(num1),
            };

            let (start_time, end_time) = match (operator, num2) {
                (Some(">"), None) => {
                    // Older than X time
                    (None, Some(now - duration1))
                }
                (Some("<"), None) => {
                    // Within last X time
                    (Some(now - duration1), None)
                }
                (None, Some(n2)) => {
                    // Range: X-Y time ago
                    let duration2 = match unit.as_str() {
                        s if s.starts_with("hour") => Duration::hours(n2),
                        s if s.starts_with("day") => Duration::days(n2),
                        s if s.starts_with("week") => Duration::weeks(n2),
                        s if s.starts_with("month") => Duration::days(n2 * 30),
                        _ => Duration::days(n2),
                    };
                    (Some(now - duration2), Some(now - duration1))
                }
                _ => {
                    // Default: exact time ago (like "5 days old" means around 5 days ago)
                    // We'll use a 1-day window around that time
                    let window = Duration::days(1);
                    (
                        Some(now - duration1 - window),
                        Some(now - duration1 + window),
                    )
                }
            };

            (cleaned_query, start_time, end_time)
        } else {
            (query.to_string(), None, None)
        }
    }

    /// Search conversation messages with advanced options including fuzzy search and scoring
    pub async fn search_conversations_with_options(
        &self,
        query: Option<&str>,
        role_filter: Option<crate::message::ChatRole>,
        mut start_time: Option<DateTime<Utc>>,
        mut end_time: Option<DateTime<Utc>>,
        limit: usize,
        _fuzzy_level: Option<u8>, // 0=exact, 1=fuzzy, 2=very fuzzy
    ) -> Result<Vec<ScoredMessage>> {
        let db = self.db.as_ref().ok_or_else(|| {
            crate::db::DatabaseError::QueryFailed(surrealdb::Error::Api(
                surrealdb::error::Api::InvalidParams(
                    "No database connection available for conversation search".into(),
                ),
            ))
        })?;

        // Parse time filter from query if present
        let cleaned_query = if let Some(q) = query {
            let (cleaned, parsed_start, parsed_end) = Self::parse_time_filter(q);

            // Use parsed times if not explicitly provided
            if start_time.is_none() && parsed_start.is_some() {
                start_time = parsed_start;
                tracing::debug!("Parsed start_time from query: {:?}", start_time);
            }
            if end_time.is_none() && parsed_end.is_some() {
                end_time = parsed_end;
                tracing::debug!("Parsed end_time from query: {:?}", end_time);
            }

            if cleaned.is_empty() {
                None
            } else {
                Some(cleaned)
            }
        } else {
            None
        };

        // Use different search paths based on whether we have a content query
        if let Some(search_query) = &cleaned_query {
            // Use helper function for content search
            let mut scored_msgs = self
                .execute_content_search(
                    search_query,
                    role_filter.clone(),
                    start_time,
                    end_time,
                    limit * 10, // Get more results since we'll filter some out
                )
                .await?;

            // Filter by agent if needed
            // Get all message IDs belonging to this agent
            let agent_msg_sql = format!(
                "SELECT message_type, out FROM agent_messages WHERE in = {} AND message_type = 'archived' AND out IS NOT NULL ",
                self.agent_id.to_string()
            );

            let mut agent_msg_result = db
                .query(&agent_msg_sql)
                .await
                .map_err(DatabaseError::from)?;

            // Extract the "out" field from each result object
            #[derive(serde::Deserialize)]
            struct OutRecord {
                out: RecordId,
                message_type: MessageRelationType,
            }

            let out_records: Vec<OutRecord> =
                agent_msg_result.take(0).map_err(DatabaseError::from)?;

            let agent_msg_ids: Vec<RecordId> = out_records
                .into_iter()
                .filter(|r| r.message_type != MessageRelationType::Active)
                .map(|r| r.out)
                .collect();

            let agent_msg_id_set: std::collections::HashSet<RecordId> =
                agent_msg_ids.into_iter().collect();

            // Filter messages to only those belonging to this agent
            scored_msgs.retain(|sm| {
                let msg_record_id = RecordId::from((MessageId::PREFIX, sm.message.id.to_key()));
                agent_msg_id_set.contains(&msg_record_id)
            });

            // Truncate to limit
            scored_msgs.truncate(limit);

            // If no results, try fallback search with individual words
            if scored_msgs.is_empty() && search_query.contains(' ') {
                tracing::debug!("No results for '{}', trying fallback search", search_query);

                // Split into words and try each (longest first)
                let mut words: Vec<&str> = search_query.split_whitespace().collect();
                words.sort_by_key(|w| std::cmp::Reverse(w.len()));

                let mut fallback_results = Vec::new();
                let mut seen_ids = std::collections::HashSet::new();

                for word in words {
                    if word.len() < 3 {
                        // Skip very short words
                        continue;
                    }

                    tracing::debug!("Trying fallback search for word: '{}'", word);

                    // Search with this word
                    let word_results = self
                        .execute_content_search(
                            word,
                            role_filter.clone(),
                            start_time,
                            end_time,
                            limit * 5, // Get fewer per word
                        )
                        .await?;

                    // Add results with reduced scores (50% penalty for fallback)
                    for mut result in word_results {
                        if !seen_ids.contains(&result.message.id) {
                            seen_ids.insert(result.message.id.clone());
                            result.score *= 0.5; // Reduce score for fallback results
                            fallback_results.push(result);
                        }
                    }

                    // Stop if we have enough
                    if fallback_results.len() >= limit * 2 {
                        break;
                    }
                }

                if !fallback_results.is_empty() {
                    tracing::info!(
                        "Fallback search found {} results for original query '{}'",
                        fallback_results.len(),
                        search_query
                    );

                    // Sort by score and filter by agent
                    fallback_results.sort_by(|a, b| {
                        b.score
                            .partial_cmp(&a.score)
                            .unwrap_or(std::cmp::Ordering::Equal)
                    });

                    // Filter by agent
                    fallback_results.retain(|sm| {
                        let msg_record_id =
                            RecordId::from((MessageId::PREFIX, sm.message.id.to_key()));
                        agent_msg_id_set.contains(&msg_record_id)
                    });

                    fallback_results.truncate(limit);
                    return Ok(fallback_results);
                }
            }

            return Ok(scored_msgs);
        }

        // No content search - use graph traversal approach
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
            "SELECT position, batch, sequence_num, message_type, ->(msg{}) AS messages FROM agent_messages WHERE (in = agent:{} AND message_type = 'archived' AND out IS NOT NULL) ORDER BY batch NUMERIC DESC, sequence_num NUMERIC DESC, position NUMERIC DESC LIMIT $limit FETCH messages",
            where_clause,
            self.agent_id.to_key()
        );

        // Build query and bind all parameters
        let mut query_builder = db.query(&sql).bind(("limit", limit));

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

        // Extract messages - graph traversal query has messages nested under "messages" field
        let db_messages: Vec<Vec<<Message as DbEntity>::DbModel>> =
            result.take("messages").map_err(DatabaseError::from)?;

        let scored_messages: Vec<ScoredMessage> = db_messages
            .into_iter()
            .flatten()
            .map(|m| {
                let msg = Message::from_db_model(m).expect("message should convert from db model");
                ScoredMessage {
                    message: msg,
                    score: 1.0, // No score for graph traversal queries
                }
            })
            .collect();

        // Apply limit in application code since we may get more than limit from the query
        let mut limited_messages = scored_messages;
        limited_messages.truncate(limit);

        Ok(limited_messages)
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
        let scored = self
            .search_constellation_messages_with_options(
                query,
                role_filter,
                start_time,
                end_time,
                limit,
                None,
            )
            .await?;
        Ok(scored
            .into_iter()
            .map(|scm| (scm.agent_name, scm.message))
            .collect())
    }

    /// Search constellation messages with fuzzy search options (returns scored results)
    pub async fn search_constellation_messages_with_options(
        &self,
        query: Option<&str>,
        role_filter: Option<crate::message::ChatRole>,
        start_time: Option<DateTime<Utc>>,
        end_time: Option<DateTime<Utc>>,
        limit: usize,
        _fuzzy_level: Option<u8>,
    ) -> Result<Vec<ScoredConstellationMessage>> {
        let db = self.db.as_ref().ok_or_else(|| {
            crate::db::DatabaseError::QueryFailed(surrealdb::Error::Api(
                surrealdb::error::Api::InvalidParams(
                    "No database connection available for constellation message search".into(),
                ),
            ))
        })?;

        let mut agents_result = db
            .query("SELECT ->group_members->group<-group_members<-agent.{id, name} AS agents FROM $agent_id")
            .bind(("agent_id", RecordId::from(&self.agent_id)))
            .await
            .map_err(|e| {
                crate::log_error!("Failed to get group members", e);
                crate::db::DatabaseError::QueryFailed(e)
            })?;

        #[derive(serde::Deserialize)]
        struct AgentResult {
            id: RecordId,
            name: String,
        }

        #[derive(serde::Deserialize)]
        struct GroupResult {
            agents: Vec<AgentResult>,
        }

        let result: Vec<GroupResult> = agents_result.take(0).map_err(DatabaseError::from)?;
        let agents = result
            .into_iter()
            .next()
            .map(|r| r.agents)
            .unwrap_or_default();

        if agents.is_empty() {
            return Ok(Vec::new());
        }

        // Build agent_names map from the results
        let mut agent_names = std::collections::HashMap::new();
        for agent in &agents {
            let agent_id = crate::AgentId::from_record(agent.id.clone());
            agent_names.insert(agent_id.to_key(), agent.name.clone());
        }

        // Similar to conversation search, we need different approaches for content vs non-content search
        if let Some(search_query) = query {
            // Use helper function for content search
            let scored_msgs = self
                .execute_content_search(
                    search_query,
                    role_filter,
                    start_time,
                    end_time,
                    limit * 20, // Get more since we'll filter by constellation agents
                )
                .await?;

            // Now filter by constellation agents and pair with agent names
            // Build message-to-agent mapping by querying each agent's messages
            let mut msg_agent_map = std::collections::HashMap::new();

            for agent in &agents {
                let agent_id = crate::AgentId::from_record(agent.id.clone());
                let agent_msg_sql = format!(
                    "SELECT out FROM agent_messages WHERE in = {} AND out IS NOT NULL AND message_type = 'archived'",
                    agent_id.to_string()
                );

                let mut agent_result = db
                    .query(&agent_msg_sql)
                    .await
                    .map_err(DatabaseError::from)?;

                #[derive(serde::Deserialize)]
                struct OutRecord {
                    out: RecordId,
                }

                let out_records: Vec<OutRecord> =
                    agent_result.take(0).map_err(DatabaseError::from)?;

                for entry in out_records {
                    msg_agent_map.insert(entry.out, agent.id.clone());
                }
            }

            // Filter and pair messages with agent names and scores
            let mut scored_constellation_messages = Vec::new();
            for scored_msg in scored_msgs {
                let msg_record_id = surrealdb::RecordId::from((
                    crate::id::MessageId::PREFIX,
                    scored_msg.message.id.to_key(),
                ));

                // Check if this message belongs to a constellation agent
                if let Some(agent_record_id) = msg_agent_map.get(&msg_record_id) {
                    // Look up agent name using the agent ID
                    let agent_id = crate::AgentId::from_record(agent_record_id.clone());

                    let agent_name = agent_names
                        .get(&agent_id.to_key())
                        .cloned()
                        .unwrap_or_else(|| format!("Unknown ({})", agent_id));

                    scored_constellation_messages.push(ScoredConstellationMessage {
                        agent_name,
                        message: scored_msg.message,
                        score: scored_msg.score,
                    });

                    // Stop if we have enough results
                    if scored_constellation_messages.len() >= limit * 10 {
                        break;
                    }
                }
            }

            scored_constellation_messages.truncate(limit);

            return Ok(scored_constellation_messages);
        }

        // No content search - use graph traversal approach
        let mut traversal_conditions = vec![];
        if role_filter.is_some() {
            traversal_conditions.push("role = $role");
        }
        if start_time.is_some() {
            traversal_conditions.push("created_at >= $start_time");
        }
        if end_time.is_some() {
            traversal_conditions.push("created_at <= $end_time");
        }

        let where_clause = if traversal_conditions.is_empty() {
            String::new()
        } else {
            format!(" WHERE {}", traversal_conditions.join(" AND "))
        };

        // Get messages from all group agents
        let mut all_messages = Vec::new();

        for agent in &agents {
            let agent_id = crate::AgentId::from_record(agent.id.clone());
            let agent_sql = format!(
                r#"SELECT out AS message FROM agent_messages
                WHERE in = {} AND out IS NOT NULL {}
                ORDER BY out.created_at DESC
                LIMIT $agent_limit
                FETCH message"#,
                agent_id.to_string(),
                if where_clause.is_empty() {
                    String::new()
                } else {
                    format!("AND out.({})", where_clause.trim_start_matches(" WHERE "))
                }
            );

            // Build query and bind all parameters
            let mut query_builder = db
                .query(&agent_sql)
                .bind(("agent_limit", limit / agents.len().max(1)));

            if let Some(role) = &role_filter {
                query_builder = query_builder.bind(("role", role.to_string()));
            }

            if let Some(start) = start_time {
                query_builder =
                    query_builder.bind(("start_time", surrealdb::sql::Datetime::from(start)));
            }

            if let Some(end) = end_time {
                query_builder =
                    query_builder.bind(("end_time", surrealdb::sql::Datetime::from(end)));
            }

            // Execute query
            let mut result = query_builder.await.map_err(DatabaseError::from)?;

            // Extract messages for this agent
            let db_messages: Vec<<Message as DbEntity>::DbModel> =
                result.take("message").map_err(DatabaseError::from)?;

            // Add messages with agent name
            for db_msg in db_messages {
                if let Ok(msg) = Message::from_db_model(db_msg) {
                    let agent_name = agent.name.clone();

                    all_messages.push(ScoredConstellationMessage {
                        agent_name,
                        message: msg,
                        score: 1.0, // No score for graph traversal queries
                    });

                    if all_messages.len() >= limit * 5 {
                        break;
                    }
                }
            }

            if all_messages.len() >= limit * 10 {
                break;
            }
        }

        // Sort by created_at and truncate to limit
        all_messages.sort_by(|a, b| b.message.created_at.cmp(&a.message.created_at));
        all_messages.truncate(limit);

        Ok(all_messages)
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
                    total_tokens > (max_tokens * 2 / 3)
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
                            is_error: Some(true),
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
                        is_error: Some(true),
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

        // Build execution metadata from the call
        let mut params = call.fn_arguments.clone();

        // Check for a simple consent rule in workflow rules: rule == "requires_consent"
        let requires_consent = self
            .context_config
            .tool_workflow_rules
            .iter()
            .any(|r| r.tool_name == call.fn_name && r.rule == "requires_consent");
        let request_heartbeat = params
            .get("request_heartbeat")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        // Strip request_heartbeat before tool deserialization
        if let serde_json::Value::Object(ref mut map) = params {
            map.remove("request_heartbeat");
        }
        // Obtain permission grant if required (simple ToolExecution scope)
        let permission_grant = if requires_consent {
            let scope = crate::permission::PermissionScope::ToolExecution {
                tool: call.fn_name.clone(),
                args_digest: None,
            };
            crate::permission::broker()
                .request(
                    self.handle.agent_id.clone(),
                    call.fn_name.clone(),
                    scope,
                    Some("Tool requires consent".to_string()),
                    std::time::Duration::from_secs(90),
                )
                .await
        } else {
            None
        };

        if requires_consent && permission_grant.is_none() {
            return Err(crate::CoreError::ToolExecutionFailed {
                tool_name: call.fn_name.clone(),
                cause: "Permission denied or timed out".to_string(),
                parameters: params.clone(),
            });
        }

        let meta = crate::tool::ExecutionMeta {
            permission_grant,
            request_heartbeat,
            caller_user: None,
            call_id: Some(crate::id::ToolCallId(call.call_id.clone())),
        };

        match self.tools.execute(&call.fn_name, params, &meta).await {
            Ok(tool_response) => {
                tracing::debug!("✅ Tool {} executed successfully", call.fn_name);

                let response_json = serde_json::to_string_pretty(&tool_response)
                    .unwrap_or_else(|_| "Error serializing response".to_string());

                Ok(Some(ToolResponse {
                    call_id: call.call_id.clone(),
                    content: response_json,
                    is_error: None,
                }))
            }
            Err(e) => {
                tracing::warn!("❌ Tool {} failed: {}", call.fn_name, e);

                Ok(Some(ToolResponse {
                    call_id: call.call_id.clone(),
                    content: format!("Error: {:?}", e),
                    is_error: Some(true),
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
        if let Some(new_summary) = result.summary {
            // Align with normal compression path: append with a triple-newline delimiter to preserve history
            if let Some(existing_summary) = &mut history.archive_summary {
                *existing_summary = format!("{}\n\n\n{}", existing_summary, new_summary);
            } else {
                history.archive_summary = Some(new_summary);
            }
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
                *existing_summary = format!("{}\n\n\n{}", existing_summary, new_summary);
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
