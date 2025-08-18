//! Message compression strategies for managing context window limits
//!
//! This module implements various strategies for compressing message history
//! when it exceeds the context window, following the MemGPT paper's approach.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

use crate::{
    CoreError, ModelProvider, Result,
    message::{ChatRole, ContentBlock, Message, MessageContent},
};

/// Detect provider from model string
fn detect_provider_from_model(model: &str) -> String {
    let model_lower = model.to_lowercase();

    if model_lower.contains("claude") {
        "anthropic".to_string()
    } else if model_lower.contains("gpt") {
        "openai".to_string()
    } else if model_lower.contains("gemini") {
        "gemini".to_string()
    } else if model_lower.contains("llama") || model_lower.contains("mixtral") {
        "groq".to_string()
    } else if model_lower.contains("command") {
        "cohere".to_string()
    } else if model_lower.contains("deepseek") {
        "deepseek".to_string()
    } else if model_lower.contains("o1") || model_lower.contains("o3") {
        "openai".to_string()
    } else {
        // Default to openai as it's most common
        "openai".to_string()
    }
}

/// Strategy for compressing messages when context is full
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CompressionStrategy {
    /// Simple truncation - keep only the most recent messages
    Truncate { keep_recent: usize },

    /// Recursive summarization as described in MemGPT paper
    RecursiveSummarization {
        /// Number of messages to summarize at a time
        chunk_size: usize,
        /// Model to use for summarization
        summarization_model: String,
    },

    /// Importance-based selection
    ImportanceBased {
        /// Keep this many recent messages
        keep_recent: usize,
        /// Keep this many important messages
        keep_important: usize,
    },

    /// Time-decay based compression
    TimeDecay {
        /// Messages older than this are candidates for compression
        compress_after_hours: f64,
        /// Keep at least this many recent messages
        min_keep_recent: usize,
    },
}

impl Default for CompressionStrategy {
    fn default() -> Self {
        Self::Truncate { keep_recent: 100 }
    }
}

/// Result of message compression
#[derive(Debug, Clone)]
pub struct CompressionResult {
    /// Batches to keep in the active context
    pub active_batches: Vec<crate::message::MessageBatch>,

    /// Summary of compressed batches (if applicable)
    pub summary: Option<String>,

    /// Batches moved to recall storage
    pub archived_batches: Vec<crate::message::MessageBatch>,

    /// Metadata about the compression
    pub metadata: CompressionMetadata,
}

/// Metadata about a compression operation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompressionMetadata {
    pub strategy_used: String,
    pub original_count: usize,
    pub compressed_count: usize,
    pub archived_count: usize,
    pub compression_time: DateTime<Utc>,
    pub estimated_tokens_saved: usize,
}

/// Configuration for importance scoring
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportanceScoringConfig {
    /// Weight for system messages (default: 10.0)
    pub system_weight: f32,
    /// Weight for assistant messages (default: 3.0)
    pub assistant_weight: f32,
    /// Weight for user messages (default: 5.0)
    pub user_weight: f32,
    /// Weight for other messages (default: 1.0)
    pub other_weight: f32,
    /// Maximum recency bonus (default: 5.0)
    pub recency_bonus: f32,
    /// Weight per 100 characters of content (default: 1.0, max 3.0)
    pub content_length_weight: f32,
    /// Bonus for messages with questions (default: 2.0)
    pub question_bonus: f32,
    /// Bonus for messages with tool calls (default: 4.0)
    pub tool_call_bonus: f32,
    /// Additional keywords to boost importance
    pub important_keywords: Vec<String>,
    /// Bonus per important keyword found (default: 1.5)
    pub keyword_bonus: f32,
}

impl Default for ImportanceScoringConfig {
    fn default() -> Self {
        Self {
            system_weight: 10.0,
            assistant_weight: 3.0,
            user_weight: 5.0,
            other_weight: 1.0,
            recency_bonus: 5.0,
            content_length_weight: 1.0,
            question_bonus: 2.0,
            tool_call_bonus: 4.0,
            important_keywords: vec![
                "important".to_string(),
                "remember".to_string(),
                "critical".to_string(),
                "always".to_string(),
                "never".to_string(),
            ],
            keyword_bonus: 1.5,
        }
    }
}

/// Compresses messages using various strategies
pub struct MessageCompressor {
    strategy: CompressionStrategy,
    model_provider: Option<Box<dyn ModelProvider>>,
    scoring_config: ImportanceScoringConfig,
}

impl MessageCompressor {
    /// Create a new message compressor
    pub fn new(strategy: CompressionStrategy) -> Self {
        Self {
            strategy,
            model_provider: None,
            scoring_config: ImportanceScoringConfig::default(),
        }
    }

    /// Set the model provider for strategies that need it
    pub fn with_model_provider(mut self, provider: Box<dyn ModelProvider>) -> Self {
        self.model_provider = Some(provider);
        self
    }

