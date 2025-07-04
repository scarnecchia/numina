use crate::agent::{constellation::MultiAgentSystem, AgentId, UserId};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tracing::{debug, warn};

// Use chrono from sqlx since it's already a dependency
use sqlx::types::chrono;

/// Message types to distinguish sources
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum MessageSource {
    User(i64), // Store raw user ID since UserId doesn't implement Serialize
    Agent(AgentId),
    System,
    Tool { name: String },
}

/// Tagged message with metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaggedMessage {
    pub content: String,
    pub source: MessageSource,
    pub is_broadcast: bool,
    pub requires_response: bool,
    pub timestamp: chrono::DateTime<chrono::Utc>,
}

impl TaggedMessage {
    /// Create a user message
    pub fn from_user(user_id: UserId, content: String) -> Self {
        Self {
            content,
            source: MessageSource::User(user_id.0),
            is_broadcast: false,
            requires_response: true,
            timestamp: chrono::Utc::now(),
        }
    }

    /// Create an agent message
    pub fn from_agent(agent_id: AgentId, content: String, requires_response: bool) -> Self {
        Self {
            content,
            source: MessageSource::Agent(agent_id),
            is_broadcast: false,
            requires_response,
            timestamp: chrono::Utc::now(),
        }
    }

    /// Create a system message
    pub fn from_system(content: String) -> Self {
        Self {
            content,
            source: MessageSource::System,
            is_broadcast: false,
            requires_response: false,
            timestamp: chrono::Utc::now(),
        }
    }

    /// Create a tool result message
    pub fn from_tool(tool_name: String, content: String) -> Self {
        Self {
            content,
            source: MessageSource::Tool { name: tool_name },
            is_broadcast: false,
            requires_response: false,
            timestamp: chrono::Utc::now(),
        }
    }

    /// Check if message should trigger a response
    pub fn should_respond(&self, agent_id: &AgentId) -> bool {
        match &self.source {
            MessageSource::User(_) => true,
            MessageSource::Agent(sender_id) => sender_id != agent_id && self.requires_response,
            MessageSource::System => false,
            MessageSource::Tool { .. } => false,
        }
    }

    /// Format message with source information
    pub fn format_with_source(&self) -> String {
        match &self.source {
            MessageSource::User(id) => {
                format!("[USER {}] {}", id, self.content)
            }
            MessageSource::Agent(id) => {
                if self.requires_response {
                    format!("[AGENT {} - NEEDS RESPONSE] {}", id.as_str(), self.content)
                } else {
                    format!("[AGENT {} - INFO ONLY] {}", id.as_str(), self.content)
                }
            }
            MessageSource::System => {
                format!("[SYSTEM] {}", self.content)
            }
            MessageSource::Tool { name } => {
                format!("[TOOL {} RESULT] {}", name, self.content)
            }
        }
    }
}

/// Agent coordination utilities
#[derive(Debug)]
pub struct AgentCoordinator {
    system: Arc<MultiAgentSystem>,
}

impl AgentCoordinator {
    pub fn new(system: Arc<MultiAgentSystem>) -> Self {
        Self { system }
    }

    /// Send a tagged message between agents
    pub async fn send_tagged_message(
        &self,
        user_id: UserId,
        from_agent: &AgentId,
        to_agent: &AgentId,
        content: String,
        requires_response: bool,
    ) -> crate::error::Result<String> {
        let tagged_msg = TaggedMessage::from_agent(from_agent.clone(), content, requires_response);

        // Format the message with metadata
        let formatted_message = tagged_msg.format_with_source();

        // Send via the regular system (for now)
        // In the future, we could store tagged messages separately
        self.system
            .send_message_to_agent(user_id, Some(to_agent.as_str()), &formatted_message)
            .await
    }

