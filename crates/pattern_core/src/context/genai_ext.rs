//! Extension traits and helpers for working with genai chat types

use genai::chat::{ChatMessage, ChatRole, ContentPart, MessageContent};

/// Extension trait for ChatMessage
pub trait ChatMessageExt {
    /// Get the text content from a message, if any
    fn text_content(&self) -> Option<String>;

    /// Check if this message has tool calls
    fn has_tool_calls(&self) -> bool;

    /// Get the number of tool calls in this message
    fn tool_call_count(&self) -> usize;

    /// Estimate the token count for this message
    fn estimate_tokens(&self) -> usize;
}

impl ChatMessageExt for ChatMessage {
    fn text_content(&self) -> Option<String> {
        match &self.content {
            MessageContent::Text(text) => Some(text.clone()),
            MessageContent::Parts(parts) => {
                // Concatenate all text parts
                let text_parts: Vec<String> = parts
                    .iter()
                    .filter_map(|part| match part {
                        ContentPart::Text(text) => Some(text.clone()),
                        _ => None,
                    })
                    .collect();

                if text_parts.is_empty() {
                    None
                } else {
                    Some(text_parts.join(" "))
                }
            }
            _ => None,
        }
    }

    fn has_tool_calls(&self) -> bool {
        matches!(&self.content, MessageContent::ToolCalls(_))
    }

    fn tool_call_count(&self) -> usize {
        match &self.content {
            MessageContent::ToolCalls(calls) => calls.len(),
            _ => 0,
        }
    }

    fn estimate_tokens(&self) -> usize {
        // Rough estimation: ~4 chars per token
        match &self.content {
            MessageContent::Text(text) => (text.len() as f32 / 4.0) as usize,
            MessageContent::Parts(parts) => {
                parts
                    .iter()
                    .map(|part| match part {
                        ContentPart::Text(text) => (text.len() as f32 / 4.0) as usize,
                        _ => 25, // Rough estimate for non-text content
                    })
                    .sum()
            }
            MessageContent::ToolCalls(calls) => calls.len() * 50, // Rough estimate
            MessageContent::ToolResponses(responses) => responses.len() * 30, // Rough estimate
        }
    }
}

/// Extension trait for MessageContent
pub trait MessageContentExt {
    /// Convert to string representation
    fn to_string(&self) -> String;
}

impl MessageContentExt for MessageContent {
    fn to_string(&self) -> String {
        match self {
            MessageContent::Text(text) => text.clone(),
            MessageContent::Parts(parts) => parts
                .iter()
                .map(|part| match part {
                    ContentPart::Text(text) => text.clone(),
                    ContentPart::Image { .. } => "[Image]".to_string(),
                })
                .collect::<Vec<_>>()
                .join(" "),
            MessageContent::ToolCalls(calls) => {
                format!("[Tool calls: {}]", calls.len())
            }
            MessageContent::ToolResponses(responses) => {
                format!("[Tool responses: {}]", responses.len())
            }
        }
    }
}

/// Extension trait for ChatRole
pub trait ChatRoleExt {
    /// Check if this is an assistant role
    fn is_assistant(&self) -> bool;

    /// Check if this is a user role
    fn is_user(&self) -> bool;

    /// Check if this is a system role
    fn is_system(&self) -> bool;
}

impl ChatRoleExt for ChatRole {
    fn is_assistant(&self) -> bool {
        matches!(self, ChatRole::Assistant)
    }

    fn is_user(&self) -> bool {
        matches!(self, ChatRole::User)
    }

    fn is_system(&self) -> bool {
        matches!(self, ChatRole::System)
    }
}

/// Helper to create a simple text message
pub fn text_message(role: ChatRole, content: impl Into<String>) -> ChatMessage {
    ChatMessage {
        role,
        content: MessageContent::Text(content.into()),
    }
}

/// Helper to extract text from various content types
pub fn extract_text_from_content(content: &MessageContent) -> Option<String> {
    match content {
        MessageContent::Text(text) => Some(text.clone()),
        MessageContent::Parts(parts) => {
            let texts: Vec<String> = parts
                .iter()
                .filter_map(|part| match part {
                    ContentPart::Text(text) => Some(text.clone()),
                    ContentPart::Image { .. } => None,
                })
                .collect();

            if texts.is_empty() {
                None
            } else {
                Some(texts.join(" "))
            }
        }
        _ => None,
    }
}