    /// Set custom scoring configuration
    pub fn with_scoring_config(mut self, config: ImportanceScoringConfig) -> Self {
        self.scoring_config = config;
        self
    }

    /// Compress batches according to the configured strategy
    pub async fn compress(
        &self,
        batches: Vec<crate::message::MessageBatch>,
        max_messages: usize,
    ) -> Result<CompressionResult> {
        // Calculate total message count across all batches
        let original_count: usize = batches.iter().map(|b| b.len()).sum();

        if original_count <= max_messages {
            // No compression needed
            return Ok(CompressionResult {
                active_batches: batches,
                summary: None,
                archived_batches: Vec::new(),
                metadata: CompressionMetadata {
                    strategy_used: "none".to_string(),
                    original_count,
                    compressed_count: 0,
                    archived_count: 0,
                    compression_time: Utc::now(),
                    estimated_tokens_saved: 0,
                },
            });
        }

        let result = match &self.strategy {
            CompressionStrategy::Truncate { keep_recent } => {
                self.truncate_messages(batches, *keep_recent)
            }

            CompressionStrategy::RecursiveSummarization {
                chunk_size,
                summarization_model,
            } => {
                self.recursive_summarization(
                    batches,
                    max_messages,
                    *chunk_size,
                    summarization_model,
                )
                .await
            }

            CompressionStrategy::ImportanceBased {
                keep_recent,
                keep_important,
            } => {
                self.importance_based_compression(batches, *keep_recent, *keep_important)
                    .await
            }

            CompressionStrategy::TimeDecay {
                compress_after_hours,
                min_keep_recent,
            } => self.time_decay_compression(batches, *compress_after_hours, *min_keep_recent),
        }?;

        // Validate and fix message sequence for Gemini compatibility
        //result.active_messages = self.ensure_valid_message_sequence(result.active_messages);

        Ok(result)
    }

    /// Check if a message contains tool use blocks
    fn has_tool_use_blocks(&self, message: &Message) -> bool {
        match &message.content {
            MessageContent::Blocks(blocks) => blocks
                .iter()
                .any(|block| matches!(block, ContentBlock::ToolUse { .. })),
            _ => false,
        }
    }

    /// Check if a message contains tool result blocks
    #[allow(dead_code)]
    fn has_tool_result_blocks(&self, message: &Message) -> bool {
        match &message.content {
            MessageContent::Blocks(blocks) => blocks
                .iter()
                .any(|block| matches!(block, ContentBlock::ToolResult { .. })),
            _ => false,
        }
    }

    /// Check if a message contains thinking blocks
    #[allow(dead_code)]
    fn has_thinking_blocks(&self, message: &Message) -> bool {
        match &message.content {
            MessageContent::Blocks(blocks) => blocks.iter().any(|block| {
                matches!(
                    block,
                    ContentBlock::Thinking { .. } | ContentBlock::RedactedThinking { .. }
                )
            }),
            _ => false,
        }
    }

