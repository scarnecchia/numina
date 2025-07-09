//! Agent state management for building stateful agents on stateless protocols
//!
//! This module provides the infrastructure for managing agent state between
//! conversations, including message history, memory persistence, and context rebuilding.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

use std::sync::Arc;

use crate::{
    AgentId, AgentType, CoreError, Result,
    memory::Memory,
    message::{Message, Response},
    tool::ToolRegistry,
};

use super::{
    CompressionResult, CompressionStrategy, ContextBuilder, ContextConfig, MemoryContext,
    MessageCompressor, genai_ext::ChatRoleExt,
};

/// Cheap handle to agent internals that built-in tools can hold
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentHandle {
    pub agent_id: AgentId,
    pub memory: Memory, // already cheap (DashMap)
                        // TODO: Add message_sender when we implement it
}

/// Message history that needs locking
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MessageHistory {
    pub messages: Vec<Message>,
    pub archived_messages: Vec<Message>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message_summary: Option<String>,
    pub compression_strategy: CompressionStrategy,
    pub last_compression: DateTime<Utc>,
}

impl MessageHistory {
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
pub struct AgentContext {
    /// Cheap, frequently accessed stuff
    pub handle: AgentHandle,

    /// Type of agent (e.g., memgpt_agent, custom)
    pub agent_type: AgentType,

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

impl AgentContext {
    /// Create a new agent state
    pub fn new(
        agent_id: AgentId,
        agent_type: AgentType,
        memory: Memory,
        tools: ToolRegistry,
        context_config: ContextConfig,
    ) -> Self {
        let handle = AgentHandle { agent_id, memory };

        Self {
            handle,
            agent_type,
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
    pub fn handle(&self) -> AgentHandle {
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
        let context =
            ContextBuilder::new(self.handle.agent_id.clone(), self.context_config.clone())
                .with_memory_blocks(memory_blocks)
                .with_tools_from_registry(&self.tools)
                .with_messages(history.messages.clone())
                .build()?;

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
    pub fn process_response(&self, _response: &Response) -> Result<()> {
        Ok(())
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

        let new_value = format!("{}\n{}", current.content, content);
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

        let new_value = current.content.replace(old_content, new_content);
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

        self.apply_compression_result(&mut history, result)?;
        history.last_compression = Utc::now();
        self.metadata.write().await.compression_events += 1;
        Ok(())
    }

    /// Apply compression result to state
    fn apply_compression_result(
        &self,
        history: &mut MessageHistory,
        result: CompressionResult,
    ) -> Result<()> {
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

        Ok(())
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
                agent_type: format!("{:?}", self.agent_type),
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
