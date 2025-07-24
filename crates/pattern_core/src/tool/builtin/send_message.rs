//! Message sending tool for agents

use async_trait::async_trait;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{Result, context::AgentHandle, tool::AiTool};

use super::{MessageTarget, TargetType};

/// Input parameters for sending a message
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub struct SendMessageInput {
    /// The target to send the message to
    pub target: MessageTarget,

    /// The message content
    pub content: String,

    /// Optional metadata for the message
    #[schemars(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
}

/// Output from send message operation
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct SendMessageOutput {
    /// Whether the message was sent successfully
    pub success: bool,

    /// Unique identifier for the sent message
    #[schemars(default, with = "String")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message_id: Option<String>,

    /// Any additional information about the send operation
    #[schemars(default, with = "String")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<String>,
}

/// Tool for sending messages to various targets
#[derive(Debug, Clone)]
pub struct SendMessageTool<C: surrealdb::Connection + Clone> {
    pub handle: AgentHandle<C>,
}

#[async_trait]
impl<C: surrealdb::Connection + Clone + std::fmt::Debug> AiTool for SendMessageTool<C> {
    type Input = SendMessageInput;
    type Output = SendMessageOutput;

    fn name(&self) -> &str {
        "send_message"
    }

    fn description(&self) -> &str {
        "Send a message to the user, another agent, a group, or a specific channel. This is the primary way to communicate."
    }

    async fn execute(&self, params: Self::Input) -> Result<Self::Output> {
        // TODO: Implement actual message sending logic
        // This will need to integrate with the message bus/sender in the AgentHandle

        let result = match params.target.target_type {
            TargetType::User => {
                // Send to user through default channel
                Ok(SendMessageOutput {
                    success: true,
                    message_id: Some(format!("msg_{}", chrono::Utc::now().timestamp())),
                    details: Some("Message sent to user".to_string()),
                })
            }
            TargetType::Agent => {
                // Send to another agent
                let agent_id = params.target.target_id.as_deref().unwrap_or("unknown");
                Ok(SendMessageOutput {
                    success: true,
                    message_id: Some(format!("msg_{}", chrono::Utc::now().timestamp())),
                    details: Some(format!("Message sent to agent {}", agent_id)),
                })
            }
            TargetType::Group => {
                // Send to a group
                let group_id = params.target.target_id.as_deref().unwrap_or("unknown");
                Ok(SendMessageOutput {
                    success: true,
                    message_id: Some(format!("msg_{}", chrono::Utc::now().timestamp())),
                    details: Some(format!("Message sent to group {}", group_id)),
                })
            }
            TargetType::Channel => {
                // Send to a specific channel
                let channel_id = params.target.target_id.as_deref().unwrap_or("unknown");
                Ok(SendMessageOutput {
                    success: true,
                    message_id: Some(format!("msg_{}", chrono::Utc::now().timestamp())),
                    details: Some(format!("Message sent to channel {}", channel_id)),
                })
            }
        };

        result
    }

    fn examples(&self) -> Vec<crate::tool::ToolExample<Self::Input, Self::Output>> {
        vec![
            crate::tool::ToolExample {
                description: "Send a message to the user".to_string(),
                parameters: SendMessageInput {
                    target: MessageTarget {
                        target_type: TargetType::User,
                        target_id: None,
                    },
                    content: "Hello! How can I help you today?".to_string(),
                    metadata: None,
                },
                expected_output: Some(SendMessageOutput {
                    success: true,
                    message_id: Some("msg_1234567890".to_string()),
                    details: Some("Message sent to user".to_string()),
                }),
            },
            crate::tool::ToolExample {
                description: "Send a message to another agent".to_string(),
                parameters: SendMessageInput {
                    target: MessageTarget {
                        target_type: TargetType::Agent,
                        target_id: Some("entropy_123".to_string()),
                    },
                    content: "Can you help break down this task?".to_string(),
                    metadata: Some(serde_json::json!({
                        "priority": "high",
                        "context": "task_breakdown"
                    })),
                },
                expected_output: Some(SendMessageOutput {
                    success: true,
                    message_id: Some("msg_1234567891".to_string()),
                    details: Some("Message sent to agent entropy_123".to_string()),
                }),
            },
        ]
    }

    fn usage_rule(&self) -> Option<&'static str> {
        Some("ends your response (yields control) when called")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{UserId, memory::Memory};

    #[tokio::test]
    async fn test_send_message_tool() {
        let memory = Memory::with_owner(UserId::generate());
        let handle = AgentHandle::test_with_memory(memory);

        let tool = SendMessageTool { handle };

        // Test sending to user
        let result = tool
            .execute(SendMessageInput {
                target: MessageTarget {
                    target_type: TargetType::User,
                    target_id: None,
                },
                content: "Test message".to_string(),
                metadata: None,
            })
            .await
            .unwrap();

        assert!(result.success);
        assert!(result.message_id.is_some());
    }
}