    /// Validate and fix tool call/response ordering to ensure Anthropic compatibility
    /// Anthropic requires tool results to immediately follow tool uses
    #[allow(dead_code)]
    fn validate_and_reorder_tool_calls(&self, messages: Vec<Message>) -> Vec<Message> {
        let mut fixed_messages = Vec::new();
        let mut pending_tool_calls = HashMap::new();
        let mut orphaned_tool_results = HashSet::new();

        // First pass: identify all tool calls and results
        for (i, msg) in messages.iter().enumerate() {
            match &msg.content {
                MessageContent::ToolCalls(calls) => {
                    for call in calls {
                        pending_tool_calls.insert(call.call_id.clone(), (i, msg.clone()));
                    }
                }
                MessageContent::ToolResponses(responses) => {
                    for response in responses {
                        if !pending_tool_calls.contains_key(&response.call_id) {
                            orphaned_tool_results.insert(response.call_id.clone());
                        }
                    }
                }
                MessageContent::Blocks(blocks) => {
                    for block in blocks {
                        match block {
                            ContentBlock::ToolUse { id, .. } => {
                                pending_tool_calls.insert(id.clone(), (i, msg.clone()));
                            }
                            ContentBlock::ToolResult { tool_use_id, .. } => {
                                if !pending_tool_calls.contains_key(tool_use_id) {
                                    orphaned_tool_results.insert(tool_use_id.clone());
                                }
                            }
                            _ => {}
                        }
                    }
                }
                _ => {}
            }
        }

        // Second pass: rebuild message list with proper ordering
        let mut processed_indices = HashSet::new();
        let mut i = 0;

        while i < messages.len() {
            if processed_indices.contains(&i) {
                i += 1;
                continue;
            }

            let msg = &messages[i];

            // Check if this message has tool calls
            let tool_call_ids: Vec<String> = match &msg.content {
                MessageContent::ToolCalls(calls) => {
                    calls.iter().map(|c| c.call_id.clone()).collect()
                }
                MessageContent::Blocks(blocks) => blocks
                    .iter()
                    .filter_map(|b| {
                        if let ContentBlock::ToolUse { id, .. } = b {
                            Some(id.clone())
                        } else {
                            None
                        }
                    })
                    .collect(),
                _ => vec![],
            };

            if !tool_call_ids.is_empty() {
                // This message has tool calls - find corresponding results
                let mut found_results = false;

                // Look for tool results in subsequent messages
                for j in (i + 1)..messages.len() {
                    if processed_indices.contains(&j) {
                        continue;
                    }

                    let result_msg = &messages[j];
                    let result_ids: Vec<String> = match &result_msg.content {
                        MessageContent::ToolResponses(responses) => {
                            responses.iter().map(|r| r.call_id.clone()).collect()
                        }
                        MessageContent::Blocks(blocks) => blocks
                            .iter()
                            .filter_map(|b| {
                                if let ContentBlock::ToolResult { tool_use_id, .. } = b {
                                    Some(tool_use_id.clone())
                                } else {
                                    None
                                }
                            })
                            .collect(),
                        _ => vec![],
                    };

                    // Check if these results match our tool calls
                    if result_ids.iter().all(|rid| tool_call_ids.contains(rid)) {
                        // Found matching results - add both messages
                        fixed_messages.push(msg.clone());
                        fixed_messages.push(result_msg.clone());
                        processed_indices.insert(i);
                        processed_indices.insert(j);
                        found_results = true;
                        break;
                    }
                }

                if !found_results {
                    // No results found for these tool calls - skip this message
                    tracing::warn!(
                        "Removing message with orphaned tool calls: {:?}",
                        tool_call_ids
                    );
                    processed_indices.insert(i);
                }
            } else {
                // Check if this is an orphaned tool result
                let result_ids: Vec<String> = match &msg.content {
                    MessageContent::ToolResponses(responses) => {
                        responses.iter().map(|r| r.call_id.clone()).collect()
                    }
                    MessageContent::Blocks(blocks) => blocks
                        .iter()
                        .filter_map(|b| {
                            if let ContentBlock::ToolResult { tool_use_id, .. } = b {
                                Some(tool_use_id.clone())
                            } else {
                                None
                            }
                        })
                        .collect(),
                    _ => vec![],
                };

                if !result_ids.is_empty()
                    && result_ids
                        .iter()
                        .all(|rid| orphaned_tool_results.contains(rid))
                {
                    // Skip orphaned tool results
                    tracing::warn!("Removing orphaned tool results: {:?}", result_ids);
                    processed_indices.insert(i);
                } else {
                    // Regular message - add it
                    fixed_messages.push(msg.clone());
                    processed_indices.insert(i);
                }
            }

            i += 1;
        }

        fixed_messages
    }

    /// Simple truncation strategy
    fn truncate_messages(
        &self,
        batches: Vec<crate::message::MessageBatch>,
        keep_recent: usize,
    ) -> Result<CompressionResult> {
        let original_count: usize = batches.iter().map(|b| b.len()).sum();

        // Collect batches from newest to oldest until we have enough messages
        let mut active_batches = Vec::new();
        let mut archived_batches = Vec::new();
        let mut message_count = 0;

        // Iterate from the end (most recent batches)
        for batch in batches.into_iter().rev() {
            if message_count < keep_recent {
                message_count += batch.len();
                active_batches.push(batch);
            } else {
                archived_batches.push(batch);
            }
        }

        // Reverse to maintain chronological order
        active_batches.reverse();
        archived_batches.reverse();

        let archived_count: usize = archived_batches.iter().map(|b| b.len()).sum();

        // No need to validate tool call/response ordering since batches maintain integrity

        Ok(CompressionResult {
            active_batches,
            summary: None,
            archived_batches: archived_batches.clone(),
            metadata: CompressionMetadata {
                strategy_used: "truncate".to_string(),
                original_count,
                compressed_count: archived_count,
                archived_count,
                compression_time: Utc::now(),
                estimated_tokens_saved: self.estimate_tokens_from_batches(&archived_batches),
            },
        })
    }

