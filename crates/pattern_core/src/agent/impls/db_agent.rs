//! Database-backed agent implementation

use async_trait::async_trait;
use compact_str::CompactString;
use std::str::FromStr;
use std::sync::Arc;
use surrealdb::{RecordId, Surreal};
use tokio::sync::RwLock;
use uuid::Uuid;

use crate::db::models::{from_surreal_datetime, strip_brackets};
use crate::tool::builtin::BuiltinTools;

use crate::{
    CoreError, MemoryBlock, ModelProvider, Result, UserId,
    agent::{Agent, AgentState, AgentType, MemoryAccessLevel},
    context::AgentContext,
    db::{DatabaseError, DbAgent, ops::SurrealExt, schema},
    embeddings::EmbeddingProvider,
    id::{AgentId, IdType},
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
    /// The agent's ID (stored separately for sync access)
    agent_id: AgentId,

    /// The agent's name (stored separately for sync access)
    name: String,

    /// The agent's type (stored separately for sync access)
    agent_type: AgentType,

    /// The user who owns this agent
    user_id: UserId,

    /// The agent's current state (stored separately for sync access)
    state: AgentState,

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
    pub async fn new(
        agent_id: AgentId,
        db: Arc<Surreal<C>>,
        model: Arc<RwLock<M>>,
        embeddings: Option<Arc<E>>,
        tools: ToolRegistry,
    ) -> Result<Self> {
        // Load the agent from database
        let agent = db
            .get_agent(agent_id)
            .await?
            .ok_or_else(|| CoreError::AgentNotFound {
                src: format!("agent lookup: {}", agent_id),
                span: (14, 14 + agent_id.to_string().len()),
                id: agent_id.to_string(),
            })?;

        let user_id = UserId::from_uuid(
            Uuid::from_str(strip_brackets(&agent.user_id.key().to_string())).unwrap(),
        );

        // Create memory with owner
        let memory = Memory::with_owner(user_id.clone());

        // Add memory blocks
        for block in &agent.memory_blocks {
            memory.create_block(block.label.clone(), &block.content)?;
            if let Some(desc) = &block.description {
                if let Some(mut block) = memory.get_block_mut(&block.label) {
                    block.description = Some(desc.clone());
                }
            }
        }

        // Build AgentContext with tools
        let context = AgentContext::new(
            agent_id,
            agent.agent_type.clone(),
            memory,
            tools,
            serde_json::from_value(agent.context_config)
                .map_err(|e| {
                    crate::id::IdError::InvalidFormat(format!("Invalid context_config: {}", e))
                })
                .expect("context config should be valid"),
        );

        // Register built-in tools
        let builtin = BuiltinTools::default_for_agent(context.handle());
        builtin.register_all(&context.tools);

        // Restore message history
        {
            let mut history = context.history.write().await;
            history.messages = agent.messages.clone();
            history.archived_messages = agent.archived_messages.clone();
            history.message_summary = agent.message_summary.clone();
            history.compression_strategy = serde_json::from_value(agent.compression_strategy)
                .expect("compression strategy should be valid")
        }

        // Restore metadata
        {
            let mut metadata = context.metadata.write().await;
            metadata.created_at = from_surreal_datetime(agent.created_at);
            metadata.last_active = from_surreal_datetime(agent.last_active);
            metadata.total_messages = agent.total_messages;
            metadata.total_tool_calls = agent.total_tool_calls;
            metadata.context_rebuilds = agent.context_rebuilds;
            metadata.compression_events = agent.compression_events;
        }

        Ok(Self {
            agent_id,
            name: agent.name.clone(),
            agent_type: agent.agent_type.clone(),
            user_id,
            state: agent.state,
            context,
            chat_options: Arc::new(RwLock::const_new(None)),
            db,
            embeddings,
            model,
        })
    }

    /// Persist the current context state to the database
    async fn persist_context(&self) -> Result<()> {
        let history = self.context.history.read().await;
        let metadata = self.context.metadata.read().await;

        // Update the agent record with the current context
        self.db
            .update_agent_context(
                self.context.handle.agent_id.clone(),
                self.context.handle.memory.get_all_blocks(),
                history.messages.clone(),
                history.archived_messages.clone(),
                history.message_summary.clone(),
                self.context.context_config.clone(),
                history.compression_strategy.clone(),
                metadata.clone(),
            )
            .await?;

        Ok(())
    }

    /// Start background task to sync memory updates via live queries
    pub async fn start_memory_sync(self: Arc<Self>) -> Result<()> {
        let agent_id = self.agent_id;
        let memory = self.context.handle.memory.clone();
        let db = Arc::clone(&self.db);

        // Spawn background task to handle updates
        tokio::spawn(async move {
            use futures::StreamExt;
            tracing::info!("Memory sync task started for agent {}", agent_id);

            // Subscribe to all memory changes for this agent
            let stream = match db.subscribe_to_agent_memory_updates(agent_id).await {
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
                    memory_block.content
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
                            memory.create_block(memory_block.label.clone(), &memory_block.content)
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
                            block.owner_id = memory_block.owner_id;
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
                                memory_block.content
                            );
                            // Update existing block
                            if let Err(e) = memory
                                .update_block_value(&memory_block.label, &memory_block.content)
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
                            if let Err(e) = memory
                                .create_block(memory_block.label.clone(), &memory_block.content)
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
                            block.owner_id = memory_block.owner_id;
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
        f.debug_struct("DatabaseAgent")
            .field("agent_id", &self.agent_id)
            .field("name", &self.name)
            .field("agent_type", &self.agent_type)
            .field("user_id", &self.user_id)
            .field("state", &self.state)
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
        self.agent_id
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn agent_type(&self) -> AgentType {
        self.agent_type.clone()
    }

    async fn process_message(&self, message: Message) -> Result<Response> {
        // Update state to processing
        self.db
            .update_agent_state(self.agent_id, AgentState::Processing)
            .await?;

        // Add message to context
        self.context.add_message(message.clone()).await;

        // Build the memory context for the LLM
        let memory_context = self.context.build_context().await?;
        let request = Request {
            system: Some(vec![memory_context.system_prompt.clone()]),
            messages: memory_context.messages,
            tools: Some(memory_context.tools),
        };

        let model = self.model.read().await;

        let options = self.chat_options.read().await;
        let response = model
            .complete(
                &options.clone().expect("should have options or default"),
                request,
            )
            .await
            .expect("Add error handling later");

        // Handle

        // Process response and update state
        self.context.process_response(&response)?;

        // Persist context changes
        self.persist_context().await?;

        // Update state back to ready
        self.db
            .update_agent_state(self.agent_id, AgentState::Ready)
            .await?;

        Ok(response)
    }

    async fn get_memory(&self, key: &str) -> Result<Option<MemoryBlock>> {
        Ok(self.context.handle.memory.get_block(key).map(|b| b.clone()))
    }

    async fn update_memory(&self, key: &str, memory: MemoryBlock) -> Result<()> {
        tracing::info!("🔧 Agent {} updating memory key '{}'", self.agent_id, key);

        // Update in context
        self.context
            .update_memory_block(key, memory.content.clone())
            .await?;

        self.db
            .update_memory_content(memory.id, memory.content, self.embeddings.as_deref())
            .await?;

        // Persist to database
        //self.persist_context().await?;

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

        // Execute the tool
        let result = tool.execute(params).await?;

        // Update tool call count in metadata
        self.context.metadata.write().await.total_tool_calls += 1;

        // Persist the updated metadata
        self.persist_context().await?;

        Ok(result)
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
            let content_lower = block.content.to_lowercase();
            if content_lower.contains(&query_lower) {
                // Simple scoring based on match count
                let score =
                    content_lower.matches(&query_lower).count() as f32 / block.content.len() as f32;
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
        _memory_key: &str,
        _target_agent_id: AgentId,
        _access_level: MemoryAccessLevel,
    ) -> Result<()> {
        // This would need to be reimplemented based on new sharing model
        // For now, return not implemented
        Err(CoreError::tool_validation_error(
            "share_memory_with",
            "Memory sharing not yet implemented with new context model",
        ))
    }

    async fn get_shared_memories(&self) -> Result<Vec<(AgentId, CompactString, MemoryBlock)>> {
        // This would need a reverse lookup - find all memory blocks that other agents
        // have shared with this agent. For now, return empty.
        // TODO: Implement reverse lookup query
        Ok(vec![])
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
        self.state
    }

    async fn set_state(&mut self, state: AgentState) -> Result<()> {
        self.state = state;
        self.db.update_agent_state(self.agent_id, state).await?;
        Ok(())
    }
}

// Extension trait for database operations
#[async_trait]
pub trait AgentDbExt<C: surrealdb::Connection> {
    async fn get_agent(&self, agent_id: AgentId) -> Result<Option<DbAgent>>;
    async fn create_tool_call(&self, tool_call: schema::ToolCall) -> Result<()>;
}

#[async_trait]
impl<T, C> AgentDbExt<C> for T
where
    T: AsRef<Surreal<C>> + Sync,
    C: surrealdb::Connection,
{
    async fn get_agent(&self, agent_id: AgentId) -> Result<Option<DbAgent>> {
        let query = format!(
            "SELECT * FROM {} WHERE id = $agent_id LIMIT 1 FETCH memory_blocks, messages, archived_messages",
            crate::id::AgentIdType::PREFIX
        );

        let mut result = self
            .as_ref()
            .query(query)
            .bind(("agent_id", RecordId::from(agent_id)))
            .await
            .map_err(DatabaseError::QueryFailed)?;

        println!("agent response: {:#?}", result);
        let agents: Vec<DbAgent> = result.take(0).map_err(DatabaseError::QueryFailed)?;

        Ok(agents
            .into_iter()
            .next()
            .and_then(|db_agent| db_agent.try_into().ok()))
    }

    async fn create_tool_call(&self, _tool_call: schema::ToolCall) -> Result<()> {
        // Store the tool call in the database
        todo!("Implement create_tool_call")
    }
}
