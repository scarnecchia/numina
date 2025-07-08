//! Database-backed agent implementation

use async_trait::async_trait;
use parking_lot::RwLock;
use std::sync::Arc;
use surrealdb::{RecordId, Surreal};

use crate::{
    CoreError, Result,
    agent::{Agent, AgentState, AgentType, MemoryAccessLevel},
    db::{DatabaseError, DbAgent, ops::SurrealExt, schema},
    embeddings::EmbeddingProvider,
    id::{AgentId, IdType},
    llm::LlmProvider,
    memory::Memory,
    message::{Message, Response, ResponseMetadata},
    tool::{DynamicTool, ToolRegistry},
};

/// A concrete agent implementation backed by the database
pub struct DatabaseAgent<C: surrealdb::Connection> {
    /// The agent's database record
    agent: schema::Agent,

    /// Database connection
    db: Arc<Surreal<C>>,

    /// LLM provider for generating responses
    llm: Arc<dyn LlmProvider>,

    /// Embedding provider for semantic search
    embeddings: Option<Arc<dyn EmbeddingProvider>>,

    /// Tool registry for this agent
    tools: Arc<ToolRegistry>,

    /// Cached memory blocks
    memory_cache: Arc<RwLock<Memory>>,

    /// Subscribed memory keys for live updates
    subscriptions: Arc<RwLock<Vec<String>>>,
}

impl<C: surrealdb::Connection> DatabaseAgent<C> {
    /// Create a new database-backed agent
    pub async fn new(
        agent_id: AgentId,
        db: Arc<Surreal<C>>,
        llm: Arc<dyn LlmProvider>,
        embeddings: Option<Arc<dyn EmbeddingProvider>>,
        tools: Arc<ToolRegistry>,
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

        Ok(Self {
            agent,
            db,
            llm,
            embeddings,
            tools,
            memory_cache: Arc::new(RwLock::new(Memory::new())),
            subscriptions: Arc::new(RwLock::new(Vec::new())),
        })
    }

    /// Start background task to sync memory updates via live queries
    pub async fn start_memory_sync(self: Arc<Self>) -> Result<()> {
        let agent_id = self.agent.id;
        let memory_cache = Arc::clone(&self.memory_cache);
        let db = Arc::clone(&self.db);
        self.refresh_memory_cache().await?;

        // Spawn background task to handle updates
        tokio::spawn(async move {
            use futures::StreamExt;
            tracing::debug!("Memory sync task started for agent {}", agent_id);

            // Subscribe to all memory changes for this agent
            let stream = match db.subscribe_to_agent_memory_updates(agent_id).await {
                Ok(s) => {
                    tracing::debug!(
                        "Successfully subscribed to memory updates for agent {}",
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
                tracing::trace!(
                    "Agent {} received memory update: {:?} for block '{}'",
                    agent_id,
                    action,
                    memory_block.label
                );
                let mut cache = memory_cache.write();

                match action {
                    surrealdb::Action::Create => {
                        tracing::trace!("creating {}", memory_block.label);
                        // Create new block
                        if let Err(e) =
                            cache.create_block(&memory_block.label, &memory_block.content)
                        {
                            tracing::error!(
                                "Failed to create memory block {}: {}",
                                memory_block.label,
                                e
                            );
                        }
                        // Update description if present
                        if let Some(desc) = &memory_block.description {
                            if let Some(mem_block) = cache.get_block_mut(&memory_block.label) {
                                mem_block.description = Some(desc.clone());
                            }
                        }
                    }

                    surrealdb::Action::Update => {
                        // Update or create the block
                        if cache.get_block(&memory_block.label).is_some() {
                            tracing::trace!("updating {}", memory_block.label);
                            // Update existing block
                            if let Err(e) =
                                cache.update_block_value(&memory_block.label, &memory_block.content)
                            {
                                tracing::error!(
                                    "Failed to update memory block {}: {}",
                                    memory_block.label,
                                    e
                                );
                            }
                        } else {
                            tracing::trace!("creating {}", memory_block.label);
                            // Create new block
                            if let Err(e) =
                                cache.create_block(&memory_block.label, &memory_block.content)
                            {
                                tracing::error!(
                                    "Failed to create memory block {}: {}",
                                    memory_block.label,
                                    e
                                );
                            }
                        }

                        // Update description if present
                        if let Some(desc) = &memory_block.description {
                            if let Some(mem_block) = cache.get_block_mut(&memory_block.label) {
                                mem_block.description = Some(desc.clone());
                            }
                        }
                    }
                    surrealdb::Action::Delete => {
                        tracing::debug!(
                            "Agent {} received DELETE for memory block '{}'",
                            agent_id,
                            memory_block.label
                        );
                        cache.remove_block(&memory_block.label);
                    }
                    _ => {
                        tracing::debug!("Ignoring action {:?} for memory block", action);
                    }
                }
            }

            tracing::debug!("Memory sync task exiting for agent {}", agent_id);
            tracing::info!("Memory sync stream ended for agent {}", agent_id);
        });

        Ok(())
    }

    /// Load memory blocks from database into cache
    async fn refresh_memory_cache(&self) -> Result<()> {
        let memories = self.db.get_agent_memories(self.agent.id).await?;

        let mut memory = Memory::new();
        for (block, _access_level) in memories {
            memory.create_block(&block.label, &block.content)?;
            if let Some(desc) = &block.description {
                if let Some(mem_block) = memory.get_block_mut(&block.label) {
                    mem_block.description = Some(desc.clone());
                }
            }
        }

        *self.memory_cache.write() = memory;
        Ok(())
    }

    /// Get or load memory cache
    async fn get_memory_cache(&self) -> Result<Memory> {
        // // Cache miss, load from database
        // self.refresh_memory_cache().await?;

        Ok(self.memory_cache.read().clone())
    }
}

impl<C: surrealdb::Connection> std::fmt::Debug for DatabaseAgent<C> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DatabaseAgent")
            .field("agent_id", &self.agent.id)
            .field("agent_name", &self.agent.name)
            .field("agent_type", &self.agent.agent_type)
            .field("state", &self.agent.state)
            .field("has_embeddings", &self.embeddings.is_some())
            .field("tools_count", &self.tools.list_tools().len())
            .field("subscriptions", &self.subscriptions.read().len())
            .finish()
    }
}

