//! Extension traits and helpers for working with genai chat types

use genai::chat::{ChatMessage, ChatRole, ContentPart, MessageContent};

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
            MessageContent::Blocks(blocks) => {
                // Convert blocks to string representation
                blocks
                    .iter()
                    .map(|block| match block {
                        genai::chat::ContentBlock::Text { text } => text.clone(),
                        genai::chat::ContentBlock::Thinking { text, .. } => {
                            format!("[Thinking: {}]", text)
                        }
                        genai::chat::ContentBlock::ToolUse { name, .. } => {
                            format!("[Tool use: {}]", name)
                        }
                        genai::chat::ContentBlock::ToolResult { .. } => "[Tool result]".to_string(),
                        genai::chat::ContentBlock::RedactedThinking { .. } => {
                            "[Thinking: <redacted>]".to_string()
                        }
                    })
                    .collect::<Vec<_>>()
                    .join(" ")
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
        options: None,
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