    /// Broadcast a message to all agents except sender
    pub async fn broadcast_message(
        &self,
        user_id: UserId,
        from_agent: &AgentId,
        content: String,
    ) -> crate::error::Result<()> {
        let tagged_msg = TaggedMessage {
            content: content.clone(),
            source: MessageSource::Agent(from_agent.clone()),
            is_broadcast: true,
            requires_response: false,
            timestamp: chrono::Utc::now(),
        };

        let formatted_message = format!(
            "[BROADCAST FROM {} - NO RESPONSE NEEDED] {}",
            from_agent.as_str(),
            tagged_msg.content
        );

        // Send to all agents except the sender
        for (agent_id, _config) in self.system.agent_configs() {
            if agent_id != from_agent {
                match self
                    .system
                    .send_message_to_agent(user_id, Some(agent_id.as_str()), &formatted_message)
                    .await
                {
                    Ok(_) => debug!("Broadcast sent to {}", agent_id.as_str()),
                    Err(e) => warn!("Failed to broadcast to {}: {}", agent_id.as_str(), e),
                }
            }
        }

        Ok(())
    }

    /// Filter tool responses from message history
    pub fn filter_tool_responses(messages: Vec<String>) -> Vec<String> {
        messages
            .into_iter()
            .filter(|msg| {
                // Filter out common tool responses
                !msg.contains("Message sent successfully")
                    && !msg.contains("successfully sent")
                    && !msg.contains("[TOOL")
                    && !msg.contains("Tool execution completed")
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::StandardAgent;

    #[test]
    fn test_message_tagging() {
        let user_msg = TaggedMessage::from_user(UserId(1), "Hello".to_string());
        assert_eq!(user_msg.source, MessageSource::User(1));
        assert!(user_msg.requires_response);

        let agent_msg =
            TaggedMessage::from_agent(StandardAgent::Pattern.id(), "Update".to_string(), false);
        assert!(!agent_msg.requires_response);

        let tool_msg = TaggedMessage::from_tool("send_message".to_string(), "Success".to_string());
        assert!(!tool_msg.requires_response);
    }

    #[test]
    fn test_should_respond() {
        let pattern_id = StandardAgent::Pattern.id();
        let entropy_id = StandardAgent::Entropy.id();

        // User messages always get responses
        let user_msg = TaggedMessage::from_user(UserId(1), "Hello".to_string());
        assert!(user_msg.should_respond(&pattern_id));

        // Agent messages with requires_response=true from different agent
        let agent_msg =
            TaggedMessage::from_agent(entropy_id.clone(), "Need help".to_string(), true);
        assert!(agent_msg.should_respond(&pattern_id));

        // Agent messages from self should not get response
        let self_msg = TaggedMessage::from_agent(pattern_id.clone(), "My update".to_string(), true);
        assert!(!self_msg.should_respond(&pattern_id));

        // Info-only agent messages
        let info_msg = TaggedMessage::from_agent(entropy_id, "FYI".to_string(), false);
        assert!(!info_msg.should_respond(&pattern_id));

        // System and tool messages never get responses
        let system_msg = TaggedMessage::from_system("Update".to_string());
        assert!(!system_msg.should_respond(&pattern_id));

        let tool_msg = TaggedMessage::from_tool("test".to_string(), "Done".to_string());
        assert!(!tool_msg.should_respond(&pattern_id));
    }

    #[test]
    fn test_message_formatting() {
        let user_msg = TaggedMessage::from_user(UserId(42), "Hello".to_string());
        assert_eq!(user_msg.format_with_source(), "[USER 42] Hello");

        let agent_msg =
            TaggedMessage::from_agent(StandardAgent::Flux.id(), "Time update".to_string(), true);
        assert_eq!(
            agent_msg.format_with_source(),
            "[AGENT flux - NEEDS RESPONSE] Time update"
        );

        let info_msg =
            TaggedMessage::from_agent(StandardAgent::Archive.id(), "Stored".to_string(), false);
        assert_eq!(
            info_msg.format_with_source(),
            "[AGENT archive - INFO ONLY] Stored"
        );
    }

    #[test]
    fn test_filter_tool_responses() {
        let messages = vec![
            "Hello from user".to_string(),
            "Message sent successfully".to_string(),
            "What's next?".to_string(),
            "[TOOL send_message RESULT] Done".to_string(),
            "Real content here".to_string(),
        ];

        let filtered = AgentCoordinator::filter_tool_responses(messages);
        assert_eq!(filtered.len(), 3);
        assert!(filtered.contains(&"Hello from user".to_string()));
        assert!(filtered.contains(&"What's next?".to_string()));
        assert!(filtered.contains(&"Real content here".to_string()));
    }
}
