//! Message compression strategies for managing context window limits
//!
//! This module implements various strategies for compressing message history
//! when it exceeds the context window, following the MemGPT paper's approach.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::{CoreError, ModelProvider, Result, message::Message};

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
        Self::Truncate { keep_recent: 50 }
    }
}

/// Result of message compression
#[derive(Debug, Clone)]
pub struct CompressionResult {
    /// Messages to keep in the active context
    pub active_messages: Vec<Message>,

    /// Summary of compressed messages (if applicable)
    pub summary: Option<String>,

    /// Messages moved to recall storage
    pub archived_messages: Vec<Message>,

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
                "decision".to_string(),
                "important".to_string(),
                "critical".to_string(),
                "remember".to_string(),
                "todo".to_string(),
                "task".to_string(),
                "deadline".to_string(),
            ],
            keyword_bonus: 1.5,
        }
    }
}

/// Compressor for managing message history
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

    /// Compress messages according to the configured strategy
    pub async fn compress(
        &self,
        messages: Vec<Message>,
        max_messages: usize,
    ) -> Result<CompressionResult> {
        let original_count = messages.len();

        if original_count <= max_messages {
            // No compression needed
            return Ok(CompressionResult {
                active_messages: messages,
                summary: None,
                archived_messages: Vec::new(),
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

        match &self.strategy {
            CompressionStrategy::Truncate { keep_recent } => {
                self.truncate_messages(messages, *keep_recent)
            }

            CompressionStrategy::RecursiveSummarization {
                chunk_size,
                summarization_model,
            } => {
                self.recursive_summarization(
                    messages,
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
                self.importance_based_compression(messages, *keep_recent, *keep_important)
                    .await
            }

            CompressionStrategy::TimeDecay {
                compress_after_hours,
                min_keep_recent,
            } => self.time_decay_compression(messages, *compress_after_hours, *min_keep_recent),
        }
    }

    /// Simple truncation strategy
    fn truncate_messages(
        &self,
        messages: Vec<Message>,
        keep_recent: usize,
    ) -> Result<CompressionResult> {
        let original_count = messages.len();
        let archive_count = messages.len().saturating_sub(keep_recent);

        let (archived, active): (Vec<_>, Vec<_>) = if messages.len() > keep_recent {
            let split_point = messages.len() - keep_recent;
            (
                messages[..split_point].to_vec(),
                messages[split_point..].to_vec(),
            )
        } else {
            (Vec::new(), messages)
        };

        Ok(CompressionResult {
            active_messages: active,
            summary: None,
            archived_messages: archived.clone(),
            metadata: CompressionMetadata {
                strategy_used: "truncate".to_string(),
                original_count,
                compressed_count: archive_count,
                archived_count: archive_count,
                compression_time: Utc::now(),
                estimated_tokens_saved: self.estimate_tokens(&archived),
            },
        })
    }

    /// Recursive summarization following MemGPT approach
    async fn recursive_summarization(
        &self,
        messages: Vec<Message>,
        max_messages: usize,
        chunk_size: usize,
        summarization_model: &str,
    ) -> Result<CompressionResult> {
        if self.model_provider.is_none() {
            return Err(CoreError::ConfigurationError {
                config_path: "compression".to_string(),
                field: "model_provider".to_string(),
                expected: "ModelProvider required for recursive summarization".to_string(),
                cause: Box::new(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    "No model provider configured",
                )),
            });
        }

        let original_count = messages.len();
        let messages_to_compress = messages.len().saturating_sub(max_messages);

        if messages_to_compress == 0 {
            return self.truncate_messages(messages, max_messages);
        }

        // Split messages into chunks for summarization
        let compress_until = messages_to_compress;
        let to_summarize = &messages[..compress_until];
        let to_keep = messages[compress_until..].to_vec();

        // Create summary prompt
        let summary_prompt = self.create_summary_prompt(to_summarize, chunk_size)?;

        // Generate summary using the model
        let summary = if let Some(provider) = &self.model_provider {
            let request = crate::message::Request {
                system: Some(vec![
                    "You are a helpful assistant that creates concise summaries of conversations. \
                     Focus on key information, decisions made, and important context."
                        .to_string(),
                ]),
                messages: vec![Message::user(summary_prompt)],
                tools: None,
            };

            // Create options for summarization
            let model_info = crate::model::ModelInfo {
                id: summarization_model.to_string(),
                name: summarization_model.to_string(),
                provider: "unknown".to_string(),
                capabilities: vec![],
                context_window: 128000, // Default to a large context
                max_output_tokens: Some(4096),
                cost_per_1k_prompt_tokens: None,
                cost_per_1k_completion_tokens: None,
            };

            let options = crate::model::ResponseOptions {
                model_info,
                temperature: Some(0.7),
                max_tokens: Some(1000),
                top_p: None,
                stop_sequences: vec![],
                capture_usage: Some(false),
                capture_content: Some(true),
                capture_reasoning_content: Some(false),
                capture_tool_calls: Some(false),
                capture_raw_body: Some(false),
                response_format: None,
                normalize_reasoning_content: Some(false),
                reasoning_effort: None,
            };

            let response = provider.complete(&options, request).await?;

            // Extract the summary from the response content
            response
                .content
                .first()
                .and_then(|content| content.text())
                .map(|s| s.to_string())
                .unwrap_or_else(|| {
                    format!(
                        "Summary of {} messages: Unable to generate summary.",
                        to_summarize.len()
                    )
                })
        } else {
            format!(
                "Summary of {} messages: Model provider not configured for summarization.",
                to_summarize.len()
            )
        };

        Ok(CompressionResult {
            active_messages: to_keep,
            summary: Some(summary),
            archived_messages: to_summarize.to_vec(),
            metadata: CompressionMetadata {
                strategy_used: "recursive_summarization".to_string(),
                original_count,
                compressed_count: to_summarize.len(),
                archived_count: to_summarize.len(),
                compression_time: Utc::now(),
                estimated_tokens_saved: self.estimate_tokens(to_summarize),
            },
        })
    }

    /// Importance-based compression
    async fn importance_based_compression(
        &self,
        messages: Vec<Message>,
        keep_recent: usize,
        keep_important: usize,
    ) -> Result<CompressionResult> {
        let original_count = messages.len();

        // Score messages by importance
        let mut scored_messages: Vec<(usize, Message, f32)> = Vec::new();

        // If we have a model provider, use it for better scoring
        if let Some(provider) = &self.model_provider {
            // Batch score messages using LLM for better importance detection
            let batch_size = 10; // Score 10 messages at a time

            for (batch_idx, batch) in messages.chunks(batch_size).enumerate() {
                let batch_prompt = self.create_importance_scoring_prompt(batch)?;

                let request = crate::message::Request {
                    system: Some(vec![
                        "You are an AI assistant that scores message importance in conversations. \
                         Score each message from 0-10 based on: key decisions, important information, \
                         context changes, user questions, and critical tool calls.".to_string()
                    ]),
                    messages: vec![Message::user(batch_prompt)],
                    tools: None,
                };

                // Create minimal options for scoring
                let model_info = crate::model::ModelInfo {
                    id: "importance-scorer".to_string(),
                    name: "importance-scorer".to_string(),
                    provider: "unknown".to_string(),
                    capabilities: vec![],
                    context_window: 8192,
                    max_output_tokens: Some(500),
                    cost_per_1k_prompt_tokens: None,
                    cost_per_1k_completion_tokens: None,
                };

                let options = crate::model::ResponseOptions {
                    model_info,
                    temperature: Some(0.3), // Low temperature for consistent scoring
                    max_tokens: Some(500),
                    top_p: None,
                    stop_sequences: vec![],
                    capture_usage: Some(false),
                    capture_content: Some(true),
                    capture_reasoning_content: Some(false),
                    capture_tool_calls: Some(false),
                    capture_raw_body: Some(false),
                    response_format: None,
                    normalize_reasoning_content: Some(false),
                    reasoning_effort: None,
                };

                match provider.complete(&options, request).await {
                    Ok(response) => {
                        // Parse scores from response
                        if let Some(scores_text) = response.content.first().and_then(|c| c.text()) {
                            let scores = self.parse_importance_scores(scores_text, batch.len());
                            for (idx, (msg, score)) in batch.iter().zip(scores.iter()).enumerate() {
                                let global_idx = batch_idx * batch_size + idx;
                                scored_messages.push((global_idx, msg.clone(), *score));
                            }
                        } else {
                            // Fallback to heuristic scoring
                            for (idx, msg) in batch.iter().enumerate() {
                                let global_idx = batch_idx * batch_size + idx;
                                let score =
                                    self.score_message_importance(msg, global_idx, messages.len());
                                scored_messages.push((global_idx, msg.clone(), score));
                            }
                        }
                    }
                    Err(_) => {
                        // Fallback to heuristic scoring for this batch
                        for (idx, msg) in batch.iter().enumerate() {
                            let global_idx = batch_idx * batch_size + idx;
                            let score =
                                self.score_message_importance(msg, global_idx, messages.len());
                            scored_messages.push((global_idx, msg.clone(), score));
                        }
                    }
                }
            }
        } else {
            // No model provider, use heuristic scoring
            scored_messages = messages
                .into_iter()
                .enumerate()
                .map(|(idx, msg)| {
                    let score = self.score_message_importance(&msg, idx, original_count);
                    (idx, msg, score)
                })
                .collect();
        }

        // Sort by score (descending)
        scored_messages.sort_by(|a, b| b.2.partial_cmp(&a.2).unwrap());

        // Keep the most recent and most important messages
        let recent_start = original_count.saturating_sub(keep_recent);
        let mut keep_indices: std::collections::HashSet<usize> =
            (recent_start..original_count).collect();

        // Add important messages
        for (idx, _, _) in scored_messages.iter().take(keep_important) {
            keep_indices.insert(*idx);
        }

        // Split messages, maintaining original order
        let mut active = Vec::new();
        let mut archived = Vec::new();

        for (idx, message, _score) in &scored_messages {
            if keep_indices.contains(idx) {
                active.push((*idx, message.clone()));
            } else {
                archived.push(message.clone());
            }
        }

        // Sort active messages by original index to maintain order
        active.sort_by_key(|(idx, _)| *idx);
        let active: Vec<Message> = active.into_iter().map(|(_, msg)| msg).collect();

        Ok(CompressionResult {
            active_messages: active,
            summary: None,
            archived_messages: archived.clone(),
            metadata: CompressionMetadata {
                strategy_used: "importance_based".to_string(),
                original_count,
                compressed_count: archived.len(),
                archived_count: archived.len(),
                compression_time: Utc::now(),
                estimated_tokens_saved: self.estimate_tokens(&archived),
            },
        })
    }

    /// Time-decay based compression
    fn time_decay_compression(
        &self,
        messages: Vec<Message>,
        compress_after_hours: f64,
        min_keep_recent: usize,
    ) -> Result<CompressionResult> {
        let original_count = messages.len();
        let now = Utc::now();
        let _cutoff = now - chrono::Duration::seconds((compress_after_hours * 3600.0) as i64);

        // Always keep minimum recent messages
        let recent_start = messages.len().saturating_sub(min_keep_recent);

        let mut active = Vec::new();
        let mut archived = Vec::new();

        for (idx, msg) in messages.into_iter().enumerate() {
            if idx >= recent_start {
                // Keep recent messages
                active.push(msg);
            } else {
                // For older messages, check timestamp
                // Since ChatMessage doesn't have timestamps, we'd need to extend it
                // For now, just archive older messages
                archived.push(msg);
            }
        }

        Ok(CompressionResult {
            active_messages: active,
            summary: None,
            archived_messages: archived.clone(),
            metadata: CompressionMetadata {
                strategy_used: "time_decay".to_string(),
                original_count,
                compressed_count: archived.len(),
                archived_count: archived.len(),
                compression_time: Utc::now(),
                estimated_tokens_saved: self.estimate_tokens(&archived),
            },
        })
    }

    /// Create a prompt for scoring message importance
    fn create_importance_scoring_prompt(&self, messages: &[Message]) -> Result<String> {
        let mut prompt = String::from(
            "Score the importance of each message below from 0-10. \
             Consider: key decisions, important information, context changes, \
             user questions, and critical tool calls.\n\n\
             Format your response as a simple list of scores, one per line.\n\
             Example:\n7.5\n3.0\n9.0\n\nMessages to score:\n\n",
        );

        for (idx, msg) in messages.iter().enumerate() {
            let content = msg
                .text_content()
                .unwrap_or_else(|| "[No text content]".to_string());
            let truncated = if content.len() > 200 {
                format!("{}...", &content[..200])
            } else {
                content
            };
            prompt.push_str(&format!(
                "Message {}: [{}] {}\n",
                idx + 1,
                msg.role,
                truncated
            ));
        }

        Ok(prompt)
    }

    /// Parse importance scores from model response
    fn parse_importance_scores(&self, response: &str, expected_count: usize) -> Vec<f32> {
        let mut scores = Vec::new();

        // Try to parse each line as a float
        for line in response.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }

            // Try to extract a number from the line - look through all words
            for word in trimmed.split_whitespace() {
                if let Ok(score) = word.parse::<f32>() {
                    scores.push(score.clamp(0.0, 10.0));
                    break; // Only take the first valid number from each line
                }
            }
        }

        // If we didn't get enough scores, pad with defaults
        while scores.len() < expected_count {
            scores.push(5.0); // Default middle score
        }

        scores.truncate(expected_count);
        scores
    }

    /// Score a message's importance using configurable weights
    fn score_message_importance(
        &self,
        message: &Message,
        index: usize,
        total_messages: usize,
    ) -> f32 {
        let mut score = 0.0;
        let config = &self.scoring_config;

        // Role-based scoring
        if message.role.is_system() {
            score += config.system_weight;
        } else if message.role.is_assistant() {
            score += config.assistant_weight;
        } else if message.role.is_user() {
            score += config.user_weight;
        } else {
            score += config.other_weight;
        }

        // Recency bonus (linear decay)
        let recency = index as f32 / total_messages as f32;
        score += recency * config.recency_bonus;

        // Content-based scoring
        if let Some(text) = message.text_content() {
            // Length-based importance
            let length_score = (text.len() as f32 / 100.0) * config.content_length_weight;
            score += length_score.min(3.0);

            // Messages with questions are important
            if text.contains('?') {
                score += config.question_bonus;
            }

            // Check for important keywords
            let text_lower = text.to_lowercase();
            for keyword in &config.important_keywords {
                if text_lower.contains(&keyword.to_lowercase()) {
                    score += config.keyword_bonus;
                }
            }
        }

        // Messages with tool calls are important
        if message.has_tool_calls {
            score += config.tool_call_bonus;
        }

        score
    }

    /// Create a summary prompt for the model
    fn create_summary_prompt(&self, messages: &[Message], chunk_size: usize) -> Result<String> {
        let chunks: Vec<String> = messages
            .chunks(chunk_size)
            .map(|chunk| {
                chunk
                    .iter()
                    .map(|msg| {
                        format!(
                            "{}: {}",
                            msg.role,
                            msg.text_content()
                                .unwrap_or_else(|| "[No content]".to_string())
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
            })
            .collect();

        Ok(format!(
            "Please provide a concise summary of the following conversation chunks. \
             Focus on key information, decisions made, and important context:\n\n{}",
            chunks.join("\n\n---\n\n")
        ))
    }

    /// Estimate tokens for a set of messages
    fn estimate_tokens(&self, messages: &[Message]) -> usize {
        messages.iter().map(|msg| msg.estimate_tokens()).sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{ModelCapability, ModelInfo, ModelProvider};
    use async_trait::async_trait;

    // Mock model provider for testing
    #[derive(Debug, Clone)]
    struct MockModelProvider {
        response: String,
    }

    #[async_trait]
    impl ModelProvider for MockModelProvider {
        fn name(&self) -> &str {
            "mock"
        }

        async fn complete(
            &self,
            _options: &crate::model::ResponseOptions,
            _request: crate::message::Request,
        ) -> crate::Result<crate::message::Response> {
            Ok(crate::message::Response {
                content: vec![crate::message::MessageContent::from_text(
                    self.response.clone(),
                )],
                reasoning: None,
                tool_calls: vec![],
                metadata: Default::default(),
            })
        }

        async fn list_models(&self) -> crate::Result<Vec<ModelInfo>> {
            Ok(vec![])
        }

        async fn supports_capability(&self, _model: &str, _capability: ModelCapability) -> bool {
            true
        }

        async fn count_tokens(&self, _model: &str, _content: &str) -> crate::Result<usize> {
            Ok(100)
        }
    }

    #[test]
    fn test_truncation_strategy() {
        let compressor = MessageCompressor::new(CompressionStrategy::Truncate { keep_recent: 3 });

        let messages: Vec<Message> = (0..5)
            .map(|i| Message::user(format!("Message {}", i)))
            .collect();

        let result = tokio_test::block_on(compressor.compress(messages, 3)).unwrap();

        assert_eq!(result.active_messages.len(), 3);
        assert_eq!(result.archived_messages.len(), 2);
        assert_eq!(result.metadata.compressed_count, 2);
        assert_eq!(result.metadata.strategy_used, "truncate");

        // Check that we kept the most recent messages
        assert_eq!(
            result.active_messages[0].text_content().unwrap(),
            "Message 2"
        );
        assert_eq!(
            result.active_messages[2].text_content().unwrap(),
            "Message 4"
        );
    }

    #[test]
    fn test_importance_scoring() {
        let compressor = MessageCompressor::new(CompressionStrategy::default());

        let user_msg = Message::user("What is the weather?");
        let assistant_msg = Message::agent("The weather is sunny.");
        let system_msg = Message::system("You are a helpful assistant.");

        let user_score = compressor.score_message_importance(&user_msg, 0, 3);
        let assistant_score = compressor.score_message_importance(&assistant_msg, 1, 3);
        let system_score = compressor.score_message_importance(&system_msg, 2, 3);

        // System messages should score highest by default
        assert!(system_score > user_score);
        assert!(system_score > assistant_score);

        // User message with question should score higher than assistant without
        assert!(user_score > assistant_score);
    }

    #[test]
    fn test_custom_scoring_config() {
        let mut config = ImportanceScoringConfig::default();
        config.question_bonus = 10.0; // Make questions very important
        config.important_keywords = vec!["critical".to_string(), "urgent".to_string()];
        config.keyword_bonus = 5.0;

        let compressor =
            MessageCompressor::new(CompressionStrategy::default()).with_scoring_config(config);

        let normal_msg = Message::user("Hello there");
        let question_msg = Message::user("What should I do?");
        let critical_msg = Message::user("This is critical information");

        let normal_score = compressor.score_message_importance(&normal_msg, 0, 3);
        let question_score = compressor.score_message_importance(&question_msg, 1, 3);
        let critical_score = compressor.score_message_importance(&critical_msg, 2, 3);

        // Question should have high score due to bonus
        assert!(question_score > normal_score + 5.0);

        // Critical keyword should boost score
        assert!(critical_score > normal_score);
    }

    #[test]
    fn test_importance_based_compression_with_heuristics() {
        let strategy = CompressionStrategy::ImportanceBased {
            keep_recent: 2,
            keep_important: 2,
        };
        let compressor = MessageCompressor::new(strategy);

        let messages = vec![
            Message::system("System prompt"), // High importance
            Message::user("Unimportant chat"),
            Message::user("What is the critical task?"), // Has question + keyword
            Message::agent("Here's the task..."),
            Message::user("Thanks"),          // Recent
            Message::agent("You're welcome"), // Recent
        ];

        let result = tokio_test::block_on(compressor.compress(messages, 4)).unwrap();

        assert_eq!(result.active_messages.len(), 4);
        assert_eq!(result.archived_messages.len(), 2);

        // Should keep system message (important) and recent messages
        let kept_contents: Vec<String> = result
            .active_messages
            .iter()
            .map(|m| m.text_content().unwrap_or_default())
            .collect();

        assert!(kept_contents.contains(&"System prompt".to_string()));
        assert!(kept_contents.contains(&"Thanks".to_string()));
        assert!(kept_contents.contains(&"You're welcome".to_string()));
    }

    #[tokio::test]
    async fn test_importance_based_compression_with_llm() {
        let mock_provider = MockModelProvider {
            response: "9.0\n2.0\n8.5\n3.0\n5.0\n4.0".to_string(),
        };

        let strategy = CompressionStrategy::ImportanceBased {
            keep_recent: 2,
            keep_important: 2,
        };

        let compressor =
            MessageCompressor::new(strategy).with_model_provider(Box::new(mock_provider));

        let messages = vec![
            Message::system("System prompt"),     // Score: 9.0
            Message::user("Unimportant chat"),    // Score: 2.0
            Message::user("Important question?"), // Score: 8.5
            Message::agent("Response"),           // Score: 3.0
            Message::user("Recent 1"),            // Score: 5.0
            Message::agent("Recent 2"),           // Score: 4.0
        ];

        let result = compressor.compress(messages, 4).await.unwrap();

        assert_eq!(result.active_messages.len(), 4);

        // Should keep high-scoring messages and recent ones
        let kept_contents: Vec<String> = result
            .active_messages
            .iter()
            .map(|m| m.text_content().unwrap_or_default())
            .collect();

        assert!(kept_contents.contains(&"System prompt".to_string())); // High score
        assert!(kept_contents.contains(&"Important question?".to_string())); // High score
        assert!(kept_contents.contains(&"Recent 1".to_string())); // Recent
        assert!(kept_contents.contains(&"Recent 2".to_string())); // Recent
    }

    #[tokio::test]
    async fn test_recursive_summarization() {
        let mock_provider = MockModelProvider {
            response: "Summary: Users discussed weather and made plans.".to_string(),
        };

        let strategy = CompressionStrategy::RecursiveSummarization {
            chunk_size: 3,
            summarization_model: "test-model".to_string(),
        };

        let compressor =
            MessageCompressor::new(strategy).with_model_provider(Box::new(mock_provider));

        let messages: Vec<Message> = (0..10)
            .map(|i| Message::user(format!("Message {}", i)))
            .collect();

        let result = compressor.compress(messages, 5).await.unwrap();

        assert_eq!(result.active_messages.len(), 5);
        assert_eq!(result.archived_messages.len(), 5);
        assert!(result.summary.is_some());
        assert!(result.summary.unwrap().contains("Users discussed weather"));
        assert_eq!(result.metadata.strategy_used, "recursive_summarization");
    }

    #[test]
    fn test_time_decay_compression() {
        let strategy = CompressionStrategy::TimeDecay {
            compress_after_hours: 1.0,
            min_keep_recent: 3,
        };
        let compressor = MessageCompressor::new(strategy);

        let messages: Vec<Message> = (0..10)
            .map(|i| Message::user(format!("Message {}", i)))
            .collect();

        let result = tokio_test::block_on(compressor.compress(messages, 5)).unwrap();

        // Should keep at least min_keep_recent
        assert!(result.active_messages.len() >= 3);
        assert_eq!(result.metadata.strategy_used, "time_decay");
    }

    #[test]
    fn test_compression_with_tool_calls() {
        let mut config = ImportanceScoringConfig::default();
        config.tool_call_bonus = 20.0; // Make tool calls very important

        let compressor = MessageCompressor::new(CompressionStrategy::ImportanceBased {
            keep_recent: 1,
            keep_important: 2,
        })
        .with_scoring_config(config);

        let mut tool_msg = Message::agent("Using tool");
        tool_msg.has_tool_calls = true;

        let messages = vec![
            Message::user("Do something"),
            tool_msg.clone(),
            Message::agent("Normal response"),
            Message::user("Thanks"),
        ];

        let result = tokio_test::block_on(compressor.compress(messages, 3)).unwrap();

        // Should definitely keep the tool call message
        let kept_contents: Vec<String> = result
            .active_messages
            .iter()
            .map(|m| m.text_content().unwrap_or_default())
            .collect();

        assert!(kept_contents.contains(&"Using tool".to_string()));
    }

    #[test]
    fn test_compression_metadata() {
        let compressor = MessageCompressor::new(CompressionStrategy::Truncate { keep_recent: 5 });

        let messages: Vec<Message> = (0..10)
            .map(|i| Message::user(format!("Message {} with some content", i)))
            .collect();

        let result = tokio_test::block_on(compressor.compress(messages, 5)).unwrap();

        assert_eq!(result.metadata.original_count, 10);
        assert_eq!(result.metadata.compressed_count, 5);
        assert_eq!(result.metadata.archived_count, 5);
        assert!(result.metadata.estimated_tokens_saved > 0);
        assert!(result.metadata.compression_time <= Utc::now());
    }

    #[test]
    fn test_parse_importance_scores() {
        let compressor = MessageCompressor::new(CompressionStrategy::default());

        // Test valid scores
        let response = "8.5\n3.0\n9.2\n";
        let scores = compressor.parse_importance_scores(response, 3);
        assert_eq!(scores.len(), 3);
        assert_eq!(scores[0], 8.5);
        assert_eq!(scores[1], 3.0);
        assert_eq!(scores[2], 9.2);

        // Test with extra text
        let response = "Score: 7.0\nImportance: 4.5\n9.0 is the score";
        let scores = compressor.parse_importance_scores(response, 3);
        assert_eq!(scores[0], 7.0);
        assert_eq!(scores[1], 4.5);
        assert_eq!(scores[2], 9.0);

        // Test with missing scores (should pad with 5.0)
        let response = "8.0\n";
        let scores = compressor.parse_importance_scores(response, 3);
        assert_eq!(scores.len(), 3);
        assert_eq!(scores[0], 8.0);
        assert_eq!(scores[1], 5.0);
        assert_eq!(scores[2], 5.0);

        // Test clamping
        let response = "15.0\n-3.0\n5.5";
        let scores = compressor.parse_importance_scores(response, 3);
        assert_eq!(scores[0], 10.0); // Clamped from 15.0
        assert_eq!(scores[1], 0.0); // Clamped from -3.0
        assert_eq!(scores[2], 5.5);
    }

    #[test]
    fn test_compression_strategy_serialization() {
        // Test default serialization
        let strategy = CompressionStrategy::default();
        let json = serde_json::to_string(&strategy).unwrap();
        assert_eq!(json, r#"{"type":"truncate","keep_recent":50}"#);

        // Test deserialization
        let deserialized: CompressionStrategy = serde_json::from_str(&json).unwrap();
        match deserialized {
            CompressionStrategy::Truncate { keep_recent } => assert_eq!(keep_recent, 50),
            _ => panic!("Expected Truncate variant"),
        }

        // Test importance-based serialization
        let strategy = CompressionStrategy::ImportanceBased {
            keep_recent: 10,
            keep_important: 5,
        };
        let json = serde_json::to_string(&strategy).unwrap();
        assert_eq!(
            json,
            r#"{"type":"importance_based","keep_recent":10,"keep_important":5}"#
        );

        // Test recursive summarization serialization
        let strategy = CompressionStrategy::RecursiveSummarization {
            chunk_size: 20,
            summarization_model: "gpt-4".to_string(),
        };
        let json = serde_json::to_string(&strategy).unwrap();
        assert!(json.contains("recursive_summarization"));
        assert!(json.contains("gpt-4"));

        // Test that empty object fails to deserialize
        let empty = "{}";
        let result = serde_json::from_str::<CompressionStrategy>(empty);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("missing field `type`")
        );
    }

    #[test]
    fn test_importance_scoring_config_serialization() {
        let config = ImportanceScoringConfig::default();
        let json = serde_json::to_string(&config).unwrap();
        let deserialized: ImportanceScoringConfig = serde_json::from_str(&json).unwrap();

        assert_eq!(config.system_weight, deserialized.system_weight);
        assert_eq!(
            config.important_keywords.len(),
            deserialized.important_keywords.len()
        );
    }
}
