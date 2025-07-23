//! Agent state management for building stateful agents on stateless protocols
//!
//! This module provides the infrastructure for managing agent state between
//! conversations, including message history, memory persistence, and context rebuilding.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

use std::sync::Arc;

use crate::{
    AgentId, AgentState, AgentType, CoreError, Result,
    db::{DatabaseError, DbEntity},
    memory::{Memory, MemoryBlock, MemoryPermission, MemoryType},
    message::{Message, MessageContent, Response, ToolResponse},
    tool::ToolRegistry,
};

use super::{
    CompressionResult, CompressionStrategy, ContextBuilder, ContextConfig, MemoryContext,
    MessageCompressor,
};

/// Cheap handle to agent internals that built-in tools can hold
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentHandle<C: surrealdb::Connection + Clone = surrealdb::engine::any::Any> {
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

    /// Private database connection for controlled access
    #[serde(skip)]
    db: Option<surrealdb::Surreal<C>>,
    // TODO: Add message_sender when we implement it
}

impl<C: surrealdb::Connection + Clone> AgentHandle<C> {
    /// Create a new handle with a database connection
    pub fn with_db(mut self, db: surrealdb::Surreal<C>) -> Self {
        self.db = Some(db);
        self
    }

    /// Check if this handle has a database connection
    pub fn has_db_connection(&self) -> bool {
        self.db.is_some()
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
            self.agent_id.uuid()
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
                tracing::error!("Search query failed: {:?}", e);
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

        // Build the query dynamically using graph traversal
        let mut sql = format!(
            "SELECT * FROM message WHERE (<-agent_messages<-agent:⟨{}⟩..)",
            self.agent_id.uuid()
        );

        // Add optional filters
        if query.is_some() {
            sql.push_str(" AND content @@ $query");
        }

        if role_filter.is_some() {
            sql.push_str(" AND role = $role");
        }

        if start_time.is_some() {
            sql.push_str(" AND created_at >= $start_time");
        }

        if end_time.is_some() {
            sql.push_str(" AND created_at <= $end_time");
        }

        sql.push_str(" ORDER BY created_at DESC LIMIT $limit");

        // Build query and bind all parameters
        let mut query_builder = db.query(&sql).bind(("limit", limit));

        if let Some(search_query) = query {
            query_builder = query_builder.bind(("query", search_query.to_string()));
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

        let db_messages: Vec<<Message as DbEntity>::DbModel> = query_builder
            .await
            .map_err(DatabaseError::from)?
            .take(0)
            .map_err(DatabaseError::from)?;

        // Convert from DbModel to domain type
        let messages: Vec<Message> = db_messages
            .into_iter()
            .map(|m| Message::from_db_model(m).expect("message should convert from db model"))
            .collect();

        Ok(messages)
    }
}

impl Default for AgentHandle {
    fn default() -> Self {
        Self {
            name: "".to_string(),
            agent_id: AgentId::generate(),
            memory: Memory::new(),
            state: AgentState::Ready,
            agent_type: AgentType::Generic,
            db: None,
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
            db: None,
        }
    }
}

/// Message history that needs locking
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MessageHistory {
    /// Active messages in the current context window
    pub messages: Vec<Message>,
    /// Messages that have been compressed/archived to save context space
    pub archived_messages: Vec<Message>,
    /// Optional summary of archived messages
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message_summary: Option<String>,
    /// Strategy used for compressing messages when context is full
    pub compression_strategy: CompressionStrategy,
    /// When compression was last performed
    pub last_compression: DateTime<Utc>,
}

impl MessageHistory {
    /// Create a new message history with the specified compression strategy
    pub fn new(compression_strategy: CompressionStrategy) -> Self {
        Self {
            messages: Vec::new(),
            archived_messages: Vec::new(),
            message_summary: None,
            compression_strategy,
            last_compression: Utc::now(),
        }
    }
}

/// Represents the complete state of an agent
#[derive(Clone)]
pub struct AgentContext<C: surrealdb::Connection + Clone = surrealdb::engine::any::Any> {
    /// Cheap, frequently accessed stuff
    pub handle: AgentHandle<C>,

    /// Tools available to this agent
    pub tools: ToolRegistry,

    /// Configuration for context building
    pub context_config: ContextConfig,

    /// Metadata about the agent state
    pub metadata: Arc<RwLock<AgentContextMetadata>>,

