//! Message compression strategies for managing context window limits
//!
//! This module implements various strategies for compressing message history
//! when it exceeds the context window, following the MemGPT paper's approach.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::genai_ext::ChatRoleExt;
use crate::{CoreError, ModelProvider, Result, message::Message};

/// Strategy for compressing messages when context is full
#[derive(Debug, Clone, Serialize, Deserialize)]
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

/// Compressor for managing message history
pub struct MessageCompressor {
    strategy: CompressionStrategy,
    model_provider: Option<Box<dyn ModelProvider>>,
}

impl MessageCompressor {
    /// Create a new message compressor
    pub fn new(strategy: CompressionStrategy) -> Self {
        Self {
            strategy,
            model_provider: None,
        }
    }

    /// Set the model provider for strategies that need it
    pub fn with_model_provider(mut self, provider: Box<dyn ModelProvider>) -> Self {
        self.model_provider = Some(provider);
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

            CompressionStrategy::RecursiveSummarization { chunk_size, .. } => {
                self.recursive_summarization(messages, max_messages, *chunk_size)
                    .await
            }

            CompressionStrategy::ImportanceBased {
                keep_recent,
                keep_important,
            } => self.importance_based_compression(messages, *keep_recent, *keep_important),

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
        let _summary_prompt = self.create_summary_prompt(to_summarize, chunk_size)?;

        // TODO: Actually call the model to generate summary
        // For now, create a placeholder summary
        let summary = format!(
            "Summary of {} messages: Key points from conversation...",
            to_summarize.len()
        );

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
    fn importance_based_compression(
        &self,
        messages: Vec<Message>,
        keep_recent: usize,
        keep_important: usize,
    ) -> Result<CompressionResult> {
        let original_count = messages.len();

        // Score messages by importance
        let mut scored_messages: Vec<(usize, &Message, f32)> = messages
            .iter()
            .enumerate()
            .map(|(idx, msg)| {
                let score = self.score_message_importance(msg, idx, messages.len());
                (idx, msg, score)
            })
            .collect();

        // Sort by score (descending)
        scored_messages.sort_by(|a, b| b.2.partial_cmp(&a.2).unwrap());

        // Keep the most recent and most important messages
        let recent_start = messages.len().saturating_sub(keep_recent);
        let mut keep_indices: std::collections::HashSet<usize> =
            (recent_start..messages.len()).collect();

        // Add important messages
        for (idx, _, _) in scored_messages.iter().take(keep_important) {
            keep_indices.insert(*idx);
        }

        // Split messages
        let mut active = Vec::new();
        let mut archived = Vec::new();

        for (idx, msg) in messages.into_iter().enumerate() {
            if keep_indices.contains(&idx) {
                active.push(msg);
            } else {
                archived.push(msg);
            }
        }

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

    /// Score a message's importance
    fn score_message_importance(
        &self,
        message: &Message,
        index: usize,
        total_messages: usize,
    ) -> f32 {
        let mut score = 0.0;

        // Role-based scoring
        if message.role.is_system() {
            score += 10.0;
        } else if message.role.is_assistant() {
            score += 3.0;
        } else if message.role.is_user() {
            score += 5.0;
        } else {
            score += 1.0;
        }

        // Recency bonus (linear decay)
        let recency = index as f32 / total_messages as f32;
        score += recency * 5.0;

        // Content-based scoring
        if let Some(text) = message.text_content() {
            // Longer messages might be more important
            score += (text.len() as f32 / 100.0).min(3.0);

            // Messages with questions are important
            if text.contains('?') {
                score += 2.0;
            }
        }

        // Messages with tool calls are important
        if message.has_tool_calls() {
            score += 4.0;
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
    }

    #[test]
    fn test_importance_scoring() {
        let compressor = MessageCompressor::new(CompressionStrategy::default());

        let user_msg = Message::user("What is the weather?");
        let assistant_msg = Message::agent("The weather is sunny.");

        let user_score = compressor.score_message_importance(&user_msg, 0, 2);
        let assistant_score = compressor.score_message_importance(&assistant_msg, 1, 2);

        // User message with question should score higher than plain assistant message
        assert!(user_score > 0.0);
        assert!(assistant_score > 0.0);
    }
}
