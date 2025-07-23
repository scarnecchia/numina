//! Search through conversation history

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{
    Result,
    context::AgentHandle,
    message::{ChatRole, Message},
    tool::AiTool,
};

/// Search through conversation history
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub struct SearchConversationsInput {
    /// Search query - will search in message content
    #[schemars(default, with = "String")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub query: Option<String>,

    /// Filter by participant (user_id)
    #[schemars(default, with = "String")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub participant_id: Option<String>,

    /// Filter by message role
    #[schemars(default, with = "String")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,

    /// Start time for search range (ISO 8601)
    #[schemars(default, with = "String")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_time: Option<String>,

    /// End time for search range (ISO 8601)
    #[schemars(default, with = "String")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end_time: Option<String>,

    /// Maximum number of results to return
    #[serde(default = "default_limit")]
    #[schemars(default = "default_limit", with = "i64")]
    pub limit: usize,

    /// Whether to include context messages around matches
    #[serde(default)]
    pub include_context: bool,

    /// Number of messages before/after to include as context
    #[serde(default = "default_context_window")]
    #[schemars(default = "default_context_window", with = "i64")]
    pub context_window: usize,
}

fn default_limit() -> usize {
    20
}

fn default_context_window() -> usize {
    2
}

/// A single conversation result
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct ConversationResult {
    /// The message that matched
    pub message: MessageSummary,

    /// Context messages if requested
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub context: Vec<MessageSummary>,
}

/// Simplified message for search results
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct MessageSummary {
    pub id: String,
    pub role: String,
    pub content: String,
    pub created_at: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub owner_id: Option<String>,
}

