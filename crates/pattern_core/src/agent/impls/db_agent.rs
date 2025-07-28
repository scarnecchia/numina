//! Database-backed agent implementation

use async_trait::async_trait;
use compact_str::CompactString;
use futures::{Stream, StreamExt};
use std::sync::Arc;
use surrealdb::Surreal;
use tokio::sync::RwLock;

use crate::id::RelationId;
use crate::message::{ContentPart, ImageSource, ToolResponse};
use crate::tool::builtin::BuiltinTools;

use crate::{
    CoreError, MemoryBlock, ModelProvider, Result, UserId,
    agent::{Agent, AgentMemoryRelation, AgentState, AgentType},
    context::{
        AgentContext, ContextConfig,
        heartbeat::{HeartbeatRequest, HeartbeatSender, check_heartbeat_request},
    },
    db::{DatabaseError, ops, schema},
    embeddings::EmbeddingProvider,
    id::AgentId,
    memory::Memory,
    message::{Message, MessageContent, Request, Response},
    model::ResponseOptions,
    tool::{DynamicTool, ToolRegistry},
};
use chrono::Utc;

use crate::agent::ResponseEvent;

/// A concrete agent implementation backed by the database
#[derive(Clone)]
pub struct DatabaseAgent<C, M, E>
where
    C: surrealdb::Connection + Clone,
{
    /// The user who owns this agent
    pub user_id: UserId,
    /// The agent's context (includes all state)
    context: Arc<RwLock<AgentContext<C>>>,

    /// model provider
    model: Arc<RwLock<M>>,

    /// model configuration
    pub chat_options: Arc<RwLock<Option<ResponseOptions>>>,
    /// Database connection
    db: Surreal<C>,

    /// Embedding provider for semantic search
    embeddings: Option<Arc<E>>,

    /// Channel for sending heartbeat requests
    pub heartbeat_sender: HeartbeatSender,
}