    /// The big stuff in its own lock
    pub history: Arc<RwLock<MessageHistory>>,
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

impl<C: surrealdb::Connection + Clone> AgentContext<C> {
    /// Create a new agent state
    pub fn new(
        agent_id: AgentId,
        name: String,
        agent_type: AgentType,
        memory: Memory,
        tools: ToolRegistry,
        context_config: ContextConfig,
    ) -> Self {
        let handle = AgentHandle {
            agent_id,
            memory,
            name,
            agent_type,
            state: AgentState::Ready,
            db: None,
        };

        Self {
            handle,
            tools,
            context_config,
            metadata: Arc::new(RwLock::new(AgentContextMetadata {
                created_at: Utc::now(),
                last_active: Utc::now(),
                total_messages: 0,
                total_tool_calls: 0,
                context_rebuilds: 0,
                compression_events: 0,
            })),
            history: Arc::new(RwLock::new(MessageHistory::new(
                CompressionStrategy::default(),
            ))),
        }
    }

    /// Get a cheap handle to agent internals
    pub fn handle(&self) -> AgentHandle<C> {
        self.handle.clone()
    }

    /// Build the current context for this agent
    pub async fn build_context(&self) -> Result<MemoryContext> {
        {
            let mut metadata = self.metadata.write().await;
            metadata.last_active = Utc::now();
            metadata.context_rebuilds += 1;
        }

        // Check if we need to compress messages
        {
            let history = self.history.read().await;
            if history.messages.len() > self.context_config.max_context_messages {
                drop(history); // release read lock
                self.compress_messages().await?;
            }
        }

        // Get memory blocks
        let memory_blocks = self.handle.memory.get_all_blocks();

        // Build context with read lock
        let history = self.history.read().await;
        tracing::debug!(
            "Building context for agent {}: {} messages in history, max_context_messages={}",
            self.handle.agent_id,
            history.messages.len(),
            self.context_config.max_context_messages
        );

        let context =
            ContextBuilder::new(self.handle.agent_id.clone(), self.context_config.clone())
                .with_memory_blocks(memory_blocks)
                .with_tools_from_registry(&self.tools)
                .with_messages(history.messages.clone())
                .build()?;

        tracing::debug!(
            "Built context with {} messages, system_prompt length={} chars",
            context.messages.len(),
            context.system_prompt.len()
        );

        Ok(context)
    }

    /// Add a new message to the state
    pub async fn add_message(&self, message: Message) {
        // Count tool calls if this is an assistant message
        if message.role.is_assistant() {
            let mut metadata = self.metadata.write().await;
            metadata.total_tool_calls += message.tool_call_count();
        }

        let mut history = self.history.write().await;
        history.messages.push(message);
        {
            let mut metadata = self.metadata.write().await;
            metadata.total_messages += 1;
            metadata.last_active = Utc::now();
        }
    }

    /// Process a chat response and update state
    /// Returns a vec of responses: the original (minus tool calls) and a separate tool response if needed
    pub async fn process_response(&self, response: Response) -> Result<Vec<Response>> {
        let mut tool_responses = Vec::with_capacity(response.tool_calls.len());
        let mut first_error = None;
        let mut memory_updates = Vec::new();

        // Execute all tools, collecting responses or errors
        for call in &response.tool_calls {
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

                    // Track memory tool calls for persistence
                    if call.fn_name.contains("memory") {
                        memory_updates.push((call.fn_name.clone(), call.fn_arguments.clone()));
                    }

                    tool_responses.push(ToolResponse {
                        call_id: call.call_id.clone(),
                        content: serde_json::to_string_pretty(&tool_response)
                            .unwrap_or("Error serializing tool response".to_string()),
                    });
                }
                Err(e) => {
                    tracing::error!("❌ Tool execution failed for {}: {}", call.fn_name, e);
                    // Store first error but continue executing other tools
                    if first_error.is_none() {
                        first_error = Some(CoreError::tool_execution_error(
                            call.fn_name.clone(),
                            format!("{}", e),
                        ));
                    }
                    // Add error response for this tool
                    tool_responses.push(ToolResponse {
                        call_id: call.call_id.clone(),
                        content: format!("Error: Failed to execute tool '{}': {}", call.fn_name, e),
                    });
                }
            }
        }

        // If any tool failed, return the first error
        if let Some(error) = first_error {
            return Err(error);
        }

        let mut results = Vec::new();

        // Add the original response if it has content (text/parts)
        // Don't include tool calls in this response
        if !response.content.is_empty() {
            results.push(Response {
                content: response.content,
                reasoning: response.reasoning,
                tool_calls: vec![], // Tool calls handled separately
                metadata: response.metadata.clone(),
            });
        }

        // Create a separate response for tool responses with Tool role
        if !tool_responses.is_empty() {
            results.push(Response {
                content: vec![MessageContent::ToolResponses(tool_responses)],
                reasoning: None,
                tool_calls: vec![],
                metadata: response.metadata,
            });
        }