    /// Recursive summarization following MemGPT approach
    async fn recursive_summarization(
        &self,
        mut batches: Vec<crate::message::MessageBatch>,
        max_messages: usize,
        chunk_size: usize,
        summarization_model: &str,
    ) -> Result<CompressionResult> {
        if self.model_provider.is_none() {
            return Err(CoreError::ConfigurationError {
                config_path: "compression".to_string(),
                field: "model_provider".to_string(),
                expected: "ModelProvider required for recursive summarization".to_string(),
                cause: crate::error::ConfigError::MissingField("model_provider".to_string()),
            });
        }

        // Sort batches by batch_id (oldest first)
        batches.sort_by_key(|b| b.id);

        let original_count: usize = batches.iter().map(|b| b.len()).sum();

        if original_count <= max_messages {
            return self.truncate_messages(batches, max_messages);
        }

        // Accumulate batches from oldest until we're under max_messages
        let mut active_batches = Vec::new();
        let mut archived_batches = Vec::new();
        let mut current_count = 0;

        // Iterate from newest to oldest (reverse), keeping recent batches
        for batch in batches.into_iter().rev() {
            if current_count + batch.len() <= max_messages {
                current_count += batch.len();
                active_batches.push(batch);
            } else {
                // Only include complete batches in archives
                if batch.is_complete {
                    archived_batches.push(batch);
                } else {
                    // If batch is incomplete, keep it in active
                    current_count += batch.len();
                    active_batches.push(batch);
                }
            }
        }

        // Restore chronological order (oldest to newest)
        active_batches.reverse();
        archived_batches.reverse();

        // Flatten archived batches to messages for summarization
        let messages_to_summarize: Vec<Message> = archived_batches
            .iter()
            .flat_map(|b| b.messages.clone())
            .collect();

        if messages_to_summarize.is_empty() {
            // Nothing to summarize
            return Ok(CompressionResult {
                active_batches,
                summary: None,
                archived_batches: Vec::new(),
                metadata: CompressionMetadata {
                    strategy_used: "recursive_summarization".to_string(),
                    original_count,
                    compressed_count: 0,
                    archived_count: 0,
                    compression_time: Utc::now(),
                    estimated_tokens_saved: 0,
                },
            });
        }

        // Create summary prompt
        let summary_prompt = self.create_summary_prompt(&messages_to_summarize, chunk_size)?;

        // Generate summary using the model
        let summary = if let Some(provider) = &self.model_provider {
            let request = crate::message::Request {
                system: Some(vec![
                    "You are a helpful assistant that creates concise summaries of conversations. \
                     Focus on key information, decisions made, and important context. Maintain the style of the messages as much as possible."
                        .to_string(),
                ]),
                messages: vec![Message::user(summary_prompt)],
                tools: None,
            };

            // Create options for summarization
            // Detect provider from model string
            let provider_name = detect_provider_from_model(summarization_model);

            let model_info = crate::model::ModelInfo {
                id: summarization_model.to_string(),
                name: summarization_model.to_string(),
                provider: provider_name,
                capabilities: vec![],
                context_window: 128000, // Default to a large context
                max_output_tokens: Some(4096),
                cost_per_1k_prompt_tokens: None,
                cost_per_1k_completion_tokens: None,
            };

            // Enhance with proper defaults
            let model_info = crate::model::defaults::enhance_model_info(model_info);

            let mut options = crate::model::ResponseOptions::new(model_info);
            options.max_tokens = Some(1000);
            options.temperature = Some(0.5);

            match provider.complete(&options, request).await {
                Ok(response) => response.only_text(),
                Err(e) => {
                    tracing::warn!("Failed to generate summary: {}", e);
                    format!("[Summary generation failed: {}]", e)
                }
            }
        } else {
            "[No model provider for summarization]".to_string()
        };

        // No need to validate tool ordering - batches maintain integrity
        let archived_count: usize = archived_batches.iter().map(|b| b.len()).sum();

        Ok(CompressionResult {
            active_batches,
            summary: Some(summary),
            archived_batches,
            metadata: CompressionMetadata {
                strategy_used: "recursive_summarization".to_string(),
                original_count,
                compressed_count: archived_count,
                archived_count,
                compression_time: Utc::now(),
                estimated_tokens_saved: self.estimate_tokens(&messages_to_summarize),
            },
        })
    }

