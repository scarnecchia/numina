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
    message::{CacheControl, Message, MessageBatch},
    tool::{DynamicTool, ToolRegistry},
};

pub mod compression;
pub mod endpoints;
pub mod heartbeat;
pub mod message_router;
pub mod state;

pub use compression::{CompressionResult, CompressionStrategy, MessageCompressor};
pub use state::{AgentContext, AgentContextBuilder, AgentHandle, AgentStats, StateCheckpoint};

/// Maximum characters for core memory blocks by default
const DEFAULT_CORE_MEMORY_CHAR_LIMIT: usize = 10000;

/// Maximum messages to keep in immediate context before compression
const DEFAULT_MAX_CONTEXT_MESSAGES: usize = 200;

/// A complete context ready to be sent to an LLM
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryContext {
    /// System prompt including all instructions and memory
    pub system_prompt: String,

    /// Tools available to the agent in genai format
    pub tools: Vec<genai::chat::Tool>,

    /// Message batches (complete and current)
    pub batches: Vec<MessageBatch>,

    /// Current batch being processed (if any)
    pub current_batch_id: Option<crate::agent::SnowflakePosition>,

    /// Summary of archived/compressed batches (if any)
    pub archive_summary: Option<String>,

    /// Metadata about the context (for debugging/logging)
    pub metadata: ContextMetadata,
}

impl MemoryContext {
    /// Get all messages as a flat list for sending to LLM
    /// Filters out incomplete batches except for the current one
    pub fn get_messages_for_request(&self) -> Vec<Message> {
        let mut messages = Vec::new();

        for batch in &self.batches {
            // Include batch if it's complete OR if it's the current batch being processed
            let should_include =
                batch.is_complete || self.current_batch_id.as_ref() == Some(&batch.id);

            if should_include {
                messages.extend(batch.messages.clone());
            }
        }

        messages
    }

    /// Temporary compatibility method - returns all messages
    pub fn messages(&self) -> Vec<Message> {
        self.get_messages_for_request()
    }

    /// Get the total number of messages across all batches
    pub fn len(&self) -> usize {
        self.batches.iter().map(|b| b.len()).sum()
    }

    /// Check if there are any messages
    pub fn is_empty(&self) -> bool {
        self.batches.is_empty() || self.batches.iter().all(|b| b.is_empty())
    }

    /// Convert this context into a Request for the LLM
    pub fn into_request(&self) -> crate::message::Request {
        let mut messages = Vec::new();

        // Add archive summary as initial system message if present
        if let Some(summary) = &self.archive_summary {
            messages.push(crate::message::Message::system(format!(
                "Previous conversation summary:\n{}",
                summary
            )));
        }

        // Then add all the regular messages from batches
        messages.extend(self.get_messages_for_request());

        crate::message::Request {
            system: Some(vec![self.system_prompt.clone()]),
            messages,
            tools: Some(self.tools.clone()),
        }
    }
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

    /// Maximum tokens before compression (if set, overrides model defaults)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_context_tokens: Option<usize>,

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
            max_context_tokens: Some(128000),
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
            max_context_tokens: None,
            token_multiplier: 1.5, // Rough estimate: 1 token ≈ 0.75 words
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
    batches: Vec<MessageBatch>,
    current_time: DateTime<Utc>,
    compression_strategy: CompressionStrategy,
    archive_summary: Option<String>,
}

