//! Message sending tool for agents

use async_trait::async_trait;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{Result, context::AgentHandle, tool::AiTool};

use super::MessageTarget;

/// Input parameters for sending a message
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub struct SendMessageInput {
    /// The target to send the message to
    pub target: MessageTarget,

    /// The message content
    pub content: String,

    /// Optional metadata for the message
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
}

/// Output from send message operation
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct SendMessageOutput {
    /// Whether the message was sent successfully
    pub success: bool,

    /// Unique identifier for the sent message
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message_id: Option<String>,

    /// Any additional information about the send operation
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<String>,
}

/// Tool for sending messages to various targets
#[derive(Debug, Clone)]
pub struct SendMessageTool {
    pub handle: AgentHandle,
}

#[async_trait]
impl AiTool for SendMessageTool {
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

        match params.target {
            MessageTarget::User => {
                // Send to user through default channel
                Ok(SendMessageOutput {
                    success: true,
                    message_id: Some(format!("msg_{}", chrono::Utc::now().timestamp())),
                    details: Some("Message sent to user".to_string()),
                })
            }
            MessageTarget::Agent { agent_id } => {
                // Send to another agent
                Ok(SendMessageOutput {
                    success: true,
                    message_id: Some(format!("msg_{}", chrono::Utc::now().timestamp())),
                    details: Some(format!("Message sent to agent {}", agent_id)),
                })
            }
            MessageTarget::Group { group_id } => {
                // Send to a group
                Ok(SendMessageOutput {
                    success: true,
                    message_id: Some(format!("msg_{}", chrono::Utc::now().timestamp())),
                    details: Some(format!("Message sent to group {}", group_id)),
                })
            }
            MessageTarget::Channel { channel_id } => {
                // Send to a specific channel
                Ok(SendMessageOutput {
                    success: true,
                    message_id: Some(format!("msg_{}", chrono::Utc::now().timestamp())),
                    details: Some(format!("Message sent to channel {}", channel_id)),
                })
            }
        }
    }

    fn examples(&self) -> Vec<crate::tool::ToolExample<Self::Input, Self::Output>> {
        vec![
            crate::tool::ToolExample {
                description: "Send a message to the user".to_string(),
                parameters: SendMessageInput {
                    target: MessageTarget::User,
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
                    target: MessageTarget::Agent {
                        agent_id: "entropy_123".to_string(),
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{UserId, memory::Memory};

    #[tokio::test]
    async fn test_send_message_tool() {
        let memory = Memory::with_owner(UserId::generate());
        let handle = AgentHandle {
            memory,
            ..Default::default()
        };

        let tool = SendMessageTool { handle };

        // Test sending to user
        let result = tool
            .execute(SendMessageInput {
                target: MessageTarget::User,
                content: "Test message".to_string(),
                metadata: None,
            })
            .await
            .unwrap();

        assert!(result.success);
        assert!(result.message_id.is_some());
    }
}
