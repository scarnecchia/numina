//! Agent state management for building stateful agents on stateless protocols
//!
//! This module provides the infrastructure for managing agent state between
//! conversations, including message history, memory persistence, and context rebuilding.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

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

/// Represents the complete state of an agent
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentContext {
    /// Unique identifier for this agent
    pub agent_id: AgentId,

    /// Type of agent (e.g., memgpt_agent, custom)
    pub agent_type: AgentType,

    /// Agent's memory store
    pub memory: Memory,

    /// Current message history
    pub messages: Vec<Message>,

    /// Compressed/archived messages
    pub archived_messages: Vec<Message>,

    /// Summary of compressed messages
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message_summary: Option<String>,

    /// Tools available to this agent
    #[serde(skip)]
    pub tools: Arc<ToolRegistry>,

    /// Configuration for context building
    pub context_config: ContextConfig,

    /// Compression strategy for messages
    pub compression_strategy: CompressionStrategy,

    /// Metadata about the agent state
    pub metadata: AgentContextMetadata,
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
        tools: Arc<ToolRegistry>,
        context_config: ContextConfig,
    ) -> Self {
        Self {
            agent_id,
            agent_type,
            memory,
            messages: Vec::new(),
            archived_messages: Vec::new(),
            message_summary: None,
            tools,
            context_config,
            compression_strategy: CompressionStrategy::default(),
            metadata: AgentContextMetadata {
                created_at: Utc::now(),
                last_active: Utc::now(),
                total_messages: 0,
                total_tool_calls: 0,
                context_rebuilds: 0,
                compression_events: 0,
            },
        }
    }

    /// Build the current context for this agent
    pub async fn build_context(&mut self) -> Result<MemoryContext> {
        self.metadata.last_active = Utc::now();
        self.metadata.context_rebuilds += 1;

        // Check if we need to compress messages
        if self.messages.len() > self.context_config.max_context_messages {
            self.compress_messages().await?;
        }

        // Get memory blocks
        let memory_blocks = self.memory.get_all_blocks();

        // Build context
        let context = ContextBuilder::new(self.agent_id.clone(), self.context_config.clone())
            .with_memory_blocks(memory_blocks)
            .with_tools_from_registry(&self.tools)
            .with_messages(self.messages.clone())
            .build()?;

        Ok(context)
    }

    /// Add a new message to the state
    pub fn add_message(&mut self, message: Message) {
        // Count tool calls if this is an assistant message
        if message.role.is_assistant() {
            self.metadata.total_tool_calls += message.tool_call_count();
        }

        self.messages.push(message);
        self.metadata.total_messages += 1;
        self.metadata.last_active = Utc::now();
    }

    /// Process a chat response and update state
    pub fn process_response(&mut self, response: Response) -> Result<()> {
        // Extract content and create assistant message
        if let Some(content) = response.content {
            let message = Message::agent(content);
            self.add_message(message);
        }

        Ok(())
    }

    /// Update a memory block
    pub fn update_memory_block(&mut self, label: &str, new_value: String) -> Result<()> {
        self.memory.update_block_value(label, new_value)?;
        self.metadata.last_active = Utc::now();
        Ok(())
    }

    /// Append to a memory block
    pub fn append_to_memory_block(&mut self, label: &str, content: &str) -> Result<()> {
        let current = self.memory.get_block(label).ok_or_else(|| {
            CoreError::memory_not_found(&self.agent_id, label, self.memory.list_blocks())
        })?;

        let new_value = format!("{}\n{}", current.value, content);
        self.memory.update_block_value(label, new_value)?;
        self.metadata.last_active = Utc::now();
        Ok(())
    }

    /// Replace content in a memory block
    pub fn replace_in_memory_block(
        &mut self,
        label: &str,
        old_content: &str,
        new_content: &str,
    ) -> Result<()> {
        let current = self.memory.get_block(label).ok_or_else(|| {
            CoreError::memory_not_found(&self.agent_id, label, self.memory.list_blocks())
        })?;

        let new_value = current.value.replace(old_content, new_content);
        self.memory.update_block_value(label, new_value)?;
        self.metadata.last_active = Utc::now();
        Ok(())
    }

    /// Compress messages using the configured strategy
    async fn compress_messages(&mut self) -> Result<()> {
        let compressor = MessageCompressor::new(self.compression_strategy.clone());

        let result = compressor
            .compress(
                self.messages.clone(),
                self.context_config.max_context_messages,
            )
            .await?;

        self.apply_compression_result(result)
    }

    /// Apply compression result to state
    fn apply_compression_result(&mut self, result: CompressionResult) -> Result<()> {
        // Move compressed messages to archive
        self.archived_messages.extend(result.archived_messages);

        // Update active messages
        self.messages = result.active_messages;

        // Update or append to summary
        if let Some(new_summary) = result.summary {
            if let Some(existing_summary) = &mut self.message_summary {
                *existing_summary = format!("{}\n\n{}", existing_summary, new_summary);
            } else {
                self.message_summary = Some(new_summary);
            }
        }

        self.metadata.compression_events += 1;
        Ok(())
    }

    /// Search through message history (including archived)
    pub fn search_messages(&self, query: &str, page: usize, page_size: usize) -> Vec<&Message> {
        let query_lower = query.to_lowercase();

        // Search through all messages (active + archived)
        let all_messages: Vec<&Message> = self
            .archived_messages
            .iter()
            .chain(self.messages.iter())
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
            matching_messages[start..end].to_vec()
        } else {
            Vec::new()
        }
    }

    /// Get agent statistics
    pub fn get_stats(&self) -> AgentStats {
        AgentStats {
            total_messages: self.metadata.total_messages,
            active_messages: self.messages.len(),
            archived_messages: self.archived_messages.len(),
            total_tool_calls: self.metadata.total_tool_calls,
            memory_blocks: self.memory.list_blocks().len(),
            compression_events: self.metadata.compression_events,
            uptime: Utc::now() - self.metadata.created_at,
            last_active: self.metadata.last_active,
        }
    }

    /// Create a checkpoint of the current state
    pub fn checkpoint(&self) -> StateCheckpoint {
        StateCheckpoint {
            agent_id: self.agent_id.clone(),
            timestamp: Utc::now(),
            messages: self.messages.clone(),
            memory_snapshot: self.memory.clone(),
            metadata: self.metadata.clone(),
        }
    }

    /// Restore from a checkpoint
    pub fn restore_from_checkpoint(&mut self, checkpoint: StateCheckpoint) -> Result<()> {
        if checkpoint.agent_id != self.agent_id {
            return Err(CoreError::AgentInitFailed {
                agent_type: format!("{:?}", self.agent_type),
                cause: Box::new(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "Checkpoint is for a different agent",
                )),
            });
        }

        self.messages = checkpoint.messages;
        self.memory = checkpoint.memory_snapshot;
        self.metadata = checkpoint.metadata;
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
    tools: Option<Arc<ToolRegistry>>,
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
    pub fn with_tools(mut self, tools: Arc<ToolRegistry>) -> Self {
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
    pub fn build(self) -> Result<AgentContext> {
        // Create memory
        let mut memory = Memory::new();
        for (label, value, description) in self.memory_blocks {
            memory.create_block(&label, &value)?;
            if let Some(desc) = description {
                if let Some(block) = memory.get_block_mut(&label) {
                    block.description = Some(desc);
                }
            }
        }

        // Get or create default tools
        let tools = self.tools.unwrap_or_else(|| Arc::new(ToolRegistry::new()));

        // Get or create default context config
        let context_config = self.context_config.unwrap_or_default();

        // Create state
        let mut state = AgentContext::new(
            self.agent_id,
            self.agent_type,
            memory,
            tools,
            context_config,
        );

        // Set compression strategy if provided
        if let Some(strategy) = self.compression_strategy {
            state.compression_strategy = strategy;
        }

        // Add initial messages
        for message in self.initial_messages {
            state.add_message(message);
        }

        Ok(state)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_agent_state_builder() {
        let id = AgentId::generate();
        let state = AgentContextBuilder::new(id.clone(), AgentType::Generic)
            .with_memory_block(
                "persona",
                "I am a helpful AI assistant",
                Some("Agent persona"),
            )
            .with_memory_block("human", "The user's name is unknown", None::<String>)
            .build()
            .unwrap();

        assert_eq!(state.agent_id, id);
        assert_eq!(state.memory.list_blocks().len(), 2);
        assert!(state.memory.get_block("persona").is_some());
    }

    #[test]
    fn test_message_search() {
        let mut state = AgentContextBuilder::new(AgentId::generate(), AgentType::Generic)
            .build()
            .unwrap();

        state.add_message(Message::user("Hello, how are you?"));
        state.add_message(Message::agent("I'm doing well, thank you!"));
        state.add_message(Message::user("What's the weather like?"));

        let results = state.search_messages("weather", 0, 10);
        assert_eq!(results.len(), 1);
    }
}
