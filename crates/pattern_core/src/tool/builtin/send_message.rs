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

    /// Request another turn after this tool executes
    #[serde(default)]
    pub request_heartbeat: bool,
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
        "Send a message to the user, another agent, a group, or a specific channel, or as a post on bluesky. This is the primary way to communicate."
    }

    async fn execute(&self, params: Self::Input) -> Result<Self::Output> {
        // Get the message router from the handle
        let router =
            self.handle
                .message_router()
                .ok_or_else(|| crate::CoreError::ToolExecutionFailed {
                    tool_name: "send_message".to_string(),
                    cause: "Message router not configured for this agent".to_string(),
                    parameters: serde_json::to_value(&params).unwrap_or_default(),
                })?;

        // Handle agent name resolution if target is agent type
        let target = params.target.clone();

        let (reason, content) = if matches!(params.target.target_type, TargetType::Agent) {
            let split: Vec<_> = params.content.splitn(2, &['\n', '|', '-']).collect();

            let reason = if split.len() == 1 {
                split.first().unwrap_or(&"")
            } else {
                "send_message_invocation"
            };
            (reason, split.last().unwrap_or(&"").to_string())
        } else {
            ("send_message_invocation", params.content)
        };
        // When agent uses send_message tool, origin is the agent itself
        let origin = crate::context::message_router::MessageOrigin::Agent {
            agent_id: router.agent_id().clone(),
            name: router.agent_name().clone(),
            reason: reason.to_string(),
        };

        // Send the message through the router
        match router
            .send_message(target, content.clone(), params.metadata.clone(), Some(origin))
            .await
        {
            Ok(created_uri) => {
                // Generate a message ID for tracking
                let message_id = format!("msg_{}", chrono::Utc::now().timestamp_millis());

                // Build details based on target type and whether it was a like
                let details = match params.target.target_type {
                    TargetType::User => {
                        if let Some(id) = &params.target.target_id {
                            format!("Message sent to user {}", id)
                        } else {
                            "Message sent to user".to_string()
                        }
                    }
                    TargetType::Agent => {
                        format!(
                            "Message queued for agent {}",
                            params.target.target_id.as_deref().unwrap_or("unknown")
                        )
                    }
                    TargetType::Group => {
                        format!(
                            "Message sent to group {}",
                            params.target.target_id.as_deref().unwrap_or("unknown")
                        )
                    }
                    TargetType::Channel => {
                        format!(
                            "Message sent to channel {}",
                            params.target.target_id.as_deref().unwrap_or("default")
                        )
                    }
                    TargetType::Bluesky => {
                        // Check if this was a "like" action
                        let is_like = content.trim().eq_ignore_ascii_case("like");
                        
                        if let Some(uri) = created_uri.as_ref().or(params.target.target_id.as_ref()) {
                            if is_like {
                                // Check if the URI indicates this was a like (contains "app.bsky.feed.like")
                                if uri.contains("app.bsky.feed.like") {
                                    format!("Liked Bluesky post: {}", uri)
                                } else {
                                    // Fallback if we sent "like" but didn't get a like URI back
                                    format!("Like action on Bluesky post: {}", params.target.target_id.as_deref().unwrap_or("unknown"))
                                }
                            } else {
                                format!("Reply sent to Bluesky post: {}", uri)
                            }
                        } else {
                            "Message posted to Bluesky".to_string()
                        }
                    }
                };

                Ok(SendMessageOutput {
                    success: true,
                    message_id: Some(message_id),
                    details: Some(details),
                })
            }
            Err(e) => {
                // Log the error for debugging
                tracing::error!("Failed to send message: {:?}", e);

                Ok(SendMessageOutput {
                    success: false,
                    message_id: None,
                    details: Some(format!("Failed to send message: {:?}", e)),
                })
            }
        }
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
                    request_heartbeat: false,
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
                    request_heartbeat: false,
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
        Some("the conversation will end when called")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        UserId, context::message_router::AgentMessageRouter, db::client::create_test_db,
        memory::Memory,
    };

    #[tokio::test]
    async fn test_send_message_tool() {
        let db = create_test_db().await.unwrap();

        let memory = Memory::with_owner(&UserId::generate());
        let mut handle = AgentHandle::test_with_memory(memory).with_db(db.clone());

        let router = AgentMessageRouter::new(handle.agent_id.clone(), handle.name.clone(), db);
        handle.message_router = Some(router);

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
                request_heartbeat: false,
            })
            .await
            .unwrap();

        assert!(result.success);
        assert!(result.message_id.is_some());
    }
}
