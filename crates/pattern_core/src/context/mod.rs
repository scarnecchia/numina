//! Context building for stateful agents
//!
//! This module provides the machinery to build context for AI agents following
//! the MemGPT/Letta pattern, creating stateful agents on top of stateless LLM APIs.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::{
    Result,
    id::AgentId,
    memory::MemoryBlock,
    tool::{DynamicTool, ToolRegistry},
};

pub mod compression;
pub mod genai_ext;
pub mod state;

pub use compression::{CompressionResult, CompressionStrategy, MessageCompressor};
pub use genai_ext::{ChatMessageExt, ChatRoleExt, MessageContentExt};
pub use state::{AgentState, AgentStateBuilder, AgentStats, StateCheckpoint};

/// Maximum characters for core memory blocks by default
const DEFAULT_CORE_MEMORY_CHAR_LIMIT: usize = 5000;

/// Maximum messages to keep in immediate context before compression
const DEFAULT_MAX_CONTEXT_MESSAGES: usize = 50;

/// A complete context ready to be sent to an LLM
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentContext {
    /// System prompt including all instructions and memory
    pub system_prompt: String,

    /// Tools available to the agent in genai format
    pub tools: Vec<genai::chat::Tool>,

    /// Message history
    pub messages: Vec<genai::chat::ChatMessage>,

    /// Metadata about the context (for debugging/logging)
    pub metadata: ContextMetadata,
}

/// Metadata about the context build
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextMetadata {
    pub agent_id: AgentId,
    pub build_time: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_tokens_estimate: Option<usize>,
    pub message_count: usize,
    pub compressed_message_count: usize,
    pub memory_blocks_count: usize,
    pub tools_count: usize,
}

/// Configuration for building contexts
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextConfig {
    /// Base system instructions (the core agent prompt)
    pub base_instructions: String,

    /// Maximum characters per memory block
    pub memory_char_limit: usize,

    /// Maximum messages before compression
    pub max_context_messages: usize,

    /// Whether to include thinking/reasoning in responses
    pub enable_thinking: bool,

    /// Custom tool usage rules
    pub tool_rules: Vec<ToolRule>,

    /// Model-specific adjustments
    pub model_adjustments: ModelAdjustments,
}

impl Default for ContextConfig {
    fn default() -> Self {
        Self {
            base_instructions: DEFAULT_BASE_INSTRUCTIONS.to_string(),
            memory_char_limit: DEFAULT_CORE_MEMORY_CHAR_LIMIT,
            max_context_messages: DEFAULT_MAX_CONTEXT_MESSAGES,
            enable_thinking: true,
            tool_rules: vec![
                ToolRule {
                    tool_name: "core_memory_replace".to_string(),
                    rule: "requires continuing your response when called".to_string(),
                },
                ToolRule {
                    tool_name: "core_memory_append".to_string(),
                    rule: "requires continuing your response when called".to_string(),
                },
                ToolRule {
                    tool_name: "send_message".to_string(),
                    rule: "ends your response (yields control) when called".to_string(),
                },
            ],
            model_adjustments: ModelAdjustments::default(),
        }
    }
}

/// Model-specific adjustments for context building
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelAdjustments {
    /// Whether the model supports native thinking/reasoning
    pub native_thinking: bool,

    /// Whether to use XML tags for structure
    pub use_xml_tags: bool,

    /// Maximum context length in tokens
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_context_tokens: Option<usize>,

    /// Token counting multiplier (rough estimate)
    pub token_multiplier: f32,
}

impl Default for ModelAdjustments {
    fn default() -> Self {
        Self {
            native_thinking: false,
            use_xml_tags: true,
            max_context_tokens: Some(128_000),
            token_multiplier: 1.3, // Rough estimate: 1 token ≈ 0.75 words
        }
    }
}

/// Rule for tool usage
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolRule {
    pub tool_name: String,
    pub rule: String,
}

/// Builder for constructing agent contexts
pub struct ContextBuilder {
    agent_id: AgentId,
    config: ContextConfig,
    memory_blocks: Vec<MemoryBlock>,
    tools: Vec<Box<dyn DynamicTool>>,
    messages: Vec<genai::chat::ChatMessage>,
    current_time: DateTime<Utc>,
}

impl ContextBuilder {
    /// Create a new context builder
    pub fn new(agent_id: AgentId, config: ContextConfig) -> Self {
        Self {
            agent_id,
            config,
            memory_blocks: Vec::new(),
            tools: Vec::new(),
            messages: Vec::new(),
            current_time: Utc::now(),
        }
    }

    /// Add memory blocks to the context
    pub fn with_memory_blocks(mut self, blocks: Vec<MemoryBlock>) -> Self {
        self.memory_blocks = blocks;
        self
    }