#[async_trait]
impl<C: surrealdb::Connection + 'static> Agent for DatabaseAgent<C>
where
    C: std::fmt::Debug,
{
    fn id(&self) -> AgentId {
        self.agent.id
    }

    fn name(&self) -> &str {
        &self.agent.name
    }

    fn agent_type(&self) -> AgentType {
        self.agent.agent_type.clone()
    }

    async fn process_message(&self, message: Message) -> Result<Response> {
        // Update state to processing
        self.db
            .update_agent_state(self.agent.id, AgentState::Processing)
            .await?;

        // Get current memory context
        let memory = self.get_memory_cache().await?;

        // Build context from memory blocks
        let mut context = String::new();
        context.push_str("# Current Memory Context\n\n");

        for block in memory.get_all_blocks() {
            context.push_str(&format!("## {}\n{}\n\n", block.label, block.value));
        }

        // Add available tools to context
        context.push_str("# Available Tools\n\n");
        for tool in &self.available_tools() {
            context.push_str(&format!("- {}: {}\n", tool.name(), tool.description()));
        }

        // Generate response using LLM
        let system_prompt = format!("{}\n\n{}", self.agent.system_prompt, context);

        // Extract text content from the message
        let message_text = message.text_content().ok_or_else(|| {
            CoreError::tool_validation_error("process_message", "Message has no text content")
        })?;

        let response = self
            .llm
            .generate_response(
                &system_prompt,
                &message_text,
                None, // temperature
                None, // max_tokens
            )
            .await?;

        // Update state back to ready
        self.db
            .update_agent_state(self.agent.id, AgentState::Ready)
            .await?;

        Ok(Response {
            id: crate::MessageId::generate(),
            content: Some(genai::chat::MessageContent::Text(response)),
            reasoning: None,
            tool_calls: vec![],
            metadata: ResponseMetadata {
                processing_time: None,
                tokens_used: None,
                model_used: Some(self.llm.model_name().to_string()),
                confidence: None,
                model_iden: genai::ModelIden {
                    adapter_kind: genai::adapter::AdapterKind::Anthropic,
                    model_name: self.llm.model_name().to_string().into(),
                },
                custom: serde_json::Value::Null,
            },
        })
    }

    async fn get_memory(&self, key: &str) -> Result<Option<Memory>> {
        let memory = self.get_memory_cache().await?;

        if let Some(block) = memory.get_block(key) {
            let mut single_memory = Memory::new();
            single_memory.create_block(&block.label, &block.value)?;
            if let Some(desc) = &block.description {
                if let Some(mem_block) = single_memory.get_block_mut(&block.label) {
                    mem_block.description = Some(desc.clone());
                }
            }
            Ok(Some(single_memory))
        } else {
            Ok(None)
        }
    }

    async fn update_memory(&self, key: &str, memory: Memory) -> Result<()> {
        tracing::debug!("Agent {} updating memory key '{}'", self.agent.id, key);
        // Find the memory block in our cache
        let current_memory = self.get_memory_cache().await?;

        // Look up the memory ID by label
        let memory_block_result = self.db.get_memory_by_label(self.agent.id, key).await?;

        tracing::trace!(
            "Memory lookup for '{}' returned: {:?}",
            key,
            memory_block_result.is_some()
        );

        let memory_block = memory_block_result.ok_or_else(|| CoreError::MemoryNotFound {
            agent_id: self.agent.id.to_string(),
            block_name: key.to_string(),
            available_blocks: current_memory.list_blocks(),
        })?;

        // Get the new content from the provided memory
        let new_content = memory
            .get_block(key)
            .map(|b| b.value.clone())
            .ok_or_else(|| CoreError::MemoryNotFound {
                agent_id: self.agent.id.to_string(),
                block_name: key.to_string(),
                available_blocks: memory.list_blocks(),
            })?;

        // Update in database
        self.db
            .update_memory_content(memory_block.id, new_content, self.embeddings.as_deref())
            .await?;

        // Refresh cache
        self.refresh_memory_cache().await?;

        Ok(())
    }

    async fn execute_tool(
        &self,
        tool_name: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value> {
        let tool = self.tools.get(tool_name).ok_or_else(|| {
            CoreError::tool_not_found(
                tool_name,
                self.tools
                    .list_tools()
                    .iter()
                    .map(|s| s.to_string())
                    .collect(),
            )
        })?;

        // Execute the tool
        let result = tool.execute(params).await?;

        // TODO: Log tool execution to database once we have the proper schema

        Ok(result)
    }

    async fn list_memory_keys(&self) -> Result<Vec<String>> {
        let memory = self.get_memory_cache().await?;
        Ok(memory.list_blocks())
    }

    async fn search_memory(&self, query: &str, limit: usize) -> Result<Vec<(String, Memory, f32)>> {
        let results = self
            .db
            .search_memories(
                self.embeddings.as_deref().ok_or_else(|| {
                    CoreError::tool_validation_error(
                        "search_memory",
                        "No embedding provider configured",
                    )
                })?,
                self.agent.id,
                query,
                limit,
            )
            .await?;

        let mut output = Vec::new();
        for (block, score) in results {
            let mut memory = Memory::new();
            memory.create_block(&block.label, &block.content)?;
            if let Some(desc) = &block.description {
                if let Some(mem_block) = memory.get_block_mut(&block.label) {
                    mem_block.description = Some(desc.clone());
                }
            }
            output.push((block.label.clone(), memory, score));
        }

        Ok(output)
    }

    async fn share_memory_with(
        &self,
        memory_key: &str,
        target_agent_id: AgentId,
        access_level: MemoryAccessLevel,
    ) -> Result<()> {
        // Get the memory block
        let memory_block = self
            .db
            .get_memory_by_label(self.agent.id, memory_key)
            .await?
            .ok_or_else(|| CoreError::MemoryNotFound {
                agent_id: self.agent.id.to_string(),
                block_name: memory_key.to_string(),
                available_blocks: vec![],
            })?;

        // Create the agent-memory relationship
        self.db
            .attach_memory_to_agent(target_agent_id, memory_block.id, access_level)
            .await?;

        Ok(())
    }

    async fn get_shared_memories(&self) -> Result<Vec<(AgentId, String, Memory)>> {
        // This would need a reverse lookup - find all memory blocks that other agents
        // have shared with this agent. For now, return empty.
        // TODO: Implement reverse lookup query
        Ok(vec![])
    }

    async fn subscribe_to_memory(&self, memory_key: &str) -> Result<()> {
        // Get the memory block
        let memory_block = self
            .db
            .get_memory_by_label(self.agent.id, memory_key)
            .await?
            .ok_or_else(|| CoreError::MemoryNotFound {
                agent_id: self.agent.id.to_string(),
                block_name: memory_key.to_string(),
                available_blocks: vec![],
            })?;

        // Subscribe to updates
        let _ = self.db.subscribe_to_memory_updates(memory_block.id).await?;

        // Track subscription
        self.subscriptions.write().push(memory_key.to_string());

        Ok(())
    }

    fn system_prompt(&self) -> &str {
        &self.agent.system_prompt
    }

    fn available_tools(&self) -> Vec<Box<dyn DynamicTool>> {
        self.tools.get_all_as_dynamic()
    }

    fn state(&self) -> AgentState {
        self.agent.state
    }

    async fn set_state(&mut self, state: AgentState) -> Result<()> {
        self.db.update_agent_state(self.agent.id, state).await?;
        self.agent.state = state;
        Ok(())
    }
}

// Extension trait for database operations
#[async_trait]
pub trait AgentDbExt<C: surrealdb::Connection> {
    async fn get_agent(&self, agent_id: AgentId) -> Result<Option<schema::Agent>>;
    async fn create_tool_call(&self, tool_call: schema::ToolCall) -> Result<()>;
}

#[async_trait]
impl<T, C> AgentDbExt<C> for T
where
    T: AsRef<Surreal<C>> + Sync,
    C: surrealdb::Connection,
{
    async fn get_agent(&self, agent_id: AgentId) -> Result<Option<schema::Agent>> {
        let query = format!(
            "SELECT * FROM {} WHERE id = $agent_id LIMIT 1",
            crate::id::AgentIdType::PREFIX
        );

        let mut result = self
            .as_ref()
            .query(query)
            .bind(("agent_id", RecordId::from(agent_id)))
            .await
            .map_err(DatabaseError::QueryFailed)?;

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
