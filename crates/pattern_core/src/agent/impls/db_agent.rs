//! Database-backed agent implementation

use async_trait::async_trait;
use compact_str::CompactString;
use std::sync::Arc;
use surrealdb::Surreal;
use tokio::sync::RwLock;

use crate::tool::builtin::BuiltinTools;

use crate::{
    CoreError, MemoryBlock, ModelProvider, Result, UserId,
    agent::{Agent, AgentState, AgentType, MemoryAccessLevel},
    context::{AgentContext, ContextConfig},
    db::{
        ops::{LiveQueryExt, MemoryOpsExt},
        schema,
    },
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

        // State would be updated here if using interior mutability

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

        // Update in context
        self.context
            .update_memory_block(key, memory.value.clone())
            .await?;

        self.db
            .update_memory_content(memory.id, memory.value, self.embeddings.as_deref())
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
        // Context persistence would happen here if needed

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
        self.context.handle.state
    }

    /// Set the agent's state
    async fn set_state(&mut self, state: AgentState) -> Result<()> {
        // Note: This would need to use interior mutability to modify state
        // through a shared reference. For now, state changes are local.
        let _ = state; // Suppress unused variable warning
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
    async fn create_tool_call(&self, _tool_call: schema::ToolCall) -> Result<()> {
        // Store the tool call in the database
        todo!("Implement create_tool_call")
    }
}