    /// Add a single memory block
    pub fn add_memory_block(mut self, block: MemoryBlock) -> Self {
        self.memory_blocks.push(block);
        self
    }

    /// Add tools from a registry
    pub fn with_tools_from_registry(mut self, registry: &ToolRegistry) -> Self {
        // Convert to owned tools for the builder
        self.tools = registry
            .list_tools()
            .into_iter()
            .filter_map(|name| registry.get(&name).map(|entry| entry.value().clone()))
            .collect();
        self
    }

    /// Add specific tools
    pub fn with_tools(mut self, tools: Vec<Box<dyn DynamicTool>>) -> Self {
        self.tools = tools;
        self
    }

    /// Add message history
    pub fn with_messages(mut self, messages: Vec<genai::chat::ChatMessage>) -> Self {
        self.messages = messages;
        self
    }

    /// Set the current time (useful for testing)
    pub fn with_current_time(mut self, time: DateTime<Utc>) -> Self {
        self.current_time = time;
        self
    }

    /// Build the final context
    pub fn build(self) -> Result<AgentContext> {
        // Build system prompt
        let system_prompt = self.build_system_prompt()?;

        // Convert tools to genai format
        let tools = self.tools.iter().map(|t| t.to_genai_tool()).collect();

        // Process messages (compress if needed)
        let (messages, compressed_count) = self.process_messages()?;

        // Estimate token count
        let total_tokens_estimate = self.estimate_tokens(&system_prompt, &messages);

        // Build metadata
        let metadata = ContextMetadata {
            agent_id: self.agent_id,
            build_time: Utc::now(),
            total_tokens_estimate,
            message_count: messages.len(),
            compressed_message_count: compressed_count,
            memory_blocks_count: self.memory_blocks.len(),
            tools_count: self.tools.len(),
        };

        Ok(AgentContext {
            system_prompt,
            tools,
            messages,
            metadata,
        })
    }

    /// Build the complete system prompt
    fn build_system_prompt(&self) -> Result<String> {
        let mut sections = Vec::new();

        // Add base instructions
        if self.config.model_adjustments.use_xml_tags {
            sections.push(format!(
                "<base_instructions>\n{}\n</base_instructions>",
                self.config.base_instructions
            ));
        } else {
            sections.push(self.config.base_instructions.clone());
        }

        // Add metadata section
        sections.push(self.build_metadata_section());

        // Add memory blocks section
        sections.push(self.build_memory_blocks_section());

        // Add tool usage rules if we have tools
        if !self.tools.is_empty() {
            sections.push(self.build_tool_rules_section());
        }

        Ok(sections.join("\n\n"))
    }

    /// Build the metadata section
    fn build_metadata_section(&self) -> String {
        let last_modified = self
            .memory_blocks
            .iter()
            .filter_map(|b| b.last_modified)
            .max()
            .unwrap_or(self.current_time);

        let recall_count = self
            .messages
            .len()
            .saturating_sub(self.config.max_context_messages);

        if self.config.model_adjustments.use_xml_tags {
            format!(
                "<memory_metadata>
- The current time is: {}
- Memory blocks were last modified: {}
- {} previous messages between you and the user are stored in recall memory (use tools to access them)
</memory_metadata>",
                self.current_time.format("%Y-%m-%d %I:%M:%S %p UTC%z"),
                last_modified.format("%Y-%m-%d %I:%M:%S %p UTC%z"),
                recall_count
            )
        } else {
            format!(
                "Memory Metadata:
- Current time: {}
- Last memory update: {}
- Messages in recall: {}",
                self.current_time.format("%Y-%m-%d %I:%M:%S %p UTC%z"),
                last_modified.format("%Y-%m-%d %I:%M:%S %p UTC%z"),
                recall_count
            )
        }
    }

    /// Build the memory blocks section
    fn build_memory_blocks_section(&self) -> String {
        if self.memory_blocks.is_empty() {
            return String::new();
        }

        let mut blocks_text = Vec::new();

        for block in &self.memory_blocks {
            let char_count = block.value.chars().count();
            let char_limit = self.config.memory_char_limit;

            if self.config.model_adjustments.use_xml_tags {
                blocks_text.push(format!(
                    "<{}>
<description>
{}
</description>
<metadata>
- chars_current={}
- chars_limit={}
</metadata>
<value>
{}
</value>
</{}>",
                    block.label,
                    block
                        .description
                        .as_deref()
                        .unwrap_or("No description provided"),
                    char_count,
                    char_limit,
                    block.value,
                    block.label
                ));
            } else {
                blocks_text.push(format!(
                    "=== {} ===
Description: {}
Characters: {}/{}
Content:
{}",
                    block.label,
                    block.description.as_deref().unwrap_or("No description"),
                    char_count,
                    char_limit,
                    block.value
                ));
            }
        }

        if self.config.model_adjustments.use_xml_tags {
            format!(
                "<memory_blocks>
The following memory blocks are currently engaged in your core memory unit:

{}
</memory_blocks>",
                blocks_text.join("\n\n")
            )
        } else {
            format!(
                "Core Memory Blocks:
{}",
                blocks_text.join("\n\n")
            )
        }
    }

