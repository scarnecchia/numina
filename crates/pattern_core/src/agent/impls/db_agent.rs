//! Database-backed agent implementation

use async_trait::async_trait;
use compact_str::CompactString;
use std::sync::Arc;
use surrealdb::Surreal;
use tokio::sync::RwLock;

use crate::tool::builtin::BuiltinTools;

use crate::{
    CoreError, MemoryBlock, ModelProvider, Result, UserId,
    agent::{Agent, AgentMemoryRelation, AgentState, AgentType, MemoryAccessLevel},
    context::{AgentContext, ContextConfig},
    db::{DatabaseError, ops, schema},
    embeddings::EmbeddingProvider,
    id::AgentId,
    memory::Memory,
    message::{Message, Request, Response},
    model::ResponseOptions,
    tool::{DynamicTool, ToolRegistry},
};

/// A concrete agent implementation backed by the database
pub struct DatabaseAgent<C, M, E>
where
    C: surrealdb::Connection,
{
    /// The user who owns this agent
    pub user_id: UserId,
    /// The agent's context (includes all state)
    context: AgentContext,

    /// model provider
    model: Arc<RwLock<M>>,

    /// model configuration
    chat_options: Arc<RwLock<Option<ResponseOptions>>>,
    /// Database connection
    db: Arc<Surreal<C>>,

    /// Embedding provider for semantic search
    embeddings: Option<Arc<E>>,
}