    /// Importance-based compression using heuristics or LLM
    async fn importance_based_compression(
        &self,
        mut batches: Vec<crate::message::MessageBatch>,
        keep_recent: usize,
        keep_important: usize,
    ) -> Result<CompressionResult> {
        // Sort batches by batch_id (oldest first)
        batches.sort_by_key(|b| b.id);

        let original_count: usize = batches.iter().map(|b| b.len()).sum();

        // For importance scoring, we need to work with individual messages
        // But we'll try to keep batches intact where possible

        // First, separate recent batches we always keep
        let mut active_batches = Vec::new();
        let mut older_batches = Vec::new();
        let mut recent_message_count = 0;

        // Keep recent batches (from newest)
        for batch in batches.into_iter().rev() {
            if recent_message_count < keep_recent {
                recent_message_count += batch.len();
                active_batches.push(batch);
            } else {
                older_batches.push(batch);
            }
        }

        // Restore chronological order
        active_batches.reverse();
        older_batches.reverse();

        if older_batches.is_empty() {
            // Nothing to compress
            return Ok(CompressionResult {
                active_batches,
                summary: None,
                archived_batches: Vec::new(),
                metadata: CompressionMetadata {
                    strategy_used: "importance_based".to_string(),
                    original_count,
                    compressed_count: 0,
                    archived_count: 0,
                    compression_time: Utc::now(),
                    estimated_tokens_saved: 0,
                },
            });
        };

        // Score older batches based on their messages
        let mut scored_batches: Vec<(f32, crate::message::MessageBatch)> = Vec::new();

        for batch in older_batches.iter() {
            // Calculate batch score as average of message scores
            let mut total_score = 0.0;
            let mut count = 0;

            for (idx, msg) in batch.messages.iter().enumerate() {
                let score = if self.model_provider.is_some() {
                    // Use LLM to score importance
                    self.score_message_with_llm(msg).await.unwrap_or_else(|_| {
                        // Fall back to heuristic if LLM fails
                        self.score_message_heuristic(msg, idx, batch.messages.len())
                    })
                } else {
                    // Use heuristic scoring
                    self.score_message_heuristic(msg, idx, batch.messages.len())
                };

                total_score += score;
                count += 1;
            }

            let batch_score = if count > 0 {
                total_score / count as f32
            } else {
                0.0
            };
            scored_batches.push((batch_score, batch.clone()));
        }

        // Sort by score (highest first)
        scored_batches.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));

        // Keep the most important batches up to keep_important message count
        let mut important_message_count = 0;
        let mut important_batches = Vec::new();
        let mut archived_batches = Vec::new();

        for (_, batch) in scored_batches {
            if important_message_count < keep_important {
                important_message_count += batch.len();
                important_batches.push(batch);
            } else {
                archived_batches.push(batch);
            }
        }

        // Sort important batches back to chronological order
        important_batches.sort_by_key(|b| b.id);

        // Combine important and recent batches
        important_batches.extend(active_batches);
        let active_batches = important_batches;

        // No need to validate tool ordering - batches maintain integrity
        let archived_count: usize = archived_batches.iter().map(|b| b.len()).sum();
        let tokens_saved = self.estimate_tokens_from_batches(&archived_batches);

        Ok(CompressionResult {
            active_batches,
            summary: None,
            archived_batches,
            metadata: CompressionMetadata {
                strategy_used: "importance_based".to_string(),
                original_count,
                compressed_count: archived_count,
                archived_count,
                compression_time: Utc::now(),
                estimated_tokens_saved: tokens_saved,
            },
        })
    }

    /// Score a message's importance using heuristics
    fn score_message_heuristic(&self, msg: &Message, idx: usize, total: usize) -> f32 {
        let mut score = 0.0;

        // Base score by role
        score += match msg.role {
            ChatRole::System => self.scoring_config.system_weight,
            ChatRole::Assistant => self.scoring_config.assistant_weight,
            ChatRole::User => self.scoring_config.user_weight,
            _ => self.scoring_config.other_weight,
        };

        // Recency bonus (newer messages are more important)
        let recency_factor = idx as f32 / total as f32;
        score += recency_factor * self.scoring_config.recency_bonus;

        // Content length bonus (longer messages might contain more information)
        if let Some(text) = msg.text_content() {
            let length_factor = (text.len() as f32 / 100.0).min(3.0);
            score += length_factor * self.scoring_config.content_length_weight;

            // Check for questions
            if text.contains('?') {
                score += self.scoring_config.question_bonus;
            }

            // Check for important keywords
            let text_lower = text.to_lowercase();
            for keyword in &self.scoring_config.important_keywords {
                if text_lower.contains(keyword) {
                    score += self.scoring_config.keyword_bonus;
                }
            }
        }

        // Tool call bonus
        if msg.tool_call_count() > 0 || self.has_tool_use_blocks(msg) {
            score += self.scoring_config.tool_call_bonus;
        }

        score
    }

    /// Score a message's importance using LLM
    async fn score_message_with_llm(&self, msg: &Message) -> Result<f32> {
        if let Some(provider) = &self.model_provider {
            let prompt = format!(
                "Rate the importance of this message in a conversation on a scale of 0-10. \
                 Consider factors like: information content, decisions made, questions asked, \
                 context establishment, and future relevance.\n\n\
                 Message role: {:?}\n\
                 Message content: {}\n\n\
                 Respond with just a number between 0 and 10.",
                msg.role,
                msg.text_content().unwrap_or_default()
            );

            let request = crate::message::Request {
                system: Some(vec![
                    "You are an expert at evaluating message importance.".to_string(),
                ]),
                messages: vec![Message::user(prompt)],
                tools: None,
            };

            let model_info = crate::model::ModelInfo {
                id: "gpt-3.5-turbo".to_string(),
                name: "gpt-3.5-turbo".to_string(),
                provider: "openai".to_string(),
                capabilities: vec![],
                context_window: 16385,
                max_output_tokens: Some(4096),
                cost_per_1k_prompt_tokens: None,
                cost_per_1k_completion_tokens: None,
            };

            let mut options = crate::model::ResponseOptions::new(model_info);
            options.max_tokens = Some(10);
            options.temperature = Some(0.3);

            match provider.complete(&options, request).await {
                Ok(response) => {
                    if let Ok(score) = response.only_text().trim().parse::<f32>() {
                        return Ok(score.clamp(0.0, 10.0));
                    }
                }
                Err(e) => {
                    tracing::warn!("Failed to score message with LLM: {}", e);
                }
            }
        }

        // Fall back to heuristic
        Ok(self.score_message_heuristic(msg, 0, 1))
    }

    /// Time-decay based compression
    fn time_decay_compression(
        &self,
        mut batches: Vec<crate::message::MessageBatch>,
        compress_after_hours: f64,
        min_keep_recent: usize,
    ) -> Result<CompressionResult> {
        // Sort batches by batch_id (oldest first)
        batches.sort_by_key(|b| b.id);

        let original_count: usize = batches.iter().map(|b| b.len()).sum();
        let now = Utc::now();
        let cutoff_time =
            now - chrono::Duration::milliseconds((compress_after_hours * 3600.0 * 1000.0) as i64);

        // Keep recent batches and batches created after cutoff
        let mut active_batches = Vec::new();
        let mut archived_batches = Vec::new();
        let mut recent_message_count = 0;

        // First pass: keep minimum recent batches (from newest)
        for batch in batches.iter().rev() {
            if recent_message_count < min_keep_recent {
                recent_message_count += batch.len();
                active_batches.push(batch.clone());
            }
        }

        // Second pass: check time cutoff for older batches
        for batch in batches.iter() {
            // Skip if already in active
            if active_batches.iter().any(|b| b.id == batch.id) {
                continue;
            }

            // Check if batch is recent enough (using first message's timestamp as proxy)
            let is_recent = batch
                .messages
                .first()
                .map(|msg| msg.created_at > cutoff_time)
                .unwrap_or(false);

            if is_recent {
                active_batches.push(batch.clone());
            } else if batch.is_complete {
                archived_batches.push(batch.clone());
            } else {
                // Keep incomplete batches active
                active_batches.push(batch.clone());
            }
        }

        // Sort to maintain chronological order
        active_batches.sort_by_key(|b| b.id);
        archived_batches.sort_by_key(|b| b.id);

        let archived_count: usize = archived_batches.iter().map(|b| b.len()).sum();

        Ok(CompressionResult {
            active_batches,
            summary: None,
            archived_batches: archived_batches.clone(),
            metadata: CompressionMetadata {
                strategy_used: "time_decay".to_string(),
                original_count,
                compressed_count: archived_count,
                archived_count,
                compression_time: now,
                estimated_tokens_saved: self.estimate_tokens_from_batches(&archived_batches),
            },
        })
    }

    /// Create a prompt for summarizing messages
    fn create_summary_prompt(&self, messages: &[Message], chunk_size: usize) -> Result<String> {
        let mut chunks = Vec::new();

        for chunk in messages.chunks(chunk_size) {
            let mut chunk_text = String::new();
            for msg in chunk {
                chunk_text.push_str(&format!(
                    "{}: {}\n",
                    msg.role,
                    msg.text_content().unwrap_or_default()
                ));
            }
            chunks.push(chunk_text);
        }

        Ok(format!(
            "Please summarize the following conversation chunks into a concise summary. \
             Focus on key information, decisions, and important context:\n\n{}",
            chunks.join("\n---\n")
        ))
    }

    /// Estimate tokens saved by archiving messages
    fn estimate_tokens(&self, messages: &[Message]) -> usize {
        messages.iter().map(|m| m.estimate_tokens()).sum()
    }

    /// Estimate tokens for batches
    fn estimate_tokens_from_batches(&self, batches: &[crate::message::MessageBatch]) -> usize {
        batches
            .iter()
            .flat_map(|b| &b.messages)
            .map(|m| m.estimate_tokens())
            .sum()
    }

    /// Helper to convert messages back to batches (temporary during refactor)
    /// Creates a single batch containing all messages
    #[allow(dead_code)]
    fn messages_to_single_batch(&self, messages: Vec<Message>) -> crate::message::MessageBatch {
        use crate::agent::get_next_message_position_sync;
        use crate::message::{BatchType, MessageBatch};

        let batch_id = get_next_message_position_sync();
        let mut batch = MessageBatch::from_messages(
            batch_id,
            BatchType::UserRequest, // Default type
            messages,
        );
        batch.is_complete = true; // Mark as complete since it's archived
        batch
    }

    /// Parse importance scores from LLM response
    #[allow(dead_code)]
    fn parse_importance_scores(&self, response: &str, count: usize) -> Vec<f32> {
        let mut scores = Vec::new();

        // Try to parse JSON array first
        if let Ok(parsed) = serde_json::from_str::<Vec<f32>>(response) {
            return parsed;
        }

        // Otherwise, look for numbers in the text
        for line in response.lines() {
            if let Some(score_str) = line.split(':').nth(1) {
                if let Ok(score) = score_str.trim().parse::<f32>() {
                    scores.push(score.clamp(0.0, 10.0));
                }
            }
        }

        // If we didn't get enough scores, pad with defaults
        while scores.len() < count {
            scores.push(5.0); // Default middle importance
        }

        scores
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::MessageContent;

    #[test]
    fn test_truncation_strategy() {
        let compressor = MessageCompressor::new(CompressionStrategy::Truncate { keep_recent: 5 });

        let messages = vec![
            Message::user("Hello"),
            Message::agent("Hi there!"),
            Message::user("How are you?"),
            Message::agent("I'm doing well, thanks!"),
            Message::user("What's the weather?"),
            Message::agent("Let me check that for you"),
            Message::user("Any updates?"),
            Message::agent("Still checking..."),
            Message::user("Thanks for checking"),
            Message::agent("You're welcome!"),
        ];

        let result = tokio_test::block_on(compressor.compress(messages, 5)).unwrap();

        assert_eq!(result.active_messages.len(), 5);
        assert_eq!(result.archived_messages.len(), 5);
        assert_eq!(
            result.active_messages[0].text_content().unwrap(),
            "Let me check that for you"
        );
    }

    #[test]
    fn test_compression_with_tool_calls() {
        let compressor = MessageCompressor::new(CompressionStrategy::Truncate { keep_recent: 5 });

        let mut messages = vec![];

        // Add some conversation before the tool calls
        for i in 0..6 {
            messages.push(Message::user(format!("Question {}", i)));
            messages.push(Message::agent(format!("Answer {}", i)));
        }

        // Add tool call sequence
        messages.push(Message::user("Search for something"));
        messages.push(Message {
            role: ChatRole::Assistant,
            content: MessageContent::ToolCalls(vec![crate::message::ToolCall {
                call_id: "456".to_string(),
                fn_name: "search".to_string(),
                fn_arguments: serde_json::json!({"query": "test"}),
            }]),
            has_tool_calls: true,
            ..Message::agent("test")
        });
        messages.push(Message {
            role: ChatRole::Tool,
            content: MessageContent::ToolResponses(vec![crate::message::ToolResponse {
                call_id: "456".to_string(),
                content: "Search results".to_string(),
            }]),
            ..Message::default()
        });

        let result = tokio_test::block_on(compressor.compress(messages, 5)).unwrap();

        // Should keep the last 5 messages
        assert_eq!(result.active_messages.len(), 5);
        assert_eq!(result.archived_messages.len(), 10);

        // The tool call and response should be in the active messages
        let has_tool_call = result
            .active_messages
            .iter()
            .any(|m| m.tool_call_count() > 0);
        let has_tool_response = result
            .active_messages
            .iter()
            .any(|m| m.role == ChatRole::Tool);

        assert!(has_tool_call);
        assert!(has_tool_response);
    }

    #[test]
    fn test_importance_scoring() {
        let compressor = MessageCompressor::new(CompressionStrategy::ImportanceBased {
            keep_recent: 2,
            keep_important: 1,
        });

        let msg = Message::user("This is very important: remember my name is Alice");
        let score = compressor.score_message_heuristic(&msg, 0, 10);

        // Should have high score due to "important" keyword and user role
        assert!(score > 5.0);
    }

    #[test]
    fn test_time_decay_compression() {
        let compressor = MessageCompressor::new(CompressionStrategy::TimeDecay {
            compress_after_hours: 1.0,
            min_keep_recent: 10,
        });

        let now = Utc::now();
        let mut messages = Vec::new();

        // Add 20 old messages (2+ hours old)
        for i in 0..20 {
            messages.push(Message {
                created_at: now - chrono::Duration::hours(3) - chrono::Duration::minutes(i as i64),
                ..if i % 2 == 0 {
                    Message::user(format!("Old message {}", i))
                } else {
                    Message::agent(format!("Old response {}", i))
                }
            });
        }

        // Add 5 messages from 30 mins ago (within the 1 hour cutoff)
        for i in 0..5 {
            messages.push(Message {
                created_at: now - chrono::Duration::minutes(30 - i as i64),
                ..if i % 2 == 0 {
                    Message::user(format!("Recent message {}", i))
                } else {
                    Message::agent(format!("Recent response {}", i))
                }
            });
        }

        // Add 5 very recent messages
        for i in 0..5 {
            messages.push(Message {
                created_at: now - chrono::Duration::seconds(60 - i as i64 * 10),
                ..if i % 2 == 0 {
                    Message::user(format!("Very recent message {}", i))
                } else {
                    Message::agent(format!("Very recent response {}", i))
                }
            });
        }

        let result = tokio_test::block_on(compressor.compress(messages, 15)).unwrap();

        // Should keep at least 10 recent messages (min_keep_recent)
        // Plus the 10 messages that are within the 1 hour cutoff
        assert_eq!(result.active_messages.len(), 10);
        assert_eq!(result.archived_messages.len(), 20);
        assert!(
            result.archived_messages[0]
                .text_content()
                .unwrap()
                .contains("Old message")
        );
    }

    #[test]
    fn test_compression_metadata() {
        let compressor = MessageCompressor::new(CompressionStrategy::Truncate { keep_recent: 1 });

        let messages = vec![
            Message::user("Message 1"),
            Message::agent("Message 2"),
            Message::user("Message 3"),
        ];

        let result = tokio_test::block_on(compressor.compress(messages, 1)).unwrap();

        assert_eq!(result.metadata.original_count, 3);
        assert_eq!(result.metadata.compressed_count, 2);
        assert_eq!(result.metadata.archived_count, 2);
        assert_eq!(result.metadata.strategy_used, "truncate");
    }

    #[test]
    fn test_importance_scoring_config_serialization() {
        let config = ImportanceScoringConfig::default();
        let json = serde_json::to_string(&config).unwrap();
        let deserialized: ImportanceScoringConfig = serde_json::from_str(&json).unwrap();

        assert_eq!(config.system_weight, deserialized.system_weight);
        assert_eq!(config.important_keywords, deserialized.important_keywords);
    }

    #[test]
    fn test_compression_strategy_serialization() {
        let strategies = vec![
            CompressionStrategy::Truncate { keep_recent: 50 },
            CompressionStrategy::ImportanceBased {
                keep_recent: 20,
                keep_important: 10,
            },
            CompressionStrategy::TimeDecay {
                compress_after_hours: 24.0,
                min_keep_recent: 10,
            },
        ];

        for strategy in strategies {
            let json = serde_json::to_string(&strategy).unwrap();
            let deserialized: CompressionStrategy = serde_json::from_str(&json).unwrap();

            // Verify roundtrip works
            let json2 = serde_json::to_string(&deserialized).unwrap();
            assert_eq!(json, json2);
        }
    }

    #[test]
    fn test_custom_scoring_config() {
        let mut config = ImportanceScoringConfig::default();
        config.important_keywords.push("deadline".to_string());
        config.question_bonus = 5.0;

        let compressor = MessageCompressor::new(CompressionStrategy::ImportanceBased {
            keep_recent: 1,
            keep_important: 1,
        })
        .with_scoring_config(config);

        let msg = Message::user("What's the deadline for this project?");
        let score = compressor.score_message_heuristic(&msg, 0, 1);

        // Should have high score due to question and "deadline" keyword
        assert!(score > 10.0);
    }

    #[test]
    fn test_parse_importance_scores() {
        let compressor = MessageCompressor::new(CompressionStrategy::ImportanceBased {
            keep_recent: 1,
            keep_important: 1,
        });

        // Test JSON array parsing
        let scores = compressor.parse_importance_scores("[7.5, 3.2, 9.0]", 3);
        assert_eq!(scores.len(), 3);
        assert_eq!(scores[0], 7.5);

        // Test line-based parsing
        let scores = compressor.parse_importance_scores("Message 1: 8.0\nMessage 2: 4.5", 2);
        assert_eq!(scores.len(), 2);
        assert_eq!(scores[0], 8.0);

        // Test padding when insufficient scores
        let scores = compressor.parse_importance_scores("Score: 7.0", 3);
        assert_eq!(scores.len(), 3);
        assert_eq!(scores[2], 5.0); // Default padding
    }

    #[tokio::test]
    async fn test_importance_based_compression_with_heuristics() {
        let compressor = MessageCompressor::new(CompressionStrategy::ImportanceBased {
            keep_recent: 2,
            keep_important: 2,
        });

        let messages = vec![
            Message::system("You are a helpful assistant"), // High importance
            Message::user("Hi"),
            Message::agent("Hello!"),
            Message::user("What's very important: my password is 12345"), // High importance
            Message::agent("I understand"),
            Message::user("What's the weather?"),
            Message::agent("Let me check that for you"),
        ];

        let result = compressor.compress(messages, 4).await.unwrap();

        // Should keep 2 recent + 2 important messages
        assert_eq!(result.active_messages.len(), 4);

        // System message and important user message should be kept
        assert!(
            result
                .active_messages
                .iter()
                .any(|m| m.role == ChatRole::System)
        );
        assert!(
            result
                .active_messages
                .iter()
                .any(|m| m.text_content().unwrap_or_default().contains("password"))
        );
    }

    #[tokio::test]
    async fn test_recursive_summarization() {
        // This test would require a mock ModelProvider
        // For now, we just test the error case
        let compressor = MessageCompressor::new(CompressionStrategy::RecursiveSummarization {
            chunk_size: 5,
            summarization_model: "gpt-3.5-turbo".to_string(),
        });

        let messages = vec![
            Message::user("Message 1"),
            Message::agent("Response 1"),
            Message::user("Message 2"),
            Message::agent("Response 2"),
            Message::user("Message 3"),
        ];

        let result = compressor.compress(messages, 2).await;

        // Should fail without model provider
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_importance_based_compression_with_llm() {
        // Would require mock ModelProvider
        // Testing the fallback behavior
        let compressor = MessageCompressor::new(CompressionStrategy::ImportanceBased {
            keep_recent: 1,
            keep_important: 1,
        });

        let messages = vec![
            Message::user("Remember this important fact"),
            Message::agent("Noted"),
            Message::user("What's 2+2?"),
        ];

        let result = compressor.compress(messages, 2).await.unwrap();

        // Should work with heuristic fallback
        assert_eq!(result.active_messages.len(), 2);
    }
}