/// Output from conversation search
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct SearchConversationsOutput {
    /// Whether the search was successful
    pub success: bool,

    /// Number of results found
    pub count: usize,

    /// The conversation results
    pub results: Vec<ConversationResult>,

    /// Optional message about the search
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

/// Tool for searching conversation history
#[derive(Debug, Clone)]
pub struct SearchConversationsTool<C: surrealdb::Connection + Clone> {
    pub(crate) handle: AgentHandle<C>,
}

#[async_trait]
impl<C: surrealdb::Connection + Clone + std::fmt::Debug> AiTool for SearchConversationsTool<C> {
    type Input = SearchConversationsInput;
    type Output = SearchConversationsOutput;

    fn name(&self) -> &str {
        "search_conversations"
    }

    fn description(&self) -> &str {
        "Search through conversation history. Filter by content, participant, role, or time range. Returns relevant messages with optional context."
    }

    async fn execute(&self, params: Self::Input) -> Result<Self::Output> {
        // Check if we have database connection
        if !self.handle.has_db_connection() {
            return Ok(SearchConversationsOutput {
                success: false,
                count: 0,
                results: vec![],
                message: Some(
                    "No database connection available for conversation search".to_string(),
                ),
            });
        }

        // Parse role if provided
        let role_filter = if let Some(role) = &params.role {
            // Validate role
            let chat_role = match role.to_lowercase().as_str() {
                "system" => ChatRole::System,
                "user" => ChatRole::User,
                "assistant" => ChatRole::Assistant,
                "tool" => ChatRole::Tool,
                _ => {
                    return Err(crate::CoreError::tool_execution_error(
                        "search_conversations",
                        &format!(
                            "Invalid role: {}. Must be one of: system, user, assistant, tool",
                            role
                        ),
                    ));
                }
            };
            Some(chat_role)
        } else {
            None
        };

        // Parse timestamps if provided
        let start_time = if let Some(start_time) = &params.start_time {
            let dt = DateTime::parse_from_rfc3339(start_time).map_err(|e| {
                crate::CoreError::tool_execution_error(
                    "search_conversations",
                    &format!("Invalid start_time format (expected ISO 8601): {}", e),
                )
            })?;
            Some(dt.to_utc())
        } else {
            None
        };

        let end_time = if let Some(end_time) = &params.end_time {
            let dt = DateTime::parse_from_rfc3339(end_time).map_err(|e| {
                crate::CoreError::tool_execution_error(
                    "search_conversations",
                    &format!("Invalid end_time format (expected ISO 8601): {}", e),
                )
            })?;
            Some(dt.to_utc())
        } else {
            None
        };

        // Use the AgentHandle method to search conversations
        let messages = match self
            .handle
            .search_conversations(
                params.query.as_deref(),
                role_filter,
                start_time,
                end_time,
                params.limit,
            )
            .await
        {
            Ok(messages) => messages,
            Err(e) => {
                tracing::warn!("Conversation search failed: {}", e);
                return Ok(SearchConversationsOutput {
                    success: false,
                    count: 0,
                    results: vec![],
                    message: Some(format!("Search failed: {}", e)),
                });
            }
        };

        // Convert to summaries
        let mut results = Vec::new();
        for message in messages {
            let summary = MessageSummary {
                id: message.id.0.clone(),
                role: format!("{:?}", message.role),
                content: self.extract_content_string(&message),
                created_at: message.created_at,
                owner_id: message.owner_id.map(|id| id.to_string()),
            };

            let result = ConversationResult {
                message: summary,
                context: vec![],
            };

            // Add context if requested
            if params.include_context && params.context_window > 0 {
                // TODO: Implement context fetching
                // This would fetch N messages before and after the matched message
            }

            results.push(result);
        }

        Ok(SearchConversationsOutput {
            success: true,
            count: results.len(),
            results,
            message: None,
        })
    }

    fn examples(&self) -> Vec<crate::tool::ToolExample<Self::Input, Self::Output>> {
        vec![
            crate::tool::ToolExample {
                description: "Search for messages mentioning 'meeting'".to_string(),
                parameters: SearchConversationsInput {
                    query: Some("meeting".to_string()),
                    participant_id: None,
                    role: None,
                    start_time: None,
                    end_time: None,
                    limit: 10,
                    include_context: false,
                    context_window: 0,
                },
                expected_output: Some(SearchConversationsOutput {
                    success: true,
                    count: 2,
                    results: vec![],
                    message: None,
                }),
            },
            crate::tool::ToolExample {
                description: "Find all assistant messages from today".to_string(),
                parameters: SearchConversationsInput {
                    query: None,
                    participant_id: None,
                    role: Some("assistant".to_string()),
                    start_time: Some("2024-01-20T00:00:00Z".to_string()),
                    end_time: None,
                    limit: 20,
                    include_context: true,
                    context_window: 2,
                },
                expected_output: Some(SearchConversationsOutput {
                    success: true,
                    count: 5,
                    results: vec![],
                    message: None,
                }),
            },
        ]
    }
}

impl<C: surrealdb::Connection + Clone> SearchConversationsTool<C> {
    /// Extract content as string from message
    fn extract_content_string(&self, message: &Message) -> String {
        use crate::message::{ContentPart, MessageContent};

        match &message.content {
            MessageContent::Text(text) => text.clone(),
            MessageContent::Parts(parts) => parts
                .iter()
                .filter_map(|part| match part {
                    ContentPart::Text(text) => Some(text.clone()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join(" "),
            MessageContent::ToolCalls(calls) => {
                format!("[Tool calls: {}]", calls.len())
            }
            MessageContent::ToolResponses(responses) => responses
                .iter()
                .map(|r| &r.content)
                .cloned()
                .collect::<Vec<_>>()
                .join("\n"),
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::Memory;

    use super::*;

    #[tokio::test]
    async fn test_search_conversations_without_db() {
        let handle = AgentHandle::test_with_memory(Memory::new());
        let tool = SearchConversationsTool { handle };

        let result = tool
            .execute(SearchConversationsInput {
                query: Some("test".to_string()),
                participant_id: None,
                role: None,
                start_time: None,
                end_time: None,
                limit: 10,
                include_context: false,
                context_window: 0,
            })
            .await
            .unwrap();

        assert!(!result.success);
        assert_eq!(result.count, 0);
        assert!(result.message.is_some());
    }
}