impl<C, M, E> DatabaseAgent<C, M, E>
where
    C: surrealdb::Connection,
    M: ModelProvider + 'static,
    E: EmbeddingProvider + 'static,
{
    /// Create a new database-backed agent
    /// Create a new database agent
    ///
    /// This creates a new agent instance with the provided configuration.
    /// Memory blocks and message history should be loaded separately if needed.
    pub fn new(
        agent_id: AgentId,
        user_id: UserId,
        agent_type: AgentType,
        name: String,
        system_prompt: String,
        memory: Memory,
        db: Arc<Surreal<C>>,
        model: Arc<RwLock<M>>,
        tools: ToolRegistry,
        embeddings: Option<Arc<E>>,
    ) -> Self {
        // Build AgentContext with tools
        let mut context = AgentContext::new(
            agent_id,
            name,
            agent_type,
            memory,
            tools,
            ContextConfig {
                base_instructions: system_prompt,
                ..Default::default()
            },
        );
        context.handle.state = AgentState::Ready;

        // Register built-in tools
        let builtin = BuiltinTools::default_for_agent(context.handle());
        builtin.register_all(&context.tools);

        Self {
            user_id,
            context,
            chat_options: Arc::new(RwLock::const_new(None)),
            db,
            embeddings,
            model,
        }
    }

    /// Create a DatabaseAgent from a persisted AgentRecord
    ///
    /// This reconstructs the runtime agent from its stored state.
    /// The model provider, tools, and embeddings are injected at runtime.
    pub async fn from_record(
        record: crate::agent::AgentRecord,
        db: Arc<Surreal<C>>,
        model: Arc<RwLock<M>>,
        tools: ToolRegistry,
        embeddings: Option<Arc<E>>,
    ) -> Result<Self> {
        // Create memory with the owner
        let memory = Memory::with_owner(record.owner_id);

        // Load memory blocks from the record's relations
        for (memory_block, _relation) in &record.memories {
            memory.create_block(memory_block.label.clone(), memory_block.value.clone())?;
            if let Some(mut block) = memory.get_block_mut(&memory_block.label) {
                block.id = memory_block.id;
                block.description = memory_block.description.clone();
                block.metadata = memory_block.metadata.clone();
                block.embedding_model = memory_block.embedding_model.clone();
                block.created_at = memory_block.created_at;
                block.updated_at = memory_block.updated_at;
                block.is_active = memory_block.is_active;
            }
        }

        // Build the context config from the record
        let context_config = record.to_context_config();

        // Create the agent with the loaded configuration
        let mut agent = Self::new(
            record.id,
            record.owner_id,
            record.agent_type.clone(),
            record.name.clone(),
            context_config.base_instructions.clone(),
            memory,
            db,
            model,
            tools,
            embeddings,
        );

        // Restore the agent state
        agent.context.handle.state = record.state;

        // Restore message history and compression state
        {
            let mut history = agent.context.history.write().await;
            history.compression_strategy = record.compression_strategy.clone();
            history.message_summary = record.message_summary.clone();

            // Load active messages from relations (already ordered by position)
            for (message, relation) in &record.messages {
                if relation.message_type == crate::message::MessageRelationType::Active {
                    history.messages.push(message.clone());
                }
            }
        }

        // Restore statistics
        {
            let mut metadata = agent.context.metadata.write().await;
            metadata.created_at = record.created_at;
            metadata.last_active = record.last_active;
            metadata.total_messages = record.total_messages;
            metadata.total_tool_calls = record.total_tool_calls;
            metadata.context_rebuilds = record.context_rebuilds;
            metadata.compression_events = record.compression_events;
        }

        Ok(agent)
    }

    /// Start background task to sync memory updates via live queries
    pub async fn start_memory_sync(self: Arc<Self>) -> Result<()> {
        let agent_id = self.context.handle.agent_id;
        let memory = self.context.handle.memory.clone();
        let db = Arc::clone(&self.db);

        // Spawn background task to handle updates
        tokio::spawn(async move {
            use futures::StreamExt;
            tracing::info!("Memory sync task started for agent {}", agent_id);

            // Subscribe to all memory changes for this agent
            let stream = match ops::subscribe_to_agent_memory_updates(&db, agent_id).await {
                Ok(s) => {
                    tracing::info!(
                        "Successfully subscribed to memory updates for agent {} - live query active",
                        agent_id
                    );
                    s
                }
                Err(e) => {
                    tracing::error!("Failed to subscribe to memory updates: {:?}", e);
                    return;
                }
            };

            futures::pin_mut!(stream);

            while let Some((action, memory_block)) = stream.next().await {
                tracing::info!(
                    "🔔 Agent {} received live query notification: {:?} for block '{}' with content: '{}'",
                    agent_id,
                    action,
                    memory_block.label,
                    memory_block.value
                );

                match action {
                    surrealdb::Action::Create => {
                        tracing::info!(
                            "Agent {} creating memory block '{}'",
                            agent_id,
                            memory_block.label
                        );
                        // Create new block
                        if let Err(e) =
                            memory.create_block(memory_block.label.clone(), &memory_block.value)
                        {
                            tracing::error!(
                                "Failed to create memory block {}: {}",
                                memory_block.label,
                                e
                            );
                        }
                        // Update other fields if present
                        if let Some(mut block) = memory.get_block_mut(&memory_block.label) {
                            block.id = memory_block.id;
                            // owner_id is now managed via RELATE, not stored on the entity
                            block.description = memory_block.description;
                            block.metadata = memory_block.metadata;
                            block.embedding_model = memory_block.embedding_model;
                            block.created_at = memory_block.created_at;
                            block.updated_at = memory_block.updated_at;
                            block.is_active = memory_block.is_active;
                        }
                    }

                    surrealdb::Action::Update => {
                        // Update or create the block
                        if memory.get_block(&memory_block.label).is_some() {
                            tracing::info!(
                                "✅ Agent {} updating existing memory '{}' with content: '{}'",
                                agent_id,
                                memory_block.label,
                                memory_block.value
                            );
                            // Update existing block
                            if let Err(e) =
                                memory.update_block_value(&memory_block.label, &memory_block.value)
                            {
                                tracing::error!(
                                    "Failed to update memory block {}: {}",
                                    memory_block.label,
                                    e
                                );
                            }
                        } else {
                            tracing::info!(
                                "Agent {} creating new memory block '{}' (from update)",
                                agent_id,
                                memory_block.label
                            );
                            // Create new block
                            if let Err(e) =
                                memory.create_block(memory_block.label.clone(), &memory_block.value)
                            {
                                tracing::error!(
                                    "Failed to create memory block {}: {}",
                                    memory_block.label,
                                    e
                                );
                            }
                        }

                        // Update other fields if present
                        if let Some(mut block) = memory.get_block_mut(&memory_block.label) {
                            block.id = memory_block.id;
                            // owner_id is now managed via RELATE, not stored on the entity
                            block.description = memory_block.description;
                            block.metadata = memory_block.metadata;
                            block.embedding_model = memory_block.embedding_model;
                            block.created_at = memory_block.created_at;
                            block.updated_at = memory_block.updated_at;
                            block.is_active = memory_block.is_active;
                        }
                    }
                    surrealdb::Action::Delete => {
                        tracing::info!(
                            "🗑️ Agent {} received DELETE for memory block '{}'",
                            agent_id,
                            memory_block.label
                        );
                        memory.remove_block(&memory_block.label);
                    }
                    _ => {
                        tracing::debug!("Ignoring action {:?} for memory block", action);
                    }
                }
            }

            tracing::warn!(
                "⚠️ Memory sync task exiting for agent {} - stream ended",
                agent_id
            );
        });

        Ok(())
    }
}