impl ContextBuilder {
    /// Create a new context builder
    pub fn new(agent_id: AgentId, config: ContextConfig) -> Self {
        Self {
            agent_id,
            config,
            memory_blocks: Vec::new(),
            tools: Vec::new(),
            batches: Vec::new(),
            current_time: Utc::now(),
            compression_strategy: CompressionStrategy::default(),
            archive_summary: None,
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

    /// Add message batches
    pub fn with_batches(mut self, batches: Vec<MessageBatch>) -> Self {
        self.batches = batches;
        self.batches.sort_by_key(|b| b.id.0);
        self
    }

    /// Add a single batch
    pub fn add_batch(mut self, batch: MessageBatch) -> Self {
        self.batches.push(batch);
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

    /// Set the archive summary for compressed/archived batches
    pub fn with_archive_summary(mut self, summary: Option<String>) -> Self {
        self.archive_summary = summary;
        self
    }

    /// Build the final context
    pub async fn build(
        self,
        current_batch_id: Option<crate::agent::SnowflakePosition>,
    ) -> Result<MemoryContext> {
        // Build system prompt
        let system_prompt = self.build_system_prompt()?;

        // Convert tools to genai format
        let tools = self.tools.iter().map(|t| t.to_genai_tool()).collect();

        // Process batches (compress if needed, filter incomplete)
        let (batches, compressed_count) = self.process_batches(current_batch_id).await?;

        // Count total messages for metadata
        let total_message_count: usize = batches.iter().map(|b| b.len()).sum();

        // Estimate token count
        let all_messages: Vec<Message> = batches.iter().flat_map(|b| b.messages.clone()).collect();
        let total_tokens_estimate = self.estimate_tokens(&system_prompt, &all_messages);

        // Build metadata
        let metadata = ContextMetadata {
            agent_id: self.agent_id,
            build_time: Utc::now(),
            total_tokens_estimate,
            message_count: total_message_count,
            compressed_message_count: compressed_count,
            memory_blocks_count: self.memory_blocks.len(),
            tools_count: self.tools.len(),
        };

        Ok(MemoryContext {
            system_prompt,
            tools,
            batches,
            current_batch_id,
            archive_summary: self.archive_summary.clone(),
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

        // Count total messages across all batches
        let total_message_count: usize = self.batches.iter().map(|b| b.len()).sum();
        let recall_count = total_message_count.saturating_sub(self.config.max_context_messages);
        let active_message_count = total_message_count.min(self.config.max_context_messages);

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
            .partition(|b| b.memory_type != crate::memory::MemoryType::Archival);

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
- permissions={}
- type={}
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
                    block.permission,
                    block.memory_type,
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
Type: {}
Content:
{}",
                    block.label,
                    block.description.as_deref().unwrap_or("No description"),
                    char_count,
                    char_limit,
                    block.permission.to_string(),
                    block.memory_type,
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
The following memory blocks are currently engaged in your main memory unit:

{}
</memory_blocks>{}",
                blocks_text.join("\n\n"),
                archival_section
            )
        } else {
            format!(
                "Core and Working Memory Blocks:
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

    /// Process batches, filtering incomplete and compressing if needed
    async fn process_batches(
        &self,
        current_batch_id: Option<crate::agent::SnowflakePosition>,
    ) -> Result<(Vec<MessageBatch>, usize)> {
        let mut filtered_batches = Vec::new();

        tracing::debug!(
            "Processing {} batches for context. Current batch ID: {:?}",
            self.batches.len(),
            current_batch_id
        );

        // Filter batches: include complete batches and current batch
        for batch in &self.batches {
            let should_include = batch.is_complete || current_batch_id.as_ref() == Some(&batch.id);

            if should_include {
                filtered_batches.push(batch.clone());
            }
        }

        tracing::debug!(
            "After filtering: {} batches included (from {} total)",
            filtered_batches.len(),
            self.batches.len()
        );

        // Log the included batch IDs and message counts
        if !filtered_batches.is_empty() {
            let batch_info: Vec<String> = filtered_batches
                .iter()
                .take(10) // First 10 for brevity
                .map(|b| format!("{} ({} msgs)", b.id, b.len()))
                .collect();
            tracing::debug!("Included batches (first 10): {:?}", batch_info);
        }

        // Count total messages
        let total_messages: usize = filtered_batches.iter().map(|b| b.len()).sum();

        // Apply compression if needed
        let (final_batches, compressed_count) =
            if total_messages <= self.config.max_context_messages {
                (filtered_batches, 0)
            } else {
                // For now, just take the most recent batches that fit
                // TODO: implement smarter compression that respects batch boundaries
                let mut kept_batches = Vec::new();
                let mut message_count = 0;
                let mut compressed = 0;

                // Walk backwards through batches, keeping as many as fit
                for batch in filtered_batches.iter().rev() {
                    let batch_size = batch.len();
                    if message_count + batch_size <= self.config.max_context_messages {
                        kept_batches.push(batch.clone());
                        message_count += batch_size;
                    } else {
                        compressed += batch_size;
                    }
                }

                kept_batches.reverse();
                (kept_batches, compressed)
            };

        // Log final batches after compression
        if compressed_count > 0 {
            tracing::debug!(
                "After compression: {} batches kept with {} messages (compressed {} messages)",
                final_batches.len(),
                final_batches.iter().map(|b| b.len()).sum::<usize>(),
                compressed_count
            );

            let batch_info: Vec<String> = final_batches
                .iter()
                .take(10)
                .map(|b| format!("{} ({} msgs)", b.id, b.len()))
                .collect();
            tracing::debug!(
                "Final batches after compression (first 10): {:?}",
                batch_info
            );
        }

        // Add cache control to messages in batches - max 4 cache points total
        let mut result_batches = Vec::new();
        let total_batches = final_batches.len();
        let mut cache_points_used = 0;
        const MAX_CACHE_POINTS: usize = 4;

        // Calculate which batches should get cache points
        // Strategy: cache early context and recent context
        let cache_positions: Vec<usize> = if total_batches <= MAX_CACHE_POINTS {
            // If we have 4 or fewer batches, cache the first message of each
            (0..total_batches).collect()
        } else {
            // Otherwise, distribute cache points strategically:
            // 1. First batch (early context)
            // 2. 25% through
            // 3. 50% through
            // 4. Most recent batch
            vec![0, total_batches / 4, total_batches / 2, total_batches - 1]
        };

        for (batch_idx, mut batch) in final_batches.into_iter().enumerate() {
            // Only add cache control if this batch is in our cache positions
            // and we haven't exceeded the limit
            if cache_positions.contains(&batch_idx)
                && cache_points_used < MAX_CACHE_POINTS
                && !batch.messages.is_empty()
            {
                batch.messages[0].options.cache_control = Some(CacheControl::Ephemeral);
                cache_points_used += 1;
            }

            result_batches.push(batch);
        }
        result_batches.sort_by_key(|b| b.id.0);

        Ok((result_batches, compressed_count))
    }

    /// Estimate token count for the context
    fn estimate_tokens(&self, system_prompt: &str, messages: &[Message]) -> Option<usize> {
        if let Some(max_tokens) = self.config.model_adjustments.max_context_tokens {
            // Very rough estimation
            let system_tokens = system_prompt.chars().count() / 4;
            let message_tokens: usize = messages
                .iter()
                .map(|m| m.estimate_tokens()) // Convert back to rough char count
                .sum();

            let total_tokens = system_tokens + message_tokens;
            let estimated_tokens =
                (total_tokens as f32 * self.config.model_adjustments.token_multiplier) as usize;

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
            .build(None)
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
            .build(None)
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

        let context = builder.build(None).await.unwrap();

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