impl<C, M, E> DatabaseAgent<C, M, E>
where
    C: surrealdb::Connection + Clone + std::fmt::Debug,
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
        db: Surreal<C>,
        model: Arc<RwLock<M>>,
        tools: ToolRegistry,
        embeddings: Option<Arc<E>>,
        heartbeat_sender: HeartbeatSender,
    ) -> Self {
        let context_config = if !system_prompt.is_empty() {
            ContextConfig {
                base_instructions: system_prompt,
                ..Default::default()
            }
        } else {
            ContextConfig::default()
        };
        // Build AgentContext with tools
        let mut context = AgentContext::new(
            agent_id.clone(),
            name,
            agent_type,
            memory,
            tools,
            context_config,
        );
        context.handle.state = AgentState::Ready;

        // Wire up database connection to handle for archival memory tools
        context.handle = context.handle.with_db(db.clone());

        // Create and wire up message router
        let router =
            crate::context::message_router::AgentMessageRouter::new(agent_id.clone(), db.clone());

        // Add router to handle - endpoints will be registered by the consumer
        context.handle = context.handle.with_message_router(router);

        // Register built-in tools
        // TODO: Fix tool schema for Gemini compatibility
        let builtin = BuiltinTools::default_for_agent(context.handle());
        builtin.register_all(&context.tools);

        Self {
            user_id,
            context: Arc::new(RwLock::new(context)),
            chat_options: Arc::new(RwLock::const_new(None)),
            db,
            embeddings,
            model,
            heartbeat_sender,
        }
    }

    /// Set the heartbeat sender for this agent
    pub fn set_heartbeat_sender(&mut self, sender: HeartbeatSender) {
        self.heartbeat_sender = sender;
    }

    /// Get the embedding provider for this agent
    pub fn embedding_provider(&self) -> Option<Arc<E>> {
        self.embeddings.clone()
    }

    /// Create a DatabaseAgent from a persisted AgentRecord
    ///
    /// This reconstructs the runtime agent from its stored state.
    /// The model provider, tools, and embeddings are injected at runtime.
    pub async fn from_record(
        record: crate::agent::AgentRecord,
        db: Surreal<C>,
        model: Arc<RwLock<M>>,
        tools: ToolRegistry,
        embeddings: Option<Arc<E>>,
        heartbeat_sender: HeartbeatSender,
    ) -> Result<Self> {
        tracing::debug!(
            "Creating DatabaseAgent from record: agent_id={}, name={}",
            record.id,
            record.name
        );
        tracing::debug!(
            "Record has {} messages and {} memory blocks",
            record.messages.len(),
            record.memories.len()
        );

        // Create memory with the owner
        let memory = Memory::with_owner(&record.owner_id);

        // Load memory blocks from the record's relations
        for (memory_block, _relation) in &record.memories {
            tracing::debug!(
                "Loading memory block: label={}, size={} chars",
                memory_block.label,
                memory_block.value.len()
            );
            memory.create_block(memory_block.label.clone(), memory_block.value.clone())?;
            if let Some(mut block) = memory.get_block_mut(&memory_block.label) {
                block.id = memory_block.id.clone();
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
        let agent = Self::new(
            record.id.clone(),
            record.owner_id.clone(),
            record.agent_type.clone(),
            record.name.clone(),
            context_config.base_instructions.clone(),
            memory,
            db,
            model,
            tools,
            embeddings,
            heartbeat_sender,
        );

        // Restore the agent state
        {
            let mut context = agent.context.write().await;
            context.handle.state = record.state;
        }

        // Restore message history and compression state
        {
            let context = agent.context.read().await;
            let mut history = context.history.write().await;
            history.compression_strategy = record.compression_strategy.clone();
            history.message_summary = record.message_summary.clone();

            // Load active messages from relations (already ordered by position)
            let mut loaded_messages = 0;
            for (message, relation) in &record.messages {
                if relation.message_type == crate::message::MessageRelationType::Active {
                    let content_preview = match &message.content {
                        crate::message::MessageContent::Text(text) => {
                            text.chars().take(50).collect::<String>()
                        }
                        crate::message::MessageContent::ToolCalls(tool_calls) => {
                            format!("ToolCalls: {} calls", tool_calls.len())
                        }
                        crate::message::MessageContent::ToolResponses(tool_responses) => {
                            format!("ToolResponses: {} responses", tool_responses.len())
                        }
                        crate::message::MessageContent::Parts(parts) => {
                            format!("Parts: {} parts", parts.len())
                        }
                    };
                    tracing::trace!(
                        "Loading message: id={:?}, role={:?}, content_preview={:?}",
                        message.id,
                        message.role,
                        content_preview
                    );
                    history.messages.push(message.clone());
                    loaded_messages += 1;
                }
            }
            tracing::debug!("Loaded {} active messages into history", loaded_messages);
        }

        // Restore statistics
        {
            let context = agent.context.write().await;
            let mut metadata = context.metadata.write().await;
            metadata.created_at = record.created_at;
            metadata.last_active = record.last_active;
            metadata.total_messages = record.total_messages;
            metadata.total_tool_calls = record.total_tool_calls;
            metadata.context_rebuilds = record.context_rebuilds;
            metadata.compression_events = record.compression_events;
        }

        Ok(agent)
    }

    /// Store the current agent state to the database
    pub async fn store(&self) -> Result<()> {
        // Create an AgentRecord from the current state
        let (agent_id, name, agent_type, state) = {
            let context = self.context.read().await;
            (
                context.handle.agent_id.clone(),
                context.handle.name.clone(),
                context.handle.agent_type.clone(),
                context.handle.state,
            )
        };

        let (base_instructions, max_messages) = {
            let context = self.context.read().await;
            (
                context.context_config.base_instructions.clone(),
                context.context_config.max_context_messages,
            )
        };

        let (total_messages, total_tool_calls, context_rebuilds, compression_events, last_active) = {
            let context = self.context.read().await;
            let metadata = context.metadata.read().await;
            (
                metadata.total_messages,
                metadata.total_tool_calls,
                metadata.context_rebuilds,
                metadata.compression_events,
                metadata.last_active,
            )
        };

        let now = chrono::Utc::now();
        let agent_record = crate::agent::AgentRecord {
            id: agent_id,
            name,
            agent_type,
            state,
            base_instructions,
            max_messages,
            owner_id: self.user_id.clone(),
            total_messages,
            total_tool_calls,
            context_rebuilds,
            compression_events,
            created_at: now, // This will be overwritten if agent already exists
            updated_at: now,
            last_active,
            ..Default::default()
        };

        // Store the agent record
        agent_record.store_with_relations(&self.db).await?;
        Ok(())
    }

    /// Start background task to sync memory updates via live queries
    pub async fn start_memory_sync(self: Arc<Self>) -> Result<()> {
        let (agent_id, memory) = {
            let context = self.context.read().await;
            (
                context.handle.agent_id.clone(),
                context.handle.memory.clone(),
            )
        };
        let db = self.db.clone();

        // Spawn background task to handle updates
        tokio::spawn(async move {
            use futures::StreamExt;
            tracing::debug!("Memory sync task started for agent {}", agent_id);

            // Subscribe to all memory changes for this agent
            let stream = match ops::subscribe_to_agent_memory_updates(&db, &agent_id).await {
                Ok(s) => {
                    tracing::debug!(
                        "Successfully subscribed to memory updates for agent {} - live query active",
                        agent_id
                    );
                    s
                }
                Err(e) => {
                    crate::log_error!("Failed to subscribe to memory updates", e);
                    return;
                }
            };

            futures::pin_mut!(stream);

            while let Some((action, memory_block)) = stream.next().await {
                tracing::debug!(
                    "🔔 Agent {} received live query notification: {:?} for block '{}' with content: '{}'",
                    agent_id,
                    action,
                    memory_block.label,
                    memory_block.value
                );

                match action {
                    surrealdb::Action::Create => {
                        tracing::debug!(
                            "Agent {} creating memory block '{}'",
                            agent_id,
                            memory_block.label
                        );
                        // Create new block
                        if let Err(e) =
                            memory.create_block(memory_block.label.clone(), &memory_block.value)
                        {
                            crate::log_error!(
                                format!("Failed to create memory block {}", memory_block.label),
                                e
                            );
                        }
                        // Update other fields if present
                        if let Some(mut block) = memory.get_block_mut(&memory_block.label) {
                            block.id = memory_block.id.clone();
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
                            tracing::debug!(
                                "✅ Agent {} updating existing memory '{}' with content: '{}'",
                                agent_id,
                                memory_block.label,
                                memory_block.value
                            );
                            // Update existing block
                            if let Err(e) =
                                memory.update_block_value(&memory_block.label, &memory_block.value)
                            {
                                crate::log_error!(
                                    format!("Failed to update memory block {}", memory_block.label),
                                    e
                                );
                            }
                        } else {
                            tracing::debug!(
                                "Agent {} creating new memory block '{}' (from update)",
                                agent_id,
                                memory_block.label
                            );
                            // Create new block
                            if let Err(e) =
                                memory.create_block(memory_block.label.clone(), &memory_block.value)
                            {
                                crate::log_error!(
                                    format!("Failed to create memory block {}", memory_block.label),
                                    e
                                );
                            }
                        }

                        // Update other fields if present
                        if let Some(mut block) = memory.get_block_mut(&memory_block.label) {
                            block.id = memory_block.id.clone();
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
                        tracing::debug!(
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

    /// Start background task to monitor incoming messages
    pub async fn start_message_monitoring(self: Arc<Self>) -> Result<()> {
        let db = self.db.clone();
        let agent_id = {
            let context = self.context.read().await;
            context.handle.agent_id.clone()
        };

        // Create a weak reference for the spawned task
        let agent_weak = Arc::downgrade(&self);

        tokio::spawn(async move {
            let stream = match crate::db::ops::subscribe_to_agent_messages(&db, &agent_id).await {
                Ok(stream) => stream,
                Err(e) => {
                    crate::log_error!("Failed to subscribe to agent messages", e);
                    return;
                }
            };

            tracing::info!("📬 Agent {} message monitoring started", agent_id);

            tokio::pin!(stream);
            while let Some((action, mut queued_msg)) = stream.next().await {
                match action {
                    surrealdb::Action::Create | surrealdb::Action::Update => {
                        tracing::info!(
                            "📨 Agent {} received message from {:?}{:?}: {}",
                            agent_id,
                            queued_msg.from_agent,
                            queued_msg.from_user,
                            queued_msg.content
                        );

                        // Update the call chain to include this agent
                        queued_msg.add_to_call_chain(agent_id.clone());

                        // Convert QueuedMessage to Message
                        let mut message = if let Some(_from_agent) = &queued_msg.from_agent {
                            // Message from another agent
                            Message::agent(queued_msg.content.clone())
                        } else if let Some(from_user) = &queued_msg.from_user {
                            // Message from a user
                            let mut msg = Message::user(queued_msg.content.clone());
                            msg.owner_id = Some(from_user.clone());
                            msg
                        } else {
                            // System message or unknown source
                            Message::system(queued_msg.content.clone())
                        };

                        // Add metadata if present
                        if queued_msg.metadata != serde_json::Value::Null {
                            message.metadata = crate::message::MessageMetadata {
                                custom: queued_msg.metadata.clone(),
                                ..Default::default()
                            };
                        }

                        // Mark message as read immediately to prevent reprocessing
                        if let Err(e) =
                            crate::db::ops::mark_message_as_read(&db, &queued_msg.id).await
                        {
                            crate::log_error!("Failed to mark message as read", e);
                        }

                        // Process the message directly
                        // Try to upgrade the weak reference to process the message
                        if let Some(agent) = agent_weak.upgrade() {
                            tracing::info!(
                                "💬 Agent {} processing message from {:?}{:?}",
                                agent_id,
                                queued_msg.from_agent,
                                queued_msg.from_user
                            );

                            // Call process_message through the Agent trait
                            match agent.process_message(message).await {
                                Ok(response) => {
                                    tracing::debug!(
                                        "✅ Agent {} successfully processed incoming message, response has {} content parts",
                                        agent_id,
                                        response.content.len()
                                    );

                                    // If there's a response with actual content, we might want to send it back
                                    // For now, just log it
                                    for content in &response.content {
                                        match content {
                                            MessageContent::Text(text) => {
                                                tracing::info!("📤 Agent response: {}", text);
                                            }
                                            _ => {
                                                tracing::debug!(
                                                    "📤 Agent response contains non-text content"
                                                );
                                            }
                                        }
                                    }
                                }
                                Err(e) => {
                                    tracing::error!(
                                        "❌ Agent {} failed to process incoming message: {}",
                                        agent_id,
                                        e
                                    );
                                }
                            }
                        } else {
                            tracing::warn!(
                                "⚠️ Agent {} has been dropped, cannot process incoming message",
                                agent_id
                            );
                            // Exit the monitoring task since the agent is gone
                            break;
                        }
                    }
                    _ => {
                        tracing::debug!("Ignoring action {:?} for message queue", action);
                    }
                }
            }

            tracing::warn!(
                "⚠️ Message monitoring task exiting for agent {} - stream ended",
                agent_id
            );
        });

        Ok(())
    }

    /// Start background task to sync agent stats updates with UI
    pub async fn start_stats_sync(&self) -> Result<()> {
        let db = self.db.clone();
        let (agent_id, metadata, history) = {
            let context = self.context.read().await;
            (
                context.handle.agent_id.clone(),
                Arc::clone(&context.metadata),
                Arc::clone(&context.history),
            )
        };

        tokio::spawn(async move {
            let stream = match crate::db::ops::subscribe_to_agent_stats(&db, &agent_id).await {
                Ok(stream) => stream,
                Err(e) => {
                    crate::log_error!("Failed to subscribe to agent stats updates", e);
                    return;
                }
            };

            tracing::debug!("📊 Agent {} stats sync task started", agent_id);

            tokio::pin!(stream);
            while let Some((action, agent_record)) = stream.next().await {
                match action {
                    surrealdb::Action::Update => {
                        tracing::debug!(
                            "📊 Agent {} received stats update - messages: {}, tool calls: {}",
                            agent_id,
                            agent_record.total_messages,
                            agent_record.total_tool_calls
                        );

                        // Update metadata with the latest stats
                        let mut meta = metadata.write().await;
                        meta.total_messages = agent_record.total_messages;
                        meta.total_tool_calls = agent_record.total_tool_calls;
                        meta.last_active = agent_record.last_active;

                        // Update compression events if the message summary changed
                        let history_read = history.read().await;
                        if agent_record.message_summary != history_read.message_summary {
                            meta.compression_events += 1;
                        }
                        drop(history_read);

                        // Update history if message summary changed
                        if agent_record.message_summary.is_some() {
                            let mut history_write = history.write().await;
                            history_write.message_summary = agent_record.message_summary;
                        }
                    }
                    _ => {
                        tracing::debug!("Ignoring action {:?} for agent stats", action);
                    }
                }
            }

            tracing::warn!(
                "⚠️ Stats sync task exiting for agent {} - stream ended",
                agent_id
            );
        });

        Ok(())
    }

    /// Compress messages and persist archival to database
    pub async fn compress_messages_if_needed(&self) -> Result<()> {
        // Check if compression is needed
        let needs_compression = {
            let context = self.context.read().await;
            let history = context.history.read().await;
            history.messages.len() > context.context_config.max_context_messages
        };

        if !needs_compression {
            return Ok(());
        }

        // Get compression strategy and create compressor
        let compression_strategy = {
            let context = self.context.read().await;
            let history = context.history.read().await;
            history.compression_strategy.clone()
        };

        // Create a wrapper for the model provider that can be passed to the compressor
        #[derive(Debug)]
        struct ModelProviderWrapper<M: ModelProvider> {
            model: Arc<RwLock<M>>,
        }

        #[async_trait]
        impl<M: ModelProvider> ModelProvider for ModelProviderWrapper<M> {
            fn name(&self) -> &str {
                "compression_wrapper"
            }

            async fn complete(
                &self,
                options: &crate::model::ResponseOptions,
                request: crate::message::Request,
            ) -> crate::Result<crate::message::Response> {
                let model = self.model.read().await;
                model.complete(options, request).await
            }

            async fn list_models(&self) -> crate::Result<Vec<crate::model::ModelInfo>> {
                let model = self.model.read().await;
                model.list_models().await
            }

            async fn supports_capability(
                &self,
                model: &str,
                capability: crate::model::ModelCapability,
            ) -> bool {
                let provider = self.model.read().await;
                provider.supports_capability(model, capability).await
            }

            async fn count_tokens(&self, model: &str, content: &str) -> crate::Result<usize> {
                let provider = self.model.read().await;
                provider.count_tokens(model, content).await
            }
        }

        let model_wrapper = ModelProviderWrapper {
            model: self.model.clone(),
        };

        let compressor = crate::context::MessageCompressor::new(compression_strategy)
            .with_model_provider(Box::new(model_wrapper));

        // Get messages to compress
        let messages_to_compress = {
            let context = self.context.read().await;
            let history = context.history.read().await;
            history.messages.clone()
        };

        // Perform compression
        let max_context_messages = {
            let context = self.context.read().await;
            context.context_config.max_context_messages
        };
        let compression_result = compressor
            .compress(messages_to_compress, max_context_messages)
            .await?;

        // Collect archived message IDs
        let archived_ids: Vec<crate::MessageId> = compression_result
            .archived_messages
            .iter()
            .map(|msg| msg.id.clone())
            .collect();

        // Apply compression to state
        {
            let context = self.context.read().await;
            let mut history = context.history.write().await;

            // Move compressed messages to archive
            history
                .archived_messages
                .extend(compression_result.archived_messages);

            // Update active messages
            history.messages = compression_result.active_messages;

            // Update or append to summary
            if let Some(new_summary) = compression_result.summary {
                if let Some(existing_summary) = &mut history.message_summary {
                    *existing_summary = format!("{}\n\n{}", existing_summary, new_summary);
                } else {
                    history.message_summary = Some(new_summary.clone());
                }

                // Also update the agent record's message summary in background
                let db = self.db.clone();
                let agent_id = context.handle.agent_id.clone();
                let summary = new_summary;
                tokio::spawn(async move {
                    let query = r#"
                        UPDATE agent SET
                            message_summary = $summary,
                            updated_at = time::now()
                        WHERE id = $id
                    "#;

                    if let Err(e) = db
                        .query(query)
                        .bind(("id", surrealdb::RecordId::from(agent_id)))
                        .bind(("summary", summary))
                        .await
                    {
                        crate::log_error!("Failed to update agent message summary", e);
                    }
                });
            }

            history.last_compression = chrono::Utc::now();
        }

        // Update metadata
        {
            let context = self.context.read().await;
            let mut metadata = context.metadata.write().await;
            metadata.compression_events += 1;
        }

        // Persist archived message IDs to database in background
        if !archived_ids.is_empty() {
            let db = self.db.clone();
            let agent_id = {
                let context = self.context.read().await;
                context.handle.agent_id.clone()
            };
            tokio::spawn(async move {
                if let Err(e) =
                    crate::db::ops::archive_agent_messages(&db, &agent_id, &archived_ids).await
                {
                    crate::log_error!("Failed to archive messages in database", e);
                } else {
                    tracing::debug!(
                        "Successfully archived {} messages for agent {} in database",
                        archived_ids.len(),
                        agent_id
                    );
                }
            });
        }

        Ok(())
    }

    /// Persist memory changes to database
    async fn persist_memory_changes(&self) -> Result<()> {
        let context = self.context.read().await;
        let memory = &context.handle.memory;

        // Get the lists of blocks that need persistence
        let new_blocks = memory.get_new_blocks();
        let dirty_blocks = memory.get_dirty_blocks();

        tracing::debug!(
            "Persisting memory changes: {} new blocks, {} dirty blocks",
            new_blocks.len(),
            dirty_blocks.len()
        );

        // Handle new blocks - need to create and attach to agent
        for block_id in &new_blocks {
            if let Some(block) = memory
                .get_all_blocks()
                .into_iter()
                .find(|b| &b.id == block_id)
            {
                tracing::debug!("Creating new memory block {} in database", block.label);

                // Store the new block
                let stored = block.store_with_relations(&self.db).await?;

                // Attach to agent
                match ops::attach_memory_to_agent(
                    &self.db,
                    &context.handle.agent_id.clone(),
                    &stored.id,
                    block.permission,
                )
                .await
                {
                    Ok(_) => {
                        tracing::debug!("Attached memory block {} to agent", stored.label);
                    }
                    Err(e) => {
                        // This shouldn't happen for new blocks
                        tracing::warn!(
                            "Failed to attach new memory block {} to agent: {}",
                            stored.label,
                            e
                        );
                        return Err(e.into());
                    }
                }
            }
        }

        // Handle dirty blocks - just need to update
        for block_id in &dirty_blocks {
            if let Some(block) = memory
                .get_all_blocks()
                .into_iter()
                .find(|b| &b.id == block_id)
            {
                tracing::debug!("Updating dirty memory block {} in database", block.label);

                // Update the block (store_with_relations will upsert)
                block.store_with_relations(&self.db).await?;
            }
        }

        // Clear the tracking sets after successful persistence
        memory.clear_new_blocks();
        memory.clear_dirty_blocks();

        Ok(())
    }

    /// Add messages to context and persist to database
    async fn persist_response_messages(
        &self,
        response: &Response,
        agent_id: &AgentId,
    ) -> Result<()> {
        let response_messages = {
            let context = self.context.read().await;
            Message::from_response(response, &context.handle.agent_id)
        };

        for message in response_messages {
            if !message.content.is_empty() {
                tracing::debug!(
                    "Adding message with role: {:?}, content type: {:?}",
                    message.role,
                    match &message.content {
                        MessageContent::Text(_) => "Text",
                        MessageContent::Parts(_) => "Parts",
                        MessageContent::ToolCalls(_) => "ToolCalls",
                        MessageContent::ToolResponses(_) => "ToolResponses",
                    }
                );

                // Add the message to context
                {
                    let context = self.context.read().await;
                    context.add_message(message.clone()).await;
                }

                // Also persist to DB
                let _ = crate::db::ops::persist_agent_message(
                    &self.db,
                    agent_id,
                    &message,
                    crate::message::MessageRelationType::Active,
                )
                .await
                .inspect_err(|e| {
                    crate::log_error!("Failed to persist response message", e);
                });
            }
        }

        Ok(())
    }

    /// Process a message and stream responses as they happen
    pub async fn process_message_stream(
        self: Arc<Self>,
        message: Message,
    ) -> Result<impl Stream<Item = ResponseEvent>> {
        use tokio_stream::wrappers::ReceiverStream;

        let (tx, rx) = tokio::sync::mpsc::channel(100);

        // Clone what we need for the spawned task
        let agent_id = self.id();
        let db = self.db.clone();
        let context = self.context.clone();
        let model = self.model.clone();
        let chat_options = self.chat_options.clone();
        let self_clone = self.clone();

        // Spawn the processing task
        tokio::spawn(async move {
            // Helper to send events
            let send_event = |event: ResponseEvent| {
                let tx = tx.clone();
                async move {
                    let _ = tx.send(event).await;
                }
            };

            // Capture the incoming message ID for the completion event
            let incoming_message_id = message.id.clone();

            // Update state and persist message
            {
                let mut ctx = context.write().await;
                ctx.handle.state = AgentState::Processing;
                let _ = crate::db::ops::persist_agent_message(
                    &db,
                    &agent_id,
                    &message,
                    crate::message::MessageRelationType::Active,
                )
                .await
                .inspect_err(|e| {
                    crate::log_error!("Failed to persist incoming message", e);
                });
                ctx.add_message(message).await;
            }

            // Build memory context
            let memory_context = match context.read().await.build_context().await {
                Ok(ctx) => ctx,
                Err(e) => {
                    send_event(ResponseEvent::Error {
                        message: format!("Failed to build context: {}", e),
                        recoverable: false,
                    })
                    .await;
                    return;
                }
            };

            // Create request
            let request = Request {
                system: Some(vec![memory_context.system_prompt.clone()]),
                messages: memory_context.messages,
                tools: Some(memory_context.tools),
            };

            let options = chat_options
                .read()
                .await
                .clone()
                .expect("should have options or default");

            // Get response from model
            let response = {
                let model = model.read().await;
                match model.complete(&options, request).await {
                    Ok(resp) => resp,
                    Err(e) => {
                        send_event(ResponseEvent::Error {
                            message: format!("Model error: {}", e),
                            recoverable: false,
                        })
                        .await;
                        return;
                    }
                }
            };

            let mut current_response = response;

            loop {
                let has_unpaired_tool_calls = current_response.has_unpaired_tool_calls();
                if has_unpaired_tool_calls {
                    // Emit reasoning if present
                    if let Some(reasoning) = &current_response.reasoning {
                        send_event(ResponseEvent::ReasoningChunk {
                            text: reasoning.clone(),
                            is_final: true,
                        })
                        .await;
                    }
                    // Build the processed response manually since we already executed tools
                    let mut processed_response = current_response.clone();
                    processed_response.content.clear();

                    // Use peekable iterator to properly handle tool call/response pairing
                    let mut content_iter = current_response.content.iter().peekable();

                    while let Some(content) = content_iter.next() {
                        match content {
                            MessageContent::ToolCalls(calls) => {
                                // Always include the tool calls
                                processed_response
                                    .content
                                    .push(MessageContent::ToolCalls(calls.clone()));

                                // Check if next item is already tool responses (provider-handled tools)
                                let next_is_responses = content_iter
                                    .peek()
                                    .map(|next| matches!(next, MessageContent::ToolResponses(_)))
                                    .unwrap_or(false);

                                if next_is_responses {
                                    // Provider already included responses, pass them through
                                    // TODO: check if some of the tool calls here are ours, execute those,
                                    // then append out responses to them.

                                    if let Some(next_content) = content_iter.next() {
                                        processed_response.content.push(next_content.clone());
                                    }
                                } else {
                                    // We need to add our executed tool responses
                                    let mut our_responses = Vec::new();

                                    if !calls.is_empty() {
                                        send_event(ResponseEvent::ToolCalls {
                                            calls: calls.clone(),
                                        })
                                        .await;
                                        // Collect responses matching these tool calls
                                        for call in calls {
                                            send_event(ResponseEvent::ToolCallStarted {
                                                call_id: call.call_id.clone(),
                                                fn_name: call.fn_name.clone(),
                                                args: call.fn_arguments.clone(),
                                            })
                                            .await;
                                            if check_heartbeat_request(&call.fn_arguments) {
                                                tracing::debug!(
                                                    "💓 Heartbeat requested by tool {} (call_id: {})",
                                                    call.fn_name,
                                                    call.call_id
                                                );

                                                let heartbeat_req = HeartbeatRequest {
                                                    agent_id: agent_id.clone(),
                                                    tool_name: call.fn_name.clone(),
                                                    tool_call_id: call.call_id.clone(),
                                                };

                                                // Try to send, but don't block if the channel is full
                                                match self.heartbeat_sender.try_send(heartbeat_req)
                                                {
                                                    Ok(()) => {
                                                        tracing::debug!(
                                                            "✅ Heartbeat request sent"
                                                        );
                                                    }
                                                    Err(e) => {
                                                        tracing::warn!(
                                                            "⚠️ Failed to send heartbeat request: {}",
                                                            e
                                                        );
                                                    }
                                                }
                                            }

                                            // Execute tool using the context method
                                            let ctx = context.read().await;
                                            let tool_response = ctx
                                                .process_tool_call(&call)
                                                .await
                                                .unwrap_or_else(|e| {
                                                    Some(ToolResponse {
                                                        call_id: call.call_id.clone(),
                                                        content: format!(
                                                            "Error executing tool: {}",
                                                            e
                                                        ),
                                                    })
                                                });
                                            if let Some(tool_response) = tool_response {
                                                let tool_result = if tool_response
                                                    .content
                                                    .starts_with("Error:")
                                                {
                                                    Err(tool_response.content.clone())
                                                } else {
                                                    Ok(tool_response.content.clone())
                                                };

                                                send_event(ResponseEvent::ToolCallCompleted {
                                                    call_id: call.call_id.clone(),
                                                    result: tool_result,
                                                })
                                                .await;

                                                our_responses.push(tool_response);
                                            }
                                        }
                                        // Add our tool responses if we have any
                                        if !our_responses.is_empty() {
                                            processed_response
                                                .content
                                                .push(MessageContent::ToolResponses(our_responses));
                                        }
                                    }
                                }
                            }
                            MessageContent::ToolResponses(_) => {
                                // Ignore them here, because we have already handled them
                                // Either they were pre-existing and added when we peeked at them,
                                // or we literally just added them after executing the tools.
                            }
                            MessageContent::Text(text) => {
                                send_event(ResponseEvent::TextChunk {
                                    text: text.clone(),
                                    is_final: true,
                                })
                                .await;
                                processed_response.content.push(content.clone());
                            }
                            MessageContent::Parts(parts) => {
                                for part in parts {
                                    match part {
                                        ContentPart::Text(text) => {
                                            send_event(ResponseEvent::TextChunk {
                                                text: text.clone(),
                                                is_final: true,
                                            })
                                            .await;
                                        }
                                        ContentPart::Image {
                                            content_type,
                                            source,
                                        } => {
                                            let source_string = match source {
                                                ImageSource::Url(url) => url.clone(),
                                                ImageSource::Base64(_) => {
                                                    "base64-encoded image".to_string()
                                                }
                                            };
                                            send_event(ResponseEvent::TextChunk {
                                                text: format!(
                                                    "Image ({}): {}",
                                                    content_type, source_string
                                                ),
                                                is_final: true,
                                            })
                                            .await;
                                        }
                                    }
                                }

                                processed_response.content.push(content.clone());
                            }
                        }
                    }

                    // Persist any memory changes from tool execution
                    if let Err(e) = self_clone.persist_memory_changes().await {
                        send_event(ResponseEvent::Error {
                            message: format!("Failed to persist response: {}", e),
                            recoverable: true,
                        })
                        .await;
                    }

                    // Persist response
                    if let Err(e) = self_clone
                        .persist_response_messages(&processed_response, &agent_id)
                        .await
                    {
                        send_event(ResponseEvent::Error {
                            message: format!("Failed to persist response: {}", e),
                            recoverable: true,
                        })
                        .await;
                    }

                    // Only continue if there are unpaired tool calls that need responses
                    if !has_unpaired_tool_calls {
                        break;
                    }

                    // Get next response
                    let context_lock = context.read().await;
                    let memory_context = match context_lock.build_context().await {
                        Ok(ctx) => ctx,
                        Err(e) => {
                            send_event(ResponseEvent::Error {
                                message: format!("Failed to rebuild context: {}", e),
                                recoverable: false,
                            })
                            .await;
                            break;
                        }
                    };
                    drop(context_lock);

                    let request_with_tools = Request {
                        system: Some(vec![memory_context.system_prompt.clone()]),
                        messages: memory_context.messages.clone(),
                        tools: Some(memory_context.tools),
                    };

                    current_response = {
                        let model = model.read().await;
                        match model.complete(&options, request_with_tools).await {
                            Ok(resp) => resp,
                            Err(e) => {
                                send_event(ResponseEvent::Error {
                                    message: format!("Model error in continuation: {}", e),
                                    recoverable: false,
                                })
                                .await;
                                tracing::error!(
                                    "offending request:\n{:?}",
                                    memory_context.messages
                                );
                                break;
                            }
                        }
                    };

                    if current_response.num_tool_calls() == 0 {
                        // Persist any memory changes from tool execution
                        if let Err(e) = self_clone.persist_memory_changes().await {
                            send_event(ResponseEvent::Error {
                                message: format!("Failed to persist response: {}", e),
                                recoverable: true,
                            })
                            .await;
                        }

                        // Persist response
                        if let Err(e) = self_clone
                            .persist_response_messages(&current_response, &agent_id)
                            .await
                        {
                            send_event(ResponseEvent::Error {
                                message: format!("Failed to persist response: {}", e),
                                recoverable: true,
                            })
                            .await;
                        }
                        break;
                    }
                } else {
                    // Emit reasoning if present
                    if let Some(reasoning) = &current_response.reasoning {
                        send_event(ResponseEvent::ReasoningChunk {
                            text: reasoning.clone(),
                            is_final: true,
                        })
                        .await;
                    }
                    // No tool calls, emit final content
                    for content in &current_response.content {
                        match content {
                            MessageContent::Text(text) => {
                                send_event(ResponseEvent::TextChunk {
                                    text: text.clone(),
                                    is_final: true,
                                })
                                .await;
                            }
                            MessageContent::Parts(parts) => {
                                for part in parts {
                                    match part {
                                        ContentPart::Text(text) => {
                                            send_event(ResponseEvent::TextChunk {
                                                text: text.clone(),
                                                is_final: true,
                                            })
                                            .await;
                                        }
                                        ContentPart::Image {
                                            content_type,
                                            source,
                                        } => {
                                            let source_string = match source {
                                                ImageSource::Url(url) => url.clone(),
                                                ImageSource::Base64(_) => {
                                                    "base64-encoded image".to_string()
                                                }
                                            };
                                            send_event(ResponseEvent::TextChunk {
                                                text: format!(
                                                    "Image ({}): {}",
                                                    content_type, source_string
                                                ),
                                                is_final: true,
                                            })
                                            .await;
                                        }
                                    }
                                }
                            }
                            _ => {}
                        }
                    }

                    // Persist final response
                    let persist_result = self_clone
                        .persist_response_messages(&current_response, &agent_id)
                        .await;
                    if let Err(e) = persist_result {
                        send_event(ResponseEvent::Error {
                            message: format!("Failed to persist final response: {}", e),
                            recoverable: true,
                        })
                        .await;
                    }

                    break;
                }
            }

            // Update stats and complete
            let stats = context.read().await.get_stats().await;
            let _ = crate::db::ops::update_agent_stats(&db, agent_id.clone(), &stats).await;

            // Reset state
            {
                let mut ctx = context.write().await;
                ctx.handle.state = AgentState::Ready;
            }

            // Send completion event with the incoming message ID
            send_event(ResponseEvent::Complete {
                message_id: incoming_message_id,
                metadata: current_response.metadata,
            })
            .await;
        });

        Ok(ReceiverStream::new(rx))
    }
}

impl<C, M, E> std::fmt::Debug for DatabaseAgent<C, M, E>
where
    C: surrealdb::Connection + Clone,
    M: ModelProvider,
    E: EmbeddingProvider,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Can't await in Debug, so we'll use try_read
        if let Ok(context) = self.context.try_read() {
            f.debug_struct("DatabaseAgent")
                .field("agent_id", &context.handle.agent_id)
                .field("name", &context.handle.name)
                .field("agent_type", &context.handle.agent_type)
                .field("user_id", &self.user_id)
                .field("state", &context.handle.state)
                .field("has_embeddings", &self.embeddings.is_some())
                .field("memory_blocks", &context.handle.memory.list_blocks().len())
                .finish()
        } else {
            f.debug_struct("DatabaseAgent")
                .field("user_id", &self.user_id)
                .field("has_embeddings", &self.embeddings.is_some())
                .field("<locked>", &"...")
                .finish()
        }
    }
}

#[async_trait]
impl<C, M, E> Agent for DatabaseAgent<C, M, E>
where
    C: surrealdb::Connection + std::fmt::Debug + 'static + Clone,
    M: ModelProvider + 'static,
    E: EmbeddingProvider + 'static,
{
    fn id(&self) -> AgentId {
        // These methods are synchronous, so we need to store these values outside the lock
        // For now, we'll panic if we can't get the lock - this should be rare
        futures::executor::block_on(async { self.context.read().await.handle.agent_id.clone() })
    }

    fn name(&self) -> String {
        futures::executor::block_on(async { self.context.read().await.handle.name.clone() })
    }

    fn agent_type(&self) -> AgentType {
        futures::executor::block_on(async { self.context.read().await.handle.agent_type.clone() })
    }

    async fn process_message(self: Arc<Self>, message: Message) -> Result<Response> {
        use crate::message::ResponseMetadata;
        use futures::StreamExt;

        let mut stream = DatabaseAgent::process_message_stream(self.clone(), message).await?;

        // Collect all events from the stream
        let mut content = Vec::new();
        let mut reasoning: Option<String> = None;
        let mut metadata = ResponseMetadata::default();
        let mut has_error = false;
        let mut error_message = String::new();

        while let Some(event) = stream.next().await {
            match event {
                ResponseEvent::TextChunk { text, is_final: _ } => {
                    // Append text to response
                    if let Some(MessageContent::Text(existing_text)) = content.last_mut() {
                        existing_text.push_str(&text);
                    } else {
                        content.push(MessageContent::Text(text));
                    }
                }
                ResponseEvent::ReasoningChunk { text, is_final: _ } => {
                    // Append reasoning
                    if let Some(ref mut existing_reasoning) = reasoning {
                        existing_reasoning.push_str(&text);
                    } else {
                        reasoning = Some(text);
                    }
                }
                ResponseEvent::ToolCalls { calls } => {
                    content.push(MessageContent::ToolCalls(calls));
                }
                ResponseEvent::ToolResponses { responses } => {
                    content.push(MessageContent::ToolResponses(responses));
                }
                ResponseEvent::Complete {
                    message_id: _,
                    metadata: event_metadata,
                } => {
                    metadata = event_metadata;
                }
                ResponseEvent::Error {
                    message,
                    recoverable,
                } => {
                    if !recoverable {
                        has_error = true;
                        error_message = message;
                    }
                }
                _ => {} // Ignore other events for backward compatibility
            }
        }

        if has_error {
            Err(CoreError::model_error(
                error_message,
                "",
                genai::Error::NoChatResponse {
                    model_iden: metadata.model_iden,
                },
            ))
        } else {
            Ok(Response {
                content,
                reasoning,
                metadata,
            })
        }
    }

    async fn get_memory(&self, key: &str) -> Result<Option<MemoryBlock>> {
        let context = self.context.read().await;
        Ok(context.handle.memory.get_block(key).map(|b| b.clone()))
    }

    async fn update_memory(&self, key: &str, memory: MemoryBlock) -> Result<()> {
        let agent_id = {
            let context = self.context.read().await;
            context.handle.agent_id.clone()
        };
        tracing::debug!("🔧 Agent {} updating memory key '{}'", agent_id, key);

        // Update in context immediately - upsert the complete block
        {
            let context = self.context.write().await;
            context.handle.memory.upsert_block(key, memory.clone())?;
            context.metadata.write().await.last_active = Utc::now();
        };

        // Persist memory block synchronously during initial setup
        // TODO: Consider spawning background task for updates after agent is initialized

        crate::db::ops::persist_agent_memory(
            &self.db,
            agent_id,
            &memory,
            crate::memory::MemoryPermission::ReadWrite, // Agent has full access to its own memory
        )
        .await?;

        Ok(())
    }

    async fn execute_tool(
        &self,
        tool_name: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value> {
        // Get the tool from the registry
        let (tool, agent_id) = {
            let context = self.context.read().await;
            let tool = context.tools.get(tool_name).ok_or_else(|| {
                CoreError::tool_not_found(
                    tool_name,
                    context
                        .tools
                        .list_tools()
                        .iter()
                        .map(|s| s.to_string())
                        .collect(),
                )
            })?;
            (tool.clone(), context.handle.agent_id.clone())
        };

        // Record start time
        let start_time = std::time::Instant::now();
        let created_at = chrono::Utc::now();

        // Execute the tool
        let result = tool.execute(params.clone()).await;

        // Calculate duration
        let duration_ms = start_time.elapsed().as_millis() as i64;

        // Update tool call count in metadata
        {
            let context = self.context.read().await;
            let mut metadata = context.metadata.write().await;
            metadata.total_tool_calls += 1;
        }

        let tool_call = schema::ToolCall {
            id: crate::id::ToolCallId::generate(),
            agent_id,
            tool_name: tool_name.to_string(),
            parameters: params,
            result: result.as_ref().unwrap_or(&serde_json::json!(null)).clone(),
            error: result.as_ref().err().map(|e| e.to_string()),
            duration_ms,
            created_at,
        };

        let _ = crate::db::ops::create_entity(&self.db, &tool_call)
            .await
            .inspect_err(|e| {
                crate::log_error!("Failed to persist tool call", e);
            });

        // Return the result
        result
    }

    async fn list_memory_keys(&self) -> Result<Vec<CompactString>> {
        let context = self.context.read().await;
        Ok(context.handle.memory.list_blocks())
    }

    async fn share_memory_with(
        &self,
        memory_key: &str,
        target_agent_id: AgentId,
        access_level: crate::memory::MemoryPermission,
    ) -> Result<()> {
        // First verify the memory block exists
        let memory_block = {
            let context = self.context.read().await;
            context
                .handle
                .memory
                .get_block(memory_key)
                .ok_or_else(|| {
                    CoreError::memory_not_found(
                        &context.handle.agent_id,
                        memory_key,
                        context.handle.memory.list_blocks(),
                    )
                })?
                .clone()
        };

        // Create the memory relation for the target agent
        let db = self.db.clone();
        let memory_id = memory_block.id.clone();
        let relation = AgentMemoryRelation {
            id: RelationId::nil(),
            in_id: target_agent_id.clone(),
            out_id: memory_id.clone(),
            access_level,
            created_at: chrono::Utc::now(),
        };

        // Persist the sharing relationship in background
        let target_id_clone = target_agent_id.clone();
        let _handle = tokio::spawn(async move {
            if let Err(e) = crate::db::ops::create_relation_typed(&db, &relation).await {
                crate::log_error!("Failed to share memory block", e);
            } else {
                tracing::debug!(
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
        let db = self.db.clone();
        let agent_id = {
            let context = self.context.read().await;
            context.handle.agent_id.clone()
        };

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
        let context = self.context.read().await;
        match context.build_context().await {
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
                vec![context.context_config.base_instructions.clone()]
            }
        }
    }

    async fn available_tools(&self) -> Vec<Box<dyn DynamicTool>> {
        let context = self.context.read().await;
        context.tools.get_all_as_dynamic()
    }

    async fn state(&self) -> AgentState {
        let context = self.context.read().await;
        context.handle.state
    }

    /// Set the agent's state
    async fn set_state(&self, state: AgentState) -> Result<()> {
        // Update the state in the context
        {
            let mut context = self.context.write().await;
            context.handle.state = state;
            let mut metadata = context.metadata.write().await;
            metadata.last_active = chrono::Utc::now();
        }

        // Persist the state update in background
        let db = self.db.clone();
        let agent_id = {
            let context = self.context.read().await;
            context.handle.agent_id.clone()
        };

        let _ = db
            .query("UPDATE agent SET state = $state, last_active = $last_active WHERE id = $id")
            .bind(("id", surrealdb::RecordId::from(&agent_id)))
            .bind(("state", serde_json::to_value(&state).unwrap()))
            .bind(("last_active", surrealdb::Datetime::from(chrono::Utc::now())))
            .await
            .inspect_err(|e| {
                crate::log_error!("Failed to persist agent state update", e);
            });

        Ok(())
    }

    async fn register_endpoint(
        &self,
        name: String,
        endpoint: Arc<dyn crate::context::message_router::MessageEndpoint>,
    ) -> Result<()> {
        let context = self.context.read().await;

        // Get the message router from the handle
        let router = context.handle.message_router().ok_or_else(|| {
            crate::CoreError::ToolExecutionFailed {
                tool_name: "register_endpoint".to_string(),
                cause: "Message router not configured for this agent".to_string(),
                parameters: serde_json::json!({ "name": name }),
            }
        })?;

        // Register the endpoint
        router.register_endpoint(name, endpoint).await;
        Ok(())
    }

    async fn set_default_user_endpoint(
        &self,
        endpoint: Arc<dyn crate::context::message_router::MessageEndpoint>,
    ) -> Result<()> {
        let mut context = self.context.write().await;
        if let Some(router) = &mut context.handle.message_router {
            router.set_default_user_endpoint(endpoint).await;
            Ok(())
        } else {
            Err(crate::CoreError::AgentInitFailed {
                agent_type: self.agent_type().as_str().to_string(),
                cause: "Message router not initialized".to_string(),
            })
        }
    }

    async fn process_message_stream(
        self: Arc<Self>,
        message: Message,
    ) -> Result<Box<dyn Stream<Item = ResponseEvent> + Send + Unpin>>
    where
        Self: 'static,
    {
        // Call our actual streaming implementation
        let stream = DatabaseAgent::process_message_stream(self, message).await?;
        Ok(Box::new(stream) as Box<dyn Stream<Item = ResponseEvent> + Send + Unpin>)
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