impl<C, M, E> std::fmt::Debug for DatabaseAgent<C, M, E>
where
    C: surrealdb::Connection,
    M: ModelProvider,
    E: EmbeddingProvider,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let agent_id = self.context.handle.agent_id;
        f.debug_struct("DatabaseAgent")
            .field("agent_id", &agent_id)
            .field("name", &self.context.handle.name)
            .field("agent_type", &self.context.handle.agent_type)
            .field("user_id", &self.user_id)
            .field("state", &self.context.handle.state)
            .field("has_embeddings", &self.embeddings.is_some())
            .field(
                "memory_blocks",
                &self.context.handle.memory.list_blocks().len(),
            )
            .finish()
    }
}

#[async_trait]
impl<C, M, E> Agent for DatabaseAgent<C, M, E>
where
    C: surrealdb::Connection + std::fmt::Debug + 'static,
    M: ModelProvider + 'static,
    E: EmbeddingProvider + 'static,
{
    fn id(&self) -> AgentId {
        self.context.handle.agent_id
    }

    fn name(&self) -> &str {
        &self.context.handle.name
    }

    fn agent_type(&self) -> AgentType {
        self.context.handle.agent_type.clone()
    }

    async fn process_message(&self, message: Message) -> Result<Response> {
        // Update state to processing
        // Note: state changes would need to use interior mutability
        // For now, we'll track state locally

        // Clone what we need for the background task
        let db = Arc::clone(&self.db);
        let agent_id = self.context.handle.agent_id;
        let message_clone = message.clone();

        // Fire off message persistence in background
        let _handle1 = tokio::spawn(async move {
            if let Err(e) = crate::db::ops::persist_agent_message(
                &db,
                agent_id,
                &message_clone,
                crate::message::MessageRelationType::Active,
            )
            .await
            {
                tracing::error!("Failed to persist incoming message: {:?}", e);
            }
        });

        // Add message to context immediately
        self.context.add_message(message).await;

        // Build the memory context for the LLM
        let memory_context = self.context.build_context().await?;
        let request = Request {
            system: Some(vec![memory_context.system_prompt.clone()]),
            messages: memory_context.messages,
            tools: Some(memory_context.tools),
        };

        let model = self.model.read().await;

        let options = self.chat_options.read().await;

        // Note: this will usually make a network request and will need to wait for generated tokens from the LLM. it's the slowest bit.
        let response = model
            .complete(
                &options.clone().expect("should have options or default"),
                request,
            )
            .await
            .expect("Add error handling later");

        // Convert Response to Message
        let response_message = Message::from_response(&response, self.context.handle.agent_id);

        // Clone for background persistence
        let db = Arc::clone(&self.db);
        let response_msg_clone = response_message.clone();

        // Fire off response persistence in background
        let _handle2 = tokio::spawn(async move {
            if let Err(e) = crate::db::ops::persist_agent_message(
                &db,
                agent_id,
                &response_msg_clone,
                crate::message::MessageRelationType::Active,
            )
            .await
            {
                tracing::error!("Failed to persist response message: {:?}", e);
            }
        });

        // Add response message to context immediately
        self.context.add_message(response_message).await;

        // Process response and update state
        self.context.process_response(&response)?;

        // Update stats in background
        let db = Arc::clone(&self.db);
        let stats = self.context.get_stats().await;
        let _handle3 = tokio::spawn(async move {
            if let Err(e) = crate::db::ops::update_agent_stats(&db, agent_id, &stats).await {
                tracing::error!("Failed to update agent stats: {:?}", e);
            }
        });

        Ok(response)
    }

    async fn get_memory(&self, key: &str) -> Result<Option<MemoryBlock>> {
        Ok(self.context.handle.memory.get_block(key).map(|b| b.clone()))
    }

    async fn update_memory(&self, key: &str, memory: MemoryBlock) -> Result<()> {
        tracing::info!(
            "🔧 Agent {} updating memory key '{}'",
            self.context.handle.agent_id,
            key
        );

        // Update in context immediately
        self.context
            .update_memory_block(key, memory.value.clone())
            .await?;

        // Persist memory block in background
        let db = Arc::clone(&self.db);
        let agent_id = self.context.handle.agent_id;
        let memory_clone = memory.clone();

        let _handle = tokio::spawn(async move {
            if let Err(e) = crate::db::ops::persist_agent_memory(
                &db,
                agent_id,
                &memory_clone,
                MemoryAccessLevel::Write, // Agent has write access to its own memory
            )
            .await
            {
                tracing::error!("Failed to persist memory block: {:?}", e);
            }
        });

        Ok(())
    }

    async fn execute_tool(
        &self,
        tool_name: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value> {
        // Get the tool from the registry
        let tool = self.context.tools.get(tool_name).ok_or_else(|| {
            CoreError::tool_not_found(
                tool_name,
                self.context
                    .tools
                    .list_tools()
                    .iter()
                    .map(|s| s.to_string())
                    .collect(),
            )
        })?;

        // Record start time
        let start_time = std::time::Instant::now();
        let created_at = chrono::Utc::now();

        // Execute the tool
        let result = tool.execute(params.clone()).await;

        // Calculate duration
        let duration_ms = start_time.elapsed().as_millis() as i64;

        // Update tool call count in metadata
        self.context.metadata.write().await.total_tool_calls += 1;

        // Create tool call record
        let tool_call = schema::ToolCall {
            id: crate::id::ToolCallId::generate(),
            agent_id: self.context.handle.agent_id,
            tool_name: tool_name.to_string(),
            parameters: params,
            result: result.as_ref().unwrap_or(&serde_json::json!(null)).clone(),
            error: result.as_ref().err().map(|e| e.to_string()),
            duration_ms,
            created_at,
        };

        // Persist tool call in background
        let db = Arc::clone(&self.db);
        let _handle = tokio::spawn(async move {
            if let Err(e) = crate::db::ops::create_entity(&db, &tool_call).await {
                tracing::error!("Failed to persist tool call: {:?}", e);
            }
        });

        // Return the result
        result
    }

    async fn list_memory_keys(&self) -> Result<Vec<CompactString>> {
        Ok(self.context.handle.memory.list_blocks())
    }

    async fn search_memory(
        &self,
        query: &str,
        limit: usize,
    ) -> Result<Vec<(CompactString, MemoryBlock, f32)>> {
        // For now, do a simple text search in memory blocks
        // TODO: Implement proper semantic search with embeddings
        let query_lower = query.to_lowercase();

        let mut results: Vec<(CompactString, MemoryBlock, f32)> = Vec::new();

        let blocks = self.context.handle.memory.get_all_blocks();
        for block in blocks {
            let content_lower = block.value.to_lowercase();
            if content_lower.contains(&query_lower) {
                // Simple scoring based on match count
                let score =
                    content_lower.matches(&query_lower).count() as f32 / block.value.len() as f32;
                results.push((block.label.clone(), block, score));
            }
        }

        // Sort by score descending
        results.sort_by(|a, b| b.2.partial_cmp(&a.2).unwrap_or(std::cmp::Ordering::Equal));
        results.truncate(limit);

        Ok(results)
    }

    async fn share_memory_with(
        &self,
        memory_key: &str,
        target_agent_id: AgentId,
        access_level: MemoryAccessLevel,
    ) -> Result<()> {
        // First verify the memory block exists
        let memory_block = self
            .context
            .handle
            .memory
            .get_block(memory_key)
            .ok_or_else(|| {
                CoreError::memory_not_found(
                    &self.context.handle.agent_id,
                    memory_key,
                    self.context.handle.memory.list_blocks(),
                )
            })?;

        // Create the memory relation for the target agent
        let db = Arc::clone(&self.db);
        let memory_id = memory_block.id.clone();
        let relation = AgentMemoryRelation {
            id: None,
            in_id: target_agent_id.clone(),
            out_id: memory_id.clone(),
            access_level,
            created_at: chrono::Utc::now(),
        };

        // Persist the sharing relationship in background
        let target_id_clone = target_agent_id.clone();
        let _handle = tokio::spawn(async move {
            if let Err(e) = crate::db::ops::create_relation_typed(&db, &relation).await {
                tracing::error!("Failed to share memory block: {:?}", e);
            } else {
                tracing::info!(
                    "Shared memory block {} with agent {} (access: {:?})",
                    memory_id,
                    target_id_clone,
                    access_level
                );
            }
        });

        Ok(())
    }

    async fn get_shared_memories(&self) -> Result<Vec<(AgentId, CompactString, MemoryBlock)>> {
        let db = Arc::clone(&self.db);
        let agent_id = self.context.handle.agent_id;

        // Query for all memory blocks shared with this agent
        let query = r#"
            SELECT 
                out AS memory_block,
                in AS source_agent_id,
                access_level
            FROM agent_memories
            WHERE in = $agent_id
            FETCH out, in
        "#;

        let mut response = db
            .query(query)
            .bind(("agent_id", surrealdb::RecordId::from(&agent_id)))
            .await
            .map_err(|e| DatabaseError::QueryFailed(e))?;

        // Extract the shared memory relationships
        let results: Vec<serde_json::Value> = response
            .take(0)
            .map_err(|e| DatabaseError::QueryFailed(e))?;

        let mut shared_memories = Vec::new();

        for result in results {
            if let (Some(memory_value), Some(source_id)) =
                (result.get("memory_block"), result.get("source_agent_id"))
            {
                // Parse the memory block
                let memory_block: MemoryBlock = serde_json::from_value(memory_value.clone())
                    .map_err(|e| DatabaseError::SerdeProblem(e))?;

                // Parse the source agent ID
                let source_agent_id: AgentId = serde_json::from_value(source_id.clone())
                    .map_err(|e| DatabaseError::SerdeProblem(e))?;

                shared_memories.push((source_agent_id, memory_block.label.clone(), memory_block));
            }
        }

        Ok(shared_memories)
    }

    async fn system_prompt(&self) -> Vec<String> {
        match self.context.build_context().await {
            Ok(memory_context) => {
                // Split the built system prompt into logical sections
                memory_context
                    .system_prompt
                    .split("\n\n")
                    .map(|s| s.to_string())
                    .filter(|s| !s.is_empty())
                    .collect()
            }
            Err(_) => {
                // If context building fails, return base instructions
                vec![self.context.context_config.base_instructions.clone()]
            }
        }
    }

    fn available_tools(&self) -> Vec<Box<dyn DynamicTool>> {
        self.context.tools.get_all_as_dynamic()
    }

    fn state(&self) -> AgentState {
        self.context.handle.state
    }

    /// Set the agent's state
    async fn set_state(&mut self, state: AgentState) -> Result<()> {
        // Update the state in the context
        self.context.handle.state = state;

        // Update last_active timestamp
        self.context.metadata.write().await.last_active = chrono::Utc::now();

        // Persist the state update in background
        let db = Arc::clone(&self.db);
        let agent_id = self.context.handle.agent_id;
        let _handle = tokio::spawn(async move {
            let query =
                "UPDATE agent SET state = $state, last_active = $last_active WHERE id = $id";
            if let Err(e) = db
                .query(query)
                .bind(("id", surrealdb::RecordId::from(&agent_id)))
                .bind(("state", serde_json::to_value(&state).unwrap()))
                .bind(("last_active", chrono::Utc::now()))
                .await
            {
                tracing::error!("Failed to persist agent state update: {:?}", e);
            }
        });

        Ok(())
    }
}

// Extension trait for database operations
#[async_trait]
pub trait AgentDbExt<C: surrealdb::Connection> {
    async fn create_tool_call(&self, tool_call: schema::ToolCall) -> Result<()>;
}

#[async_trait]
impl<T, C> AgentDbExt<C> for T
where
    T: AsRef<surrealdb::Surreal<C>> + Sync,
    C: surrealdb::Connection,
{
    async fn create_tool_call(&self, tool_call: schema::ToolCall) -> Result<()> {
        // Store the tool call in the database
        crate::db::ops::create_entity(self.as_ref(), &tool_call).await?;
        Ok(())
    }
}