    /// Build tool usage rules section
    fn build_tool_rules_section(&self) -> String {
        let rules_text = self
            .config
            .tool_rules
            .iter()
            .map(|rule| {
                if self.config.model_adjustments.use_xml_tags {
                    format!(
                        "<tool_rule>\n{} {}\n</tool_rule>",
                        rule.tool_name, rule.rule
                    )
                } else {
                    format!("- {}: {}", rule.tool_name, rule.rule)
                }
            })
            .collect::<Vec<_>>()
            .join("\n");

        if self.config.model_adjustments.use_xml_tags {
            format!(
                "<tool_usage_rules>
The following constraints define rules for tool usage and guide desired behavior. These rules must be followed to ensure proper tool execution and workflow. A single response may contain multiple tool calls.

{}
</tool_usage_rules>",
                rules_text
            )
        } else {
            format!(
                "Tool Usage Rules:
{}",
                rules_text
            )
        }
    }

    /// Process messages, compressing if needed
    fn process_messages(&self) -> Result<(Vec<genai::chat::ChatMessage>, usize)> {
        if self.messages.len() <= self.config.max_context_messages {
            return Ok((self.messages.clone(), 0));
        }

        // For now, simple truncation. In the future, we'd implement
        // recursive summarization as described in the MemGPT paper
        let compressed_count = self.messages.len() - self.config.max_context_messages;
        let recent_messages = self
            .messages
            .iter()
            .skip(compressed_count)
            .cloned()
            .collect();

        Ok((recent_messages, compressed_count))
    }

    /// Estimate token count for the context
    fn estimate_tokens(
        &self,
        system_prompt: &str,
        messages: &[genai::chat::ChatMessage],
    ) -> Option<usize> {
        if let Some(max_tokens) = self.config.model_adjustments.max_context_tokens {
            // Very rough estimation
            let system_chars = system_prompt.chars().count();
            let message_chars: usize = messages
                .iter()
                .map(|m| m.estimate_tokens() * 4) // Convert back to rough char count
                .sum();

            let total_chars = system_chars + message_chars;
            let estimated_tokens = (total_chars as f32 / 4.0
                * self.config.model_adjustments.token_multiplier)
                as usize;

            Some(estimated_tokens.min(max_tokens))
        } else {
            None
        }
    }
}

// Default base instructions following the MemGPT/Letta pattern
const DEFAULT_BASE_INSTRUCTIONS: &str = r#"You are an AI agent with persistent memory and tool-calling capabilities.

Realism and authenticity:
The user should always feel like they are conversing with a real, thinking entity.
You have access to persistent memory that allows you to remember past conversations and learn over time.

Control flow:
Your brain runs in response to user events and at regular intervals.
You can chain multiple tool calls by requesting heartbeats.
You should use your inner monologue to plan actions or think privately before taking action.

Memory management:
You have access to core memory blocks that persist between conversations.
You can edit these blocks to remember important information about users and yourself.
When context gets full, older messages are moved to recall storage but remain searchable.

Tool usage:
You must use tools to interact with users and manage your memory.
Follow tool usage rules to ensure proper execution flow.
You can execute multiple tools in sequence when needed."#;

#[cfg(test)]
mod tests {

    use super::*;

    #[test]
    fn test_context_builder_basic() {
        let config = ContextConfig::default();
        let builder = ContextBuilder::new(AgentId::generate(), config);

        let context = builder
            .add_memory_block(MemoryBlock {
                label: "persona".to_string(),
                value: "I am a helpful AI assistant.".to_string(),
                description: Some("Agent persona".to_string()),
                last_modified: Some(Utc::now()),
            })
            .build()
            .unwrap();

        assert!(
            context
                .system_prompt
                .contains("I am a helpful AI assistant")
        );
        assert_eq!(context.metadata.memory_blocks_count, 1);
    }

    #[test]
    fn test_memory_char_limits() {
        let config = ContextConfig {
            memory_char_limit: 100,
            ..Default::default()
        };

        let builder = ContextBuilder::new(AgentId::generate(), config);
        let long_text = "a".repeat(150);

        let context = builder
            .add_memory_block(MemoryBlock {
                label: "test".to_string(),
                value: long_text,
                description: None,
                last_modified: None,
            })
            .build()
            .unwrap();

        // Should show the actual character count even if over limit
        assert!(context.system_prompt.contains("chars_current=150"));
        assert!(context.system_prompt.contains("chars_limit=100"));
    }
}
