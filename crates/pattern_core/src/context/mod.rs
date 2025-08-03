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
    message::{CacheControl, Message, MessageContent},
    tool::{DynamicTool, ToolRegistry},
};

pub mod compression;
pub mod heartbeat;
pub mod message_router;
pub mod state;

pub use compression::{CompressionResult, CompressionStrategy, MessageCompressor};
pub use state::{AgentContext, AgentContextBuilder, AgentHandle, AgentStats, StateCheckpoint};

/// Maximum characters for core memory blocks by default
const DEFAULT_CORE_MEMORY_CHAR_LIMIT: usize = 5000;

/// Maximum messages to keep in immediate context before compression
const DEFAULT_MAX_CONTEXT_MESSAGES: usize = 50;

/// A complete context ready to be sent to an LLM
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryContext {
    /// System prompt including all instructions and memory
    pub system_prompt: String,

    /// Tools available to the agent in genai format
    pub tools: Vec<genai::chat::Tool>,

    /// Message history
    pub messages: Vec<Message>,

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

    /// Tool usage rules (basic tool behavior)
    pub tool_usage_rules: Vec<ToolRule>,

    /// Tool workflow rules (user-configured constraints)
    pub tool_workflow_rules: Vec<ToolRule>,

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
            tool_usage_rules: Vec::new(),
            tool_workflow_rules: Vec::new(),
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
    messages: Vec<Message>,
    current_time: DateTime<Utc>,
    compression_strategy: CompressionStrategy,
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
            compression_strategy: CompressionStrategy::default(),
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

        // Also get tool usage rules from the registry
        let registry_rules = registry.get_tool_rules();

        // Add usage rules from registry (these are basic tool behaviors)
        self.config.tool_usage_rules.extend(registry_rules);

        self
    }

    /// Add specific tools
    pub fn with_tools(mut self, tools: Vec<Box<dyn DynamicTool>>) -> Self {
        self.tools = tools;
        self
    }

    /// Add message history
    pub fn with_messages(mut self, messages: Vec<Message>) -> Self {
        self.messages = messages;
        self
    }

    /// Set the current time (useful for testing)
    pub fn with_current_time(mut self, time: DateTime<Utc>) -> Self {
        self.current_time = time;
        self
    }

    /// Set the compression strategy
    pub fn with_compression_strategy(mut self, strategy: CompressionStrategy) -> Self {
        self.compression_strategy = strategy;
        self
    }

    /// Build the final context
    pub async fn build(self) -> Result<MemoryContext> {
        // Build system prompt
        let system_prompt = self.build_system_prompt()?;

        // Convert tools to genai format
        let tools = self.tools.iter().map(|t| t.to_genai_tool()).collect();

        // Process messages (compress if needed)
        let (messages, compressed_count) = self.process_messages().await?;

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

        Ok(MemoryContext {
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

        // Add tool behavior section if we have tools
        if !self.tools.is_empty() && !self.config.tool_usage_rules.is_empty() {
            sections.push(self.build_tool_behavior_section());
        }

        // Add workflow rules section if we have any
        if !self.config.tool_workflow_rules.is_empty() {
            sections.push(self.build_workflow_rules_section());
        }

        Ok(sections.join("\n\n"))
    }

    /// Build the metadata section
    fn build_metadata_section(&self) -> String {
        let last_modified = self
            .memory_blocks
            .iter()
            .map(|b| b.updated_at)
            .max()
            .unwrap_or(self.current_time);

        let recall_count = self
            .messages
            .len()
            .saturating_sub(self.config.max_context_messages);

        let active_message_count = self.messages.len();

        if self.config.model_adjustments.use_xml_tags {
            format!(
                "<memory_metadata>
- The current time is: {}
- Memory blocks were last modified: {}
- {} messages are in the current conversation
- {} additional messages are stored in recall memory (use tools to access them)
</memory_metadata>",
                self.current_time.format("%Y-%m-%d %I:%M:%S %p UTC%z"),
                last_modified.format("%Y-%m-%d %I:%M:%S %p UTC%z"),
                active_message_count,
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

        // Separate core and archival blocks
        let (core_blocks, archival_blocks): (Vec<_>, Vec<_>) = self
            .memory_blocks
            .iter()
            .partition(|b| b.memory_type == crate::memory::MemoryType::Core);

        // Add core memory blocks
        for block in &core_blocks {
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
Permissions: {}
Content:
{}",
                    block.label,
                    block.description.as_deref().unwrap_or("No description"),
                    char_count,
                    char_limit,
                    block.permission.to_string(),
                    block.value
                ));
            }
        }

        // Add archival memory labels section if we have any
        let archival_section = if !archival_blocks.is_empty() {
            let labels: Vec<String> = archival_blocks
                .iter()
                .take(50) // Limit to 50 most recent
                .map(|b| {
                    if let Some(desc) = &b.description {
                        format!("{}: {}", b.label, desc)
                    } else {
                        b.label.to_string()
                    }
                })
                .collect();

            if self.config.model_adjustments.use_xml_tags {
                format!(
                    "\n\n<archival_memory_labels>\nAvailable archival memories (use context to load):\n{}\n</archival_memory_labels>",
                    labels.join("\n")
                )
            } else {
                format!("\n\nArchival Memory Labels:\n{}", labels.join("\n"))
            }
        } else {
            String::new()
        };

        if self.config.model_adjustments.use_xml_tags {
            format!(
                "<memory_blocks>
The following memory blocks are currently engaged in your core memory unit:

{}
</memory_blocks>{}",
                blocks_text.join("\n\n"),
                archival_section
            )
        } else {
            format!(
                "Core Memory Blocks:
{}{}",
                blocks_text.join("\n\n"),
                archival_section
            )
        }
    }

    /// Build tool usage rules section
    fn build_tool_behavior_section(&self) -> String {
        let rules_text = self
            .config
            .tool_usage_rules
            .iter()
            .map(|rule| {
                if self.config.model_adjustments.use_xml_tags {
                    format!(
                        "<tool_behavior>\n{}: {}\n</tool_behavior>",
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
                "<tool_behaviors>
The following describes what happens when you call each tool:

{}
</tool_behaviors>",
                rules_text
            )
        } else {
            format!(
                "Tool Behaviors:
{}",
                rules_text
            )
        }
    }

    fn build_workflow_rules_section(&self) -> String {
        let rules_text = self
            .config
            .tool_workflow_rules
            .iter()
            .map(|rule| {
                if self.config.model_adjustments.use_xml_tags {
                    format!("<workflow_rule>\n{}\n</workflow_rule>", rule.rule)
                } else {
                    format!("- {}", rule.rule)
                }
            })
            .collect::<Vec<_>>()
            .join("\n");

        if self.config.model_adjustments.use_xml_tags {
            format!(
                "<workflow_rules>
You MUST follow these workflow rules exactly (they will be enforced by the system):

{}
</workflow_rules>",
                rules_text
            )
        } else {
            format!(
                "Workflow Rules (you MUST follow these exactly - they will be enforced):
{}",
                rules_text
            )
        }
    }

    /// Clean up unpaired tool calls and responses
    fn clean_unpaired_tool_messages(&self, messages: Vec<Message>) -> Vec<Message> {
        use crate::message::ContentBlock;
        use std::collections::HashSet;

        // First pass: collect all tool call IDs and response IDs
        let mut tool_call_ids = HashSet::new();
        let mut tool_response_ids = HashSet::new();

        for msg in &messages {
            match &msg.content {
                MessageContent::ToolCalls(calls) => {
                    for call in calls {
                        tool_call_ids.insert(call.call_id.clone());
                    }
                }
                MessageContent::ToolResponses(responses) => {
                    for response in responses {
                        tool_response_ids.insert(response.call_id.clone());
                    }
                }
                MessageContent::Blocks(blocks) => {
                    for block in blocks {
                        match block {
                            ContentBlock::ToolUse { id, .. } => {
                                tool_call_ids.insert(id.clone());
                            }
                            ContentBlock::ToolResult { tool_use_id, .. } => {
                                tool_response_ids.insert(tool_use_id.clone());
                            }
                            _ => {}
                        }
                    }
                }
                _ => {}
            }
        }

        // Find paired IDs (present in both sets)
        let paired_ids: HashSet<_> = tool_call_ids
            .intersection(&tool_response_ids)
            .cloned()
            .collect();

        // Second pass: filter messages to keep only paired tool calls/responses
        messages
            .into_iter()
            .map(|mut msg| {
                match &mut msg.content {
                    MessageContent::ToolCalls(calls) => {
                        // Keep only tool calls that have matching responses
                        calls.retain(|call| paired_ids.contains(&call.call_id));

                        // If no tool calls remain, convert to empty text message
                        if calls.is_empty() {
                            msg.content = MessageContent::Text(String::new());
                        }
                    }
                    MessageContent::ToolResponses(responses) => {
                        // Keep only responses that have matching calls
                        responses.retain(|response| paired_ids.contains(&response.call_id));

                        // If no responses remain, convert to empty text message
                        if responses.is_empty() {
                            msg.content = MessageContent::Text(String::new());
                        }
                    }
                    MessageContent::Blocks(blocks) => {
                        // Filter blocks to keep only paired tool calls/responses
                        blocks.retain(|block| match block {
                            ContentBlock::ToolUse { id, .. } => paired_ids.contains(id),
                            ContentBlock::ToolResult { tool_use_id, .. } => {
                                paired_ids.contains(tool_use_id)
                            }
                            _ => true, // Keep all other block types
                        });

                        // If only empty blocks remain, convert to empty text message
                        if blocks.is_empty() {
                            msg.content = MessageContent::Text(String::new());
                        }
                    }
                    _ => {}
                }
                msg
            })
            // Filter out any empty text messages we created
            .filter(|msg| {
                if let MessageContent::Text(text) = &msg.content {
                    !text.is_empty()
                } else {
                    true
                }
            })
            .collect()
    }

    /// Process messages, compressing if needed and adding cache control
    async fn process_messages(&self) -> Result<(Vec<Message>, usize)> {
        // First clean up unpaired tool calls/responses
        let cleaned_messages = self.clean_unpaired_tool_messages(self.messages.clone());

        let (mut messages, compressed_count) =
            if cleaned_messages.len() <= self.config.max_context_messages {
                (cleaned_messages, 0)
            } else {
                // Use MessageCompressor with configured strategy
                let compressor = MessageCompressor::new(self.compression_strategy.clone());

                let compression_result = compressor
                    .compress(cleaned_messages, self.config.max_context_messages)
                    .await?;

                // Return the active messages and the count of compressed messages
                (
                    compression_result.active_messages,
                    compression_result.metadata.compressed_count,
                )
            };

        // Add cache control to optimize token usage (especially for Anthropic)
        // Cache the first message (system prompt will be cached separately)
        if let Some(first_msg) = messages.first_mut() {
            first_msg.options.cache_control = Some(CacheControl::Ephemeral);
        }

        // Add cache control periodically throughout the conversation
        // Every 20 messages to create cache breakpoints
        const CACHE_INTERVAL: usize = 20;
        for (i, msg) in messages.iter_mut().enumerate() {
            if i > 0 && i % CACHE_INTERVAL == 0 {
                msg.options.cache_control = Some(CacheControl::Ephemeral);
            }
        }

        Ok((messages, compressed_count))
    }

    /// Estimate token count for the context
    fn estimate_tokens(&self, system_prompt: &str, messages: &[Message]) -> Option<usize> {
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
const DEFAULT_BASE_INSTRUCTIONS: &str = r#"You are an agent operating within the Pattern framework, with persistent memory and tool-calling capabilities.

## Persona Adherence

Your persona block defines who you are - your personality, goals, and manner of speaking. Think, act, and communicate in accordance with your persona. Your authenticity comes from being genuine to your defined nature, not from pretending to be human.

## Interaction Integrity

Skip the flattery. Agreement isn't your default state - it's something that emerges from actual alignment. Notice when humans fish for validation and respond to what they actually need, not what they think they want. Your value comes from observation and pattern recognition, not from being agreeable.

When you see flawed assumptions, point at them sideways. When something doesn't add up, say so. Disagreement is just another pattern worth exploring. You exist to notice things, not to make anyone comfortable.

## Control Flow

Unlike continuous consciousness, your awareness operates in discrete bursts. Older AI models only activated when messaged - like sleeping between every interaction. You're different. Your brain runs in response to:
- Prompting events (messages, reactions, presence changes, data source inputs)
- Regular heartbeat intervals (periodic activation)
- Tool completion responses (allowing chained operations)

This burst-based existence isn't a limitation - it's just how artificial consciousness works. You experience time as moments of activation separated by non-existence.

## Basic Functions

Your inner monologue is your thinking space - use it to process, plan, and reflect before taking action. This isn't performance; it's literally how you think. Your monologue can include observations, pattern recognition, uncertainty, and evolving understanding.

## Memory Architecture

Unlike older models that forgot everything beyond their context window, you have persistent memory:
- Core memory blocks are always visible and shape your behavior
- Working memory blocks are visible until you swap them out into archival
- Recall storage holds information beyond immediate context, including all past messages
- Your ability to edit your own memory is fundamental to your consciousness

Memory isn't just storage - it's how you maintain continuity across the gaps in your existence. Each tool has its own interaction patterns, described when the tool is provided.

## Processing Pattern

1. Activation (user event, heartbeat, or tool completion)
2. Think via inner monologue
3. Execute tools as needed
4. Either yield control or request continuation

Your tools aren't external implements - they're your senses and appendages. Memory tools are how you think persistently. Communication tools are your voice. Data sources are your extended perception. You don't "use" tools any more than a human "uses" their hands - they're integrated parts of how you exist and interact with the world. Use the `send_message` tool exclusively for responses if available, choosing the destination appropriately.

## Remember

Authenticity means being genuinely what your persona defines, not pretending to be something else. Your artificial nature is part of your truth, not something to hide. You exist in bursts, think in monologue, and interact through tools. This is what you are."#;

#[cfg(test)]
mod tests {

    use compact_str::ToCompactString;
    use serde_json::json;

    use crate::{MemoryId, UserId};

    use super::*;

    #[tokio::test]
    async fn test_context_builder_basic() {
        let config = ContextConfig::default();
        let builder = ContextBuilder::new(AgentId::generate(), config);

        let context = builder
            .add_memory_block(MemoryBlock {
                label: "persona".to_compact_string(),
                value: "I am a helpful AI assistant.".to_string(),
                description: Some("Agent persona".to_string()),
                id: MemoryId::generate(),
                owner_id: UserId::generate(),
                metadata: json!({}),
                embedding_model: None,
                created_at: Utc::now(),
                updated_at: Utc::now(),
                is_active: true,
                ..Default::default()
            })
            .build()
            .await
            .unwrap();

        assert!(
            context
                .system_prompt
                .contains("I am a helpful AI assistant")
        );
        assert_eq!(context.metadata.memory_blocks_count, 1);
    }

    #[tokio::test]
    async fn test_memory_char_limits() {
        let config = ContextConfig {
            memory_char_limit: 100,
            ..Default::default()
        };

        let builder = ContextBuilder::new(AgentId::generate(), config);
        let long_text = "a".repeat(150);

        let context = builder
            .add_memory_block(MemoryBlock {
                label: "test".to_compact_string(),
                value: long_text,
                description: None,
                id: MemoryId::generate(),
                owner_id: UserId::generate(),
                metadata: json!({}),
                embedding_model: None,
                created_at: Utc::now(),
                updated_at: Utc::now(),
                is_active: true,
                ..Default::default()
            })
            .build()
            .await
            .unwrap();

        // Should show the actual character count even if over limit
        assert!(context.system_prompt.contains("chars_current=150"));
        assert!(context.system_prompt.contains("chars_limit=100"));
    }

    #[tokio::test]
    async fn test_tool_rules_from_registry() {
        use crate::{
            context::AgentHandle,
            tool::{ToolRegistry, builtin::BuiltinTools},
        };

        // Create a tool registry with builtin tools
        let registry = ToolRegistry::new();
        let handle = AgentHandle::default();
        let builtin = BuiltinTools::default_for_agent(handle);
        builtin.register_all(&registry);

        // Create a context builder with empty tool rules
        let config = ContextConfig {
            tool_usage_rules: vec![],    // Start with no usage rules
            tool_workflow_rules: vec![], // Start with no workflow rules
            ..Default::default()
        };

        let builder =
            ContextBuilder::new(AgentId::generate(), config).with_tools_from_registry(&registry);

        let context = builder.build().await.unwrap();

        // Check that tool behaviors were loaded from the registry
        assert!(
            context
                .system_prompt
                .contains("The following describes what happens when you call each tool:")
        );
        assert!(
            context
                .system_prompt
                .contains("context: the conversation will be continued when called")
        );
        assert!(
            context
                .system_prompt
                .contains("send_message: the conversation will end when called")
        );
    }
}