        tracing::debug!(
            "✅ process_response complete: returning {} responses",
            results.len()
        );

        Ok(results)
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

    /// Compress messages using the configured strategy
    async fn compress_messages(&self) -> Result<()> {
        let mut history = self.history.write().await;
        let compressor = MessageCompressor::new(history.compression_strategy.clone());

        let result = compressor
            .compress(
                history.messages.clone(),
                self.context_config.max_context_messages,
            )
            .await?;

        let archived_ids = self.apply_compression_result(&mut history, result)?;
        history.last_compression = Utc::now();
        self.metadata.write().await.compression_events += 1;

        // Return the archived IDs so the caller can persist them to the database
        // For now, just log that compression happened
        if !archived_ids.is_empty() {
            tracing::info!(
                "Compressed {} messages for agent {}, ready to archive in database",
                archived_ids.len(),
                self.handle.agent_id
            );
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
            .archived_messages
            .iter()
            .map(|msg| msg.id.clone())
            .collect();

        // Move compressed messages to archive
        history.archived_messages.extend(result.archived_messages);

        // Update active messages
        history.messages = result.active_messages;

        // Update or append to summary
        if let Some(new_summary) = result.summary {
            if let Some(existing_summary) = &mut history.message_summary {
                *existing_summary = format!("{}\n\n{}", existing_summary, new_summary);
            } else {
                history.message_summary = Some(new_summary);
            }
        }

        // Return the archived message IDs for the caller to persist
        Ok(archived_ids)
    }

    /// Search through message history (including archived)
    pub async fn search_messages(
        &self,
        query: &str,
        page: usize,
        page_size: usize,
    ) -> Vec<Message> {
        let query_lower = query.to_lowercase();

        let history = self.history.read().await;

        // Search through all messages (active + archived)
        let all_messages: Vec<&Message> = history
            .archived_messages
            .iter()
            .chain(history.messages.iter())
            .collect();

        // Filter messages containing the query
        let matching_messages: Vec<&Message> = all_messages
            .into_iter()
            .filter(|msg| {
                msg.text_content()
                    .map(|text| text.to_lowercase().contains(&query_lower))
                    .unwrap_or(false)
            })
            .collect();

        // Paginate results
        let start = page * page_size;
        let end = (start + page_size).min(matching_messages.len());

        if start < matching_messages.len() {
            matching_messages[start..end]
                .iter()
                .map(|m| (*m).clone())
                .collect()
        } else {
            Vec::new()
        }
    }

    /// Get agent statistics
    pub async fn get_stats(&self) -> AgentStats {
        let history = self.history.read().await;
        let metadata = self.metadata.read().await;
        AgentStats {
            total_messages: metadata.total_messages,
            active_messages: history.messages.len(),
            archived_messages: history.archived_messages.len(),
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
            messages: history.messages.clone(),
            memory_snapshot: self.handle.memory.clone(),
            metadata: self.metadata.read().await.clone(),
        }
    }

    /// Restore from a checkpoint
    pub async fn restore_from_checkpoint(&self, checkpoint: StateCheckpoint) -> Result<()> {
        if checkpoint.agent_id != self.handle.agent_id {
            return Err(CoreError::AgentInitFailed {
                agent_type: format!("{:?}", self.handle.agent_type),
                cause: Box::new(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "Checkpoint is for a different agent",
                )),
            });
        }

        let mut history = self.history.write().await;
        history.messages = checkpoint.messages;
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
    pub messages: Vec<Message>,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_agent_state_builder() {
        let id = AgentId::generate();
        let state = AgentContextBuilder::new(id.clone(), AgentType::Generic)
            .with_memory_block(
                "persona",
                "I am a helpful AI assistant",
                Some("Agent persona"),
            )
            .with_memory_block("human", "The user's name is unknown", None::<String>)
            .build()
            .await
            .unwrap();

        assert_eq!(state.handle.agent_id, id);
        assert_eq!(state.handle.memory.list_blocks().len(), 2);
        assert!(state.handle.memory.get_block("persona").is_some());
    }

    #[tokio::test]
    async fn test_message_search() {
        let state = AgentContextBuilder::new(AgentId::generate(), AgentType::Generic)
            .build()
            .await
            .unwrap();

        state
            .add_message(Message::user("Hello, how are you?"))
            .await;
        state
            .add_message(Message::agent("I'm doing well, thank you!"))
            .await;
        state
            .add_message(Message::user("What's the weather like?"))
            .await;

        let results = state.search_messages("weather", 0, 10).await;
        assert_eq!(results.len(), 1);
    }
}
