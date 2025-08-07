use chrono::{DateTime, Utc};
use genai::{ModelIden, chat::Usage};
use pattern_macros::Entity;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::sync::Arc;

use crate::{MessageId, UserId, id::RelationId};

// Conversions to/from genai types
mod conversions;

/// A message to be processed by an agent
#[derive(Debug, Clone, Entity, Serialize, Deserialize)]
#[entity(entity_type = "message")]
pub struct Message {
    pub id: MessageId,
    pub role: ChatRole,

    /// The user (human) who initiated this conversation
    /// This helps track message ownership without tying messages to specific agents
    #[serde(skip_serializing_if = "Option::is_none")]
    pub owner_id: Option<UserId>,

    /// Message content stored as flexible object for searchability
    #[entity(db_type = "object")]
    pub content: MessageContent,

    /// Metadata stored as flexible object
    #[entity(db_type = "object")]
    pub metadata: MessageMetadata,

    /// Options stored as flexible object
    #[entity(db_type = "object")]
    pub options: MessageOptions,

    // Precomputed fields for performance
    pub has_tool_calls: bool,
    pub word_count: u32,
    pub created_at: DateTime<Utc>,

    // Embeddings - loaded selectively via custom methods
    #[serde(skip_serializing_if = "Option::is_none")]
    pub embedding: Option<Vec<f32>>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub embedding_model: Option<String>,
}

impl Default for Message {
    fn default() -> Self {
        Self {
            id: MessageId::generate(),
            role: ChatRole::User,
            owner_id: None,
            content: MessageContent::Text(String::new()),
            metadata: MessageMetadata::default(),
            options: MessageOptions::default(),
            has_tool_calls: false,
            word_count: 0,
            created_at: Utc::now(),
            embedding: None,
            embedding_model: None,
        }
    }
}

impl Message {
    /// Estimate word count for content
    fn estimate_word_count(content: &MessageContent) -> u32 {
        match content {
            MessageContent::Text(text) => text.split_whitespace().count() as u32,
            MessageContent::Parts(parts) => parts
                .iter()
                .map(|part| match part {
                    ContentPart::Text(text) => text.split_whitespace().count() as u32,
                    _ => 0,
                })
                .sum(),
            MessageContent::ToolCalls(calls) => calls.len() as u32 * 10, // Estimate
            MessageContent::ToolResponses(responses) => responses
                .iter()
                .map(|r| r.content.split_whitespace().count() as u32)
                .sum(),
            MessageContent::Blocks(blocks) => blocks
                .iter()
                .map(|block| match block {
                    ContentBlock::Text { text } => text.split_whitespace().count() as u32,
                    ContentBlock::Thinking { text, .. } => text.split_whitespace().count() as u32,
                    ContentBlock::RedactedThinking { .. } => 10, // Estimate
                    ContentBlock::ToolUse { .. } => 10,          // Estimate
                    ContentBlock::ToolResult { content, .. } => {
                        content.split_whitespace().count() as u32
                    }
                })
                .sum(),
        }
    }

    /// Convert this message to a genai ChatMessage
    pub fn as_chat_message(&self) -> genai::chat::ChatMessage {
        // Handle Gemini's requirement that ToolResponses must have Tool role
        // If we have ToolResponses with a non-Tool role, fix it
        let role = match (&self.role, &self.content) {
            (role, MessageContent::ToolResponses(_)) if !role.is_tool() => {
                tracing::warn!(
                    "Found ToolResponses with incorrect role {:?}, converting to Tool role",
                    role
                );
                ChatRole::Tool
            }
            _ => self.role.clone(),
        };

        // Debug log to track what content types are being sent
        let content = match &self.content {
            MessageContent::Text(text) => {
                tracing::trace!("Converting Text message with role {:?}", role);
                MessageContent::Text(text.trim().to_string())
            }
            MessageContent::ToolCalls(_) => {
                tracing::trace!("Converting ToolCalls message with role {:?}", role);
                self.content.clone()
            }
            MessageContent::ToolResponses(_) => {
                tracing::debug!("Converting ToolResponses message with role {:?}", role);
                self.content.clone()
            }
            MessageContent::Parts(parts) => match role {
                ChatRole::System | ChatRole::Assistant | ChatRole::Tool => {
                    tracing::debug!("Combining Parts message with role {:?}", role);
                    let string = parts
                        .into_iter()
                        .map(|part| match part {
                            ContentPart::Text(text) => text.trim().to_string(),
                            ContentPart::Image {
                                content_type,
                                source,
                            } => {
                                let source_as_text = match source {
                                    ImageSource::Url(st) => st.trim().to_string(),
                                    ImageSource::Base64(st) => st.trim().to_string(),
                                };
                                format!("{}: {}", content_type, source_as_text)
                            }
                        })
                        .collect::<Vec<_>>()
                        .join("\n---\n");
                    MessageContent::Text(string)
                }
                ChatRole::User => self.content.clone(),
            },
            MessageContent::Blocks(_) => {
                // Blocks are preserved as-is for providers that support them
                tracing::trace!("Preserving Blocks message with role {:?}", role);
                self.content.clone()
            }
        };

        genai::chat::ChatMessage {
            role: role.into(),
            content: content.into(),
            options: Some(self.options.clone().into()),
        }
    }
}

/// Metadata associated with a message
#[derive(Debug, Clone, Serialize, Deserialize, Default, JsonSchema)]
pub struct MessageMetadata {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<chrono::DateTime<chrono::Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub conversation_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub channel_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub guild_id: Option<String>,
    #[serde(flatten)]
    pub custom: serde_json::Value,
}

/// A response generated by an agent
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Request {
    pub system: Option<Vec<String>>,
    pub messages: Vec<Message>,
    pub tools: Option<Vec<genai::chat::Tool>>,
}

impl Request {
    /// Validate that the request has no orphaned tool calls and proper ordering
    pub fn validate(&mut self) -> crate::Result<()> {
        // let mut tool_call_ids = std::collections::HashSet::new();
        // let mut tool_result_ids = std::collections::HashSet::new();

        // //let mut removal_queue = vec![];

        // // Also check for immediate ordering (Anthropic requirement)
        // //
        // tracing::info!("constructing first pass of corrected sequence");
        // let mut messages = Vec::with_capacity(self.messages.capacity());

        // let mut iter = self.messages.iter().peekable();
        // while let Some(msg) = iter.next() {
        //     // Check if this message has tool calls
        //     let msg_tool_calls: Vec<String> = match &msg.content {
        //         MessageContent::ToolCalls(calls) => {
        //             calls.iter().map(|c| c.call_id.clone()).collect()
        //         }
        //         MessageContent::Blocks(blocks) => blocks
        //             .iter()
        //             .filter_map(|b| {
        //                 if let ContentBlock::ToolUse { id, .. } = b {
        //                     Some(id.clone())
        //                 } else {
        //                     None
        //                 }
        //             })
        //             .collect(),
        //         _ => vec![],
        //     };
        //     if !msg_tool_calls.is_empty() {
        //         if let Some(next_msg) = iter.peek() {
        //             let next_msg_results: Vec<String> = match &next_msg.content {
        //                 MessageContent::ToolResponses(responses) => {
        //                     responses.iter().map(|r| r.call_id.clone()).collect()
        //                 }
        //                 MessageContent::Blocks(blocks) => blocks
        //                     .iter()
        //                     .filter_map(|b| {
        //                         if let ContentBlock::ToolResult { tool_use_id, .. } = b {
        //                             Some(tool_use_id.clone())
        //                         } else {
        //                             None
        //                         }
        //                     })
        //                     .collect(),
        //                 _ => vec![],
        //             };

        //             // Collect all IDs for orphan checking
        //             match &msg.content {
        //                 MessageContent::ToolCalls(calls) => {
        //                     for call in calls {
        //                         tool_call_ids.insert(call.call_id.clone());
        //                     }
        //                     if next_msg_results.iter().all(|t| msg_tool_calls.contains(t)) {
        //                         tracing::info!("{:?}\n", msg);
        //                         messages.push(msg.clone());
        //                     } else {
        //                         tracing::warn!("skipping {:?}\n", msg);
        //                     }
        //                 }
        //                 MessageContent::ToolResponses(responses) => {
        //                     tracing::info!("{:?}\n", msg);
        //                     messages.push(msg.clone());
        //                     for response in responses {
        //                         tool_result_ids.insert(response.call_id.clone());
        //                     }
        //                 }
        //                 MessageContent::Blocks(blocks) => {
        //                     if next_msg_results.iter().all(|t| msg_tool_calls.contains(t)) {
        //                         tracing::info!("{:?}\n", msg);
        //                         messages.push(msg.clone());
        //                     } else {
        //                         tracing::warn!("skipping {:?}\n", msg);
        //                     }
        //                     for block in blocks {
        //                         match block {
        //                             ContentBlock::ToolUse { id, .. } => {
        //                                 tool_call_ids.insert(id.clone());
        //                             }
        //                             ContentBlock::ToolResult { tool_use_id, .. } => {
        //                                 tool_result_ids.insert(tool_use_id.clone());
        //                             }
        //                             _ => {}
        //                         }
        //                     }
        //                 }
        //                 _ => {
        //                     tracing::info!("{:?}\n", msg);
        //                     messages.push(msg.clone());
        //                 }
        //             }
        //         } else {
        //             tracing::warn!("skipping {:?}\n", msg);
        //         }
        //     } else {
        //         tracing::info!("{:?}\n", msg);
        //         messages.push(msg.clone());
        //     }
        // }

        // // Check for orphaned tool calls
        // let orphaned_calls: Vec<_> = tool_call_ids.difference(&tool_result_ids).collect();

        // // Also check that tool results don't reference non-existent calls
        // let orphaned_results: Vec<_> = tool_result_ids.difference(&tool_call_ids).collect();

        // // clear messages and we'll insert the known good ones back in.
        // self.messages.clear();

        // tracing::info!("constructing final message sequence");

        // let mut iter = messages.iter().peekable();
        // while let Some(msg) = iter.next() {
        //     // Collect all IDs for orphan checking
        //     let msg_tool_calls: Vec<String> = match &msg.content {
        //         MessageContent::ToolCalls(calls) => {
        //             calls.iter().map(|c| c.call_id.clone()).collect()
        //         }
        //         MessageContent::Blocks(blocks) => blocks
        //             .iter()
        //             .filter_map(|b| {
        //                 if let ContentBlock::ToolUse { id, .. } = b {
        //                     Some(id.clone())
        //                 } else {
        //                     None
        //                 }
        //             })
        //             .collect(),
        //         _ => vec![],
        //     };
        //     let has_orphan = match &msg.content {
        //         MessageContent::ToolCalls(calls) => {
        //             calls.iter().any(|t| orphaned_calls.contains(&&t.call_id))
        //         }
        //         MessageContent::ToolResponses(responses) => responses
        //             .iter()
        //             .any(|t| orphaned_results.contains(&&t.call_id)),
        //         MessageContent::Blocks(blocks) => {
        //             let mut has_orphan = false;
        //             for block in blocks {
        //                 match block {
        //                     ContentBlock::ToolUse { id, .. } => {
        //                         has_orphan = orphaned_calls.contains(&&id);
        //                     }
        //                     ContentBlock::ToolResult { tool_use_id, .. } => {
        //                         has_orphan = orphaned_results.contains(&&tool_use_id);
        //                     }
        //                     _ => {}
        //                 }
        //             }
        //             has_orphan
        //         }
        //         _ => false,
        //     };
        //     if !has_orphan {
        //         if let Some(next_msg) = iter.peek() {
        //             let next_msg_results: Vec<String> = match &next_msg.content {
        //                 MessageContent::ToolResponses(responses) => {
        //                     responses.iter().map(|r| r.call_id.clone()).collect()
        //                 }
        //                 MessageContent::Blocks(blocks) => blocks
        //                     .iter()
        //                     .filter_map(|b| {
        //                         if let ContentBlock::ToolResult { tool_use_id, .. } = b {
        //                             Some(tool_use_id.clone())
        //                         } else {
        //                             None
        //                         }
        //                     })
        //                     .collect(),
        //                 _ => vec![],
        //             };
        //             if next_msg_results.iter().all(|t| msg_tool_calls.contains(t)) {
        //                 tracing::info!("{:?}\n", msg);
        //                 self.messages.push(msg.clone());
        //             } else {
        //                 tracing::warn!("skipping {:?}\n", msg);
        //             }
        //         } else {
        //             tracing::info!("{:?}\n", msg);
        //             self.messages.push(msg.clone());
        //         }
        //     }
        // }
        Ok(())
    }

    /// Convert this request to a genai ChatRequest
    pub fn as_chat_request(&mut self) -> crate::Result<genai::chat::ChatRequest> {
        // Validate before converting
        self.validate()?;

        Ok(
            genai::chat::ChatRequest::from_system(self.system.clone().unwrap().join("\n\n"))
                .append_messages(self.messages.iter().map(|m| m.as_chat_message()).collect())
                .with_tools(self.tools.clone().unwrap_or_default()),
        )
    }
}

/// Message options
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct MessageOptions {
    pub cache_control: Option<CacheControl>,
}

/// Cache control options
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub enum CacheControl {
    Ephemeral,
}

impl From<CacheControl> for MessageOptions {
    fn from(cache_control: CacheControl) -> Self {
        Self {
            cache_control: Some(cache_control),
        }
    }
}

/// Chat roles
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ChatRole {
    System,
    User,
    Assistant,
    Tool,
}

impl std::fmt::Display for ChatRole {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ChatRole::System => write!(f, "system"),
            ChatRole::User => write!(f, "user"),
            ChatRole::Assistant => write!(f, "assistant"),
            ChatRole::Tool => write!(f, "tool"),
        }
    }
}

impl ChatRole {
    /// Check if this is a System role
    pub fn is_system(&self) -> bool {
        matches!(self, ChatRole::System)
    }

    /// Check if this is a User role
    pub fn is_user(&self) -> bool {
        matches!(self, ChatRole::User)
    }

    /// Check if this is an Assistant role
    pub fn is_assistant(&self) -> bool {
        matches!(self, ChatRole::Assistant)
    }

    /// Check if this is a Tool role
    pub fn is_tool(&self) -> bool {
        matches!(self, ChatRole::Tool)
    }
}

/// Message content variants
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub enum MessageContent {
    /// Simple text content
    Text(String),

    /// Multi-part content (text + images)
    Parts(Vec<ContentPart>),

    /// Tool calls from the assistant
    ToolCalls(Vec<ToolCall>),

    /// Tool responses
    ToolResponses(Vec<ToolResponse>),

    /// Content blocks - for providers that need exact block sequence preservation (e.g. Anthropic with thinking)
    Blocks(Vec<ContentBlock>),
}

/// Constructors
impl MessageContent {
    /// Create text content
    pub fn from_text(content: impl Into<String>) -> Self {
        MessageContent::Text(content.into())
    }

    /// Create multi-part content
    pub fn from_parts(parts: impl Into<Vec<ContentPart>>) -> Self {
        MessageContent::Parts(parts.into())
    }

    /// Create tool calls content
    pub fn from_tool_calls(tool_calls: Vec<ToolCall>) -> Self {
        MessageContent::ToolCalls(tool_calls)
    }
}

/// Getters
impl MessageContent {
    /// Get text content if this is a Text variant
    pub fn text(&self) -> Option<&str> {
        match self {
            MessageContent::Text(content) => Some(content.as_str()),
            _ => None,
        }
    }

    /// Consume and return text content if this is a Text variant
    pub fn into_text(self) -> Option<String> {
        match self {
            MessageContent::Text(content) => Some(content),
            _ => None,
        }
    }

    /// Get tool calls if this is a ToolCalls variant
    pub fn tool_calls(&self) -> Option<&[ToolCall]> {
        match self {
            MessageContent::ToolCalls(calls) => Some(calls),
            _ => None,
        }
    }

    /// Check if content is empty
    pub fn is_empty(&self) -> bool {
        match self {
            MessageContent::Text(content) => content.is_empty(),
            MessageContent::Parts(parts) => parts.is_empty(),
            MessageContent::ToolCalls(calls) => calls.is_empty(),
            MessageContent::ToolResponses(responses) => responses.is_empty(),
            MessageContent::Blocks(blocks) => blocks.is_empty(),
        }
    }
}

// From impls for convenience
impl From<&str> for MessageContent {
    fn from(s: &str) -> Self {
        MessageContent::Text(s.to_string())
    }
}

impl From<String> for MessageContent {
    fn from(s: String) -> Self {
        MessageContent::Text(s)
    }
}

impl From<&String> for MessageContent {
    fn from(s: &String) -> Self {
        MessageContent::Text(s.clone())
    }
}

impl From<Vec<ToolCall>> for MessageContent {
    fn from(calls: Vec<ToolCall>) -> Self {
        MessageContent::ToolCalls(calls)
    }
}

impl From<ToolResponse> for MessageContent {
    fn from(response: ToolResponse) -> Self {
        MessageContent::ToolResponses(vec![response])
    }
}

impl From<Vec<ContentPart>> for MessageContent {
    fn from(parts: Vec<ContentPart>) -> Self {
        MessageContent::Parts(parts)
    }
}

/// Content part for multi-modal messages
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub enum ContentPart {
    Text(String),
    Image {
        content_type: String,
        source: ImageSource,
    },
}

impl ContentPart {
    /// Create text part
    pub fn from_text(text: impl Into<String>) -> Self {
        ContentPart::Text(text.into())
    }

    /// Create image part from base64
    pub fn from_image_base64(
        content_type: impl Into<String>,
        content: impl Into<Arc<str>>,
    ) -> Self {
        ContentPart::Image {
            content_type: content_type.into(),
            source: ImageSource::Base64(content.into()),
        }
    }

    /// Create image part from URL
    pub fn from_image_url(content_type: impl Into<String>, url: impl Into<String>) -> Self {
        ContentPart::Image {
            content_type: content_type.into(),
            source: ImageSource::Url(url.into()),
        }
    }
}

impl From<&str> for ContentPart {
    fn from(s: &str) -> Self {
        ContentPart::Text(s.to_string())
    }
}

impl From<String> for ContentPart {
    fn from(s: String) -> Self {
        ContentPart::Text(s)
    }
}

/// Image source
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub enum ImageSource {
    /// URL to the image (not all models support this)
    Url(String),

    /// Base64 encoded image data
    Base64(Arc<str>),
}

/// Tool call from the assistant
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ToolCall {
    pub call_id: String,
    pub fn_name: String,
    pub fn_arguments: Value,
}

/// Tool response
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ToolResponse {
    pub call_id: String,
    pub content: String,
}

/// Content blocks for providers that need exact sequence preservation (e.g. Anthropic with thinking)
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub enum ContentBlock {
    /// Text content
    Text { text: String },
    /// Thinking content (Anthropic)
    Thinking {
        text: String,
        signature: Option<String>,
    },
    /// Redacted thinking content (Anthropic) - encrypted/hidden thinking
    RedactedThinking { data: String },
    /// Tool use request
    ToolUse {
        id: String,
        name: String,
        input: Value,
    },
    /// Tool result response
    ToolResult {
        tool_use_id: String,
        content: String,
    },
}

impl ToolResponse {
    /// Create a new tool response
    pub fn new(call_id: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            call_id: call_id.into(),
            content: content.into(),
        }
    }
}

/// A response generated by an agent
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Response {
    pub content: Vec<MessageContent>,
    pub reasoning: Option<String>,
    pub metadata: ResponseMetadata,
}

impl Response {
    /// Create a Response from a genai ChatResponse
    pub fn from_chat_response(resp: genai::chat::ChatResponse) -> Self {
        // Extract data before consuming resp
        let reasoning = resp.reasoning_content.clone();
        let metadata = ResponseMetadata {
            processing_time: None,
            tokens_used: Some(resp.usage.clone()),
            model_used: Some(resp.provider_model_iden.to_string()),
            confidence: None,
            model_iden: resp.model_iden.clone(),
            custom: resp.captured_raw_body.clone().unwrap_or_default(),
        };

        // Convert genai MessageContent to our MessageContent
        let content: Vec<MessageContent> = resp
            .content
            .clone()
            .into_iter()
            .map(|gc| gc.into())
            .collect();

        Self {
            content,
            reasoning,
            metadata,
        }
    }

    pub fn num_tool_calls(&self) -> usize {
        self.content
            .iter()
            .filter(|c| c.tool_calls().is_some())
            .count()
    }

    pub fn num_tool_responses(&self) -> usize {
        self.content
            .iter()
            .filter(|c| match c {
                MessageContent::ToolResponses(_) => true,
                _ => false,
            })
            .count()
    }

    pub fn has_unpaired_tool_calls(&self) -> bool {
        // Check for unpaired tool calls (tool calls without matching responses)
        let mut tool_call_ids: std::collections::HashSet<String> = self
            .content
            .iter()
            .filter_map(|content| match content {
                MessageContent::ToolCalls(calls) => Some(calls.iter().map(|c| c.call_id.clone())),
                _ => None,
            })
            .flatten()
            .collect();

        // Also check for tool calls inside blocks
        for content in &self.content {
            if let MessageContent::Blocks(blocks) = content {
                for block in blocks {
                    if let ContentBlock::ToolUse { id, .. } = block {
                        tool_call_ids.insert(id.clone());
                    }
                }
            }
        }

        let mut tool_response_ids: std::collections::HashSet<String> = self
            .content
            .iter()
            .filter_map(|content| match content {
                MessageContent::ToolResponses(responses) => {
                    Some(responses.iter().map(|r| r.call_id.clone()))
                }
                _ => None,
            })
            .flatten()
            .collect();

        // Also check for tool responses inside blocks
        for content in &self.content {
            if let MessageContent::Blocks(blocks) = content {
                for block in blocks {
                    if let ContentBlock::ToolResult { tool_use_id, .. } = block {
                        tool_response_ids.insert(tool_use_id.clone());
                    }
                }
            }
        }

        // Check if there are any tool calls without responses
        tool_call_ids.difference(&tool_response_ids).count() > 0
    }

    pub fn only_text(&self) -> String {
        let mut text = String::new();
        for content in &self.content {
            match content {
                MessageContent::Text(txt) => text.push_str(txt),
                MessageContent::Parts(content_parts) => {
                    for part in content_parts {
                        match part {
                            ContentPart::Text(txt) => text.push_str(txt),
                            ContentPart::Image { .. } => {}
                        }
                        text.push('\n');
                    }
                }
                MessageContent::ToolCalls(_) => {}
                MessageContent::ToolResponses(_) => {}
                MessageContent::Blocks(_) => {}
            }
            text.push('\n');
        }
        text
    }
}

/// Metadata for a response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResponseMetadata {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub processing_time: Option<chrono::Duration>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tokens_used: Option<Usage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_used: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f32>,
    pub model_iden: ModelIden,
    pub custom: serde_json::Value,
}

impl Default for ResponseMetadata {
    fn default() -> Self {
        Self {
            processing_time: None,
            tokens_used: None,
            model_used: None,
            confidence: None,
            custom: json!({}),
            model_iden: ModelIden::new(genai::adapter::AdapterKind::Ollama, "default_model"),
        }
    }
}

impl Message {
    /// Create a user message with the given content
    pub fn user(content: impl Into<MessageContent>) -> Self {
        let content = content.into();
        let has_tool_calls = matches!(&content, MessageContent::ToolCalls(_));
        let word_count = Self::estimate_word_count(&content);

        Self {
            id: MessageId::generate(),
            role: ChatRole::User,
            owner_id: None,
            content,
            metadata: MessageMetadata::default(),
            options: MessageOptions::default(),
            has_tool_calls,
            word_count,
            created_at: Utc::now(),
            embedding: None,
            embedding_model: None,
        }
    }

    /// Create a system message with the given content
    pub fn system(content: impl Into<MessageContent>) -> Self {
        let content = content.into();
        let has_tool_calls = matches!(&content, MessageContent::ToolCalls(_));
        let word_count = Self::estimate_word_count(&content);

        Self {
            id: MessageId::generate(),
            role: ChatRole::System,
            owner_id: None,
            content,
            metadata: MessageMetadata::default(),
            options: MessageOptions::default(),
            has_tool_calls,
            word_count,
            created_at: Utc::now(),
            embedding: None,
            embedding_model: None,
        }
    }

    /// Create an agent (assistant) message with the given content
    pub fn agent(content: impl Into<MessageContent>) -> Self {
        let content = content.into();
        let has_tool_calls = matches!(&content, MessageContent::ToolCalls(_));
        let word_count = Self::estimate_word_count(&content);

        Self {
            id: MessageId::generate(),
            role: ChatRole::Assistant,
            owner_id: None,
            content,
            metadata: MessageMetadata::default(),
            options: MessageOptions::default(),
            has_tool_calls,
            word_count,
            created_at: Utc::now(),
            embedding: None,
            embedding_model: None,
        }
    }

    /// Create a tool response message
    pub fn tool(responses: Vec<ToolResponse>) -> Self {
        let content = MessageContent::ToolResponses(responses);
        let word_count = Self::estimate_word_count(&content);

        Self {
            id: MessageId::generate(),
            role: ChatRole::Tool,
            owner_id: None,
            content,
            metadata: MessageMetadata::default(),
            options: MessageOptions::default(),
            has_tool_calls: false,
            word_count,
            created_at: Utc::now(),
            embedding: None,
            embedding_model: None,
        }
    }

    /// Create Messages from an agent Response
    pub fn from_response(response: &Response, agent_id: &crate::AgentId) -> Vec<Self> {
        let mut messages = Vec::new();

        // Group assistant content together, but keep tool responses separate
        let mut current_assistant_content: Vec<MessageContent> = Vec::new();

        for content in &response.content {
            match content {
                MessageContent::ToolResponses(_) => {
                    // First, flush any accumulated assistant content
                    if !current_assistant_content.is_empty() {
                        let combined_content = if current_assistant_content.len() == 1 {
                            current_assistant_content[0].clone()
                        } else {
                            // Combine multiple content items - for now just take the first
                            // TODO: properly combine Text + ToolCalls
                            current_assistant_content[0].clone()
                        };

                        let has_tool_calls =
                            matches!(&combined_content, MessageContent::ToolCalls(_));
                        let word_count = Self::estimate_word_count(&combined_content);

                        messages.push(Self {
                            id: MessageId::generate(),
                            role: ChatRole::Assistant,
                            content: combined_content,
                            metadata: MessageMetadata {
                                user_id: Some(agent_id.to_string()),
                                ..Default::default()
                            },
                            options: MessageOptions::default(),
                            created_at: Utc::now(),
                            owner_id: None,
                            has_tool_calls,
                            word_count,
                            embedding: None,
                            embedding_model: None,
                        });
                        current_assistant_content.clear();
                    }

                    // Then add the tool response as a separate message
                    messages.push(Self {
                        id: MessageId::generate(),
                        role: ChatRole::Tool,
                        content: content.clone(),
                        metadata: MessageMetadata {
                            user_id: Some(agent_id.to_string()),
                            ..Default::default()
                        },
                        options: MessageOptions::default(),
                        created_at: Utc::now(),
                        owner_id: None,
                        has_tool_calls: false,
                        word_count: Self::estimate_word_count(content),
                        embedding: None,
                        embedding_model: None,
                    });
                }
                _ => {
                    // Accumulate assistant content
                    current_assistant_content.push(content.clone());
                }
            }
        }

        // Flush any remaining assistant content
        if !current_assistant_content.is_empty() {
            let combined_content = if current_assistant_content.len() == 1 {
                current_assistant_content[0].clone()
            } else {
                // TODO: properly combine multiple content items
                current_assistant_content[0].clone()
            };

            let has_tool_calls = matches!(&combined_content, MessageContent::ToolCalls(_));
            let word_count = Self::estimate_word_count(&combined_content);

            messages.push(Self {
                id: MessageId::generate(),
                role: ChatRole::Assistant,
                content: combined_content,
                metadata: MessageMetadata {
                    user_id: Some(agent_id.to_string()),
                    ..Default::default()
                },
                options: MessageOptions::default(),
                created_at: Utc::now(),
                owner_id: None,
                has_tool_calls,
                word_count,
                embedding: None,
                embedding_model: None,
            });
        }

        // If response was empty but had reasoning, create a text message
        if messages.is_empty() && response.reasoning.is_some() {
            messages.push(Self {
                id: MessageId::generate(),
                role: ChatRole::Assistant,
                content: MessageContent::Text(response.reasoning.clone().unwrap_or_default()),
                metadata: MessageMetadata {
                    user_id: Some(agent_id.to_string()),
                    ..Default::default()
                },
                options: MessageOptions::default(),
                created_at: Utc::now(),
                owner_id: None,
                has_tool_calls: false,
                word_count: response
                    .reasoning
                    .as_ref()
                    .map(|r| r.split_whitespace().count() as u32)
                    .unwrap_or(0),
                embedding: None,
                embedding_model: None,
            });
        }

        messages
    }

    /// Extract text content from the message if available
    ///
    /// Returns None if the message contains only non-text content (e.g., tool calls)
    pub fn text_content(&self) -> Option<String> {
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

    /// Check if this message contains tool calls
    pub fn has_tool_calls(&self) -> bool {
        matches!(&self.content, MessageContent::ToolCalls(_))
    }

    /// Get the number of tool calls in this message
    pub fn tool_call_count(&self) -> usize {
        match &self.content {
            MessageContent::ToolCalls(calls) => calls.len(),
            _ => 0,
        }
    }

    /// Get the number of tool responses in this message
    pub fn tool_response_count(&self) -> usize {
        match &self.content {
            MessageContent::ToolResponses(calls) => calls.len(),
            _ => 0,
        }
    }

    /// Rough estimation of token count for this message
    ///
    /// Uses the approximation of ~4 characters per token
    pub fn estimate_tokens(&self) -> usize {
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
            MessageContent::Blocks(blocks) => blocks.len() * 50,  // Rough estimate
        }
    }
}

// Search helper methods for MessageContent
impl Message {
    /// Search for messages containing specific text (case-insensitive)
    ///
    /// This generates a SurrealQL query for searching within MessageContent
    pub fn search_text_query(search_term: &str) -> String {
        // Use full-text search operator for better performance and accuracy
        format!(
            r#"SELECT * FROM msg WHERE content @@ "{}""#,
            search_term.replace('"', r#"\""#)
        )
    }

    /// Search for messages with specific tool calls
    pub fn search_tool_calls_query(tool_name: &str) -> String {
        format!(
            r#"SELECT * FROM msg WHERE content.ToolCalls[?fn_name = "{}"]"#,
            tool_name.replace('"', r#"\""#)
        )
    }

    /// Search for messages by role
    pub fn search_by_role_query(role: &ChatRole) -> String {
        format!(r#"SELECT * FROM msg WHERE role = "{}""#, role)
    }

    /// Search for messages within a date range
    pub fn search_by_date_range_query(start: DateTime<Utc>, end: DateTime<Utc>) -> String {
        let start_dt = surrealdb::sql::Datetime::from(start);
        let end_dt = surrealdb::sql::Datetime::from(end);
        format!(
            r#"SELECT * FROM msg WHERE created_at >= {} AND created_at <= {}"#,
            start_dt, end_dt
        )
    }

    /// Search for messages with embeddings
    pub fn search_with_embeddings_query() -> String {
        "SELECT * FROM msg WHERE embedding IS NOT NULL".to_string()
    }

    /// Complex search combining multiple criteria
    pub fn search_complex_query(
        text: Option<&str>,
        role: Option<&ChatRole>,
        has_tool_calls: Option<bool>,
        start_date: Option<DateTime<Utc>>,
        end_date: Option<DateTime<Utc>>,
    ) -> String {
        let mut conditions = Vec::new();

        if let Some(text) = text {
            conditions.push(format!(r#"content @@ "{}""#, text.replace('"', r#"\""#)));
        }

        if let Some(role) = role {
            conditions.push(format!(r#"role = "{}""#, role));
        }

        if let Some(has_tools) = has_tool_calls {
            conditions.push(format!("has_tool_calls = {}", has_tools));
        }

        if let Some(start) = start_date {
            let start_dt = surrealdb::sql::Datetime::from(start);
            conditions.push(format!(r#"created_at >= {}"#, start_dt));
        }

        if let Some(end) = end_date {
            let end_dt = surrealdb::sql::Datetime::from(end);
            conditions.push(format!(r#"created_at <= {}"#, end_dt));
        }

        if conditions.is_empty() {
            "SELECT * FROM msg".to_string()
        } else {
            format!("SELECT * FROM msg WHERE {}", conditions.join(" AND "))
        }
    }
}

// Selective embedding loading
impl Message {
    /// Load a message without embeddings (more efficient for most operations)
    pub async fn load_without_embeddings<C: surrealdb::Connection>(
        db: &surrealdb::Surreal<C>,
        id: &MessageId,
    ) -> Result<Option<Self>, crate::db::DatabaseError> {
        // Use load_with_relations and then clear the embeddings
        let mut message = Self::load_with_relations(db, id).await?;

        if let Some(ref mut msg) = message {
            msg.embedding = None;
            msg.embedding_model = None;
        }

        Ok(message)
    }

    /// Load only the embedding for a message
    pub async fn load_embedding<C: surrealdb::Connection>(
        db: &surrealdb::Surreal<C>,
        id: &MessageId,
    ) -> Result<Option<(Vec<f32>, String)>, crate::db::DatabaseError> {
        let query = r#"SELECT embedding, embedding_model FROM $msg_id"#;

        let mut result = db
            .query(query)
            .bind(("msg_id", surrealdb::RecordId::from(id)))
            .await
            .map_err(crate::db::DatabaseError::QueryFailed)?;

        #[derive(serde::Deserialize)]
        struct EmbeddingResult {
            embedding: Option<Vec<f32>>,
            embedding_model: Option<String>,
        }

        let results: Vec<EmbeddingResult> = result
            .take(0)
            .map_err(crate::db::DatabaseError::QueryFailed)?;

        Ok(results
            .into_iter()
            .next()
            .and_then(|r| match (r.embedding, r.embedding_model) {
                (Some(emb), Some(model)) => Some((emb, model)),
                _ => None,
            }))
    }

    /// Update only the embedding for a message
    pub async fn update_embedding<C: surrealdb::Connection>(
        &self,
        db: &surrealdb::Surreal<C>,
        embedding: Vec<f32>,
        model: String,
    ) -> Result<(), crate::db::DatabaseError> {
        let query = r#"UPDATE $msg_id SET embedding = $embedding, embedding_model = $model"#;

        db.query(query)
            .bind(("msg_id", surrealdb::RecordId::from(&self.id)))
            .bind(("embedding", embedding))
            .bind(("model", model))
            .await
            .map_err(crate::db::DatabaseError::QueryFailed)?;

        Ok(())
    }

    /// Check if content has changed and needs re-embedding
    pub fn needs_reembedding(&self, other_content: &MessageContent) -> bool {
        // Simple content comparison - could be made more sophisticated
        match (&self.content, other_content) {
            (MessageContent::Text(a), MessageContent::Text(b)) => a != b,
            (MessageContent::Parts(a), MessageContent::Parts(b)) => {
                // Extract only text parts for comparison
                let a_texts: Vec<&str> = a
                    .iter()
                    .filter_map(|part| match part {
                        ContentPart::Text(text) => Some(text.as_str()),
                        _ => None,
                    })
                    .collect();

                let b_texts: Vec<&str> = b
                    .iter()
                    .filter_map(|part| match part {
                        ContentPart::Text(text) => Some(text.as_str()),
                        _ => None,
                    })
                    .collect();

                // Different number of text parts or different content
                a_texts != b_texts
            }
            _ => true, // Different content types means re-embedding needed
        }
    }
}

/// Type of relationship between an agent and a message
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MessageRelationType {
    /// Message is in the agent's active context window
    Active,
    /// Message has been compressed/archived to save context
    Archived,
    /// Message is shared from another agent/conversation
    Shared,
}

impl std::fmt::Display for MessageRelationType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Active => write!(f, "active"),
            Self::Archived => write!(f, "archived"),
            Self::Shared => write!(f, "shared"),
        }
    }
}

/// Edge entity for agent-message relationships
///
/// This allows messages to be shared between agents and tracks
/// the relationship type and ordering.
#[derive(Debug, Clone, Entity, Serialize, Deserialize)]
#[entity(entity_type = "agent_messages", edge = true)]
pub struct AgentMessageRelation {
    /// Edge entity ID (generated by SurrealDB)
    pub id: RelationId,

    /// The agent in this relationship
    pub in_id: crate::AgentId,

    /// The message in this relationship
    pub out_id: MessageId,

    /// Type of relationship
    pub message_type: MessageRelationType,

    /// Position in the agent's message history (for ordering)
    /// Stores a Snowflake ID as a string for distributed monotonic ordering
    pub position: String,

    /// When this relationship was created
    pub added_at: DateTime<Utc>,
}

impl Default for AgentMessageRelation {
    fn default() -> Self {
        Self {
            id: RelationId::nil(),
            in_id: crate::AgentId::generate(),
            out_id: MessageId::generate(),
            message_type: MessageRelationType::Active,
            position: "0".to_string(),
            added_at: Utc::now(),
        }
    }
}

#[cfg(test)]
mod relation_tests {
    use super::*;
    use crate::db::entity::DbEntity;

    #[test]
    fn test_agent_message_relation_schema() {
        let schema = AgentMessageRelation::schema();
        println!("AgentMessageRelation schema:\n{}", schema.schema);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_message_conversions() {
        let msg = Message::user("Hello");
        let chat_msg = msg.as_chat_message();
        assert_eq!(chat_msg.content.text().unwrap(), "Hello");
    }

    use crate::db::{client, ops::query_messages_raw};
    use tokio;

    #[tokio::test]
    async fn test_message_entity_storage() {
        let db = client::create_test_db().await.unwrap();

        // Create and store a message
        let msg = Message::user("Test message for database");
        let stored = msg.store_with_relations(&db).await.unwrap();

        // Load it back
        let loaded = Message::load_with_relations(&db, &stored.id).await.unwrap();
        assert!(loaded.is_some());

        let loaded = loaded.unwrap();
        assert_eq!(loaded.id, stored.id);
        assert_eq!(
            loaded.text_content(),
            Some("Test message for database".to_string())
        );
        assert_eq!(loaded.word_count, 4);
        assert!(!loaded.has_tool_calls);
    }

    #[tokio::test]
    async fn test_search_text_query() {
        let db = client::create_test_db().await.unwrap();

        // Store multiple messages
        let msg1 = Message::user("Hello world from Pattern");
        let msg2 = Message::user("Goodbye cruel world");
        let msg3 = Message::agent("I understand your message about the world");

        msg1.store_with_relations(&db).await.unwrap();
        msg2.store_with_relations(&db).await.unwrap();
        msg3.store_with_relations(&db).await.unwrap();

        // Search for "world"
        let query = Message::search_text_query("world");
        let messages = query_messages_raw(&db, &query).await.unwrap();

        assert_eq!(messages.len(), 3);

        // Search for "Pattern"
        let query = Message::search_text_query("Pattern");
        let messages = query_messages_raw(&db, &query).await.unwrap();

        assert_eq!(messages.len(), 1);
        assert!(messages[0].text_content().unwrap().contains("Pattern"));
    }

    #[tokio::test]
    async fn test_search_tool_calls_query() {
        let db = client::create_test_db().await.unwrap();

        // Create messages with and without tool calls
        let msg1 = Message::user("Please send a message");
        let msg2 = Message::agent(MessageContent::ToolCalls(vec![ToolCall {
            call_id: "call_123".to_string(),
            fn_name: "send_message".to_string(),
            fn_arguments: json!({"message": "Hello"}),
        }]));
        let msg3 = Message::agent(MessageContent::ToolCalls(vec![ToolCall {
            call_id: "call_456".to_string(),
            fn_name: "update_memory".to_string(),
            fn_arguments: json!({"key": "test"}),
        }]));

        msg1.store_with_relations(&db).await.unwrap();
        msg2.store_with_relations(&db).await.unwrap();
        msg3.store_with_relations(&db).await.unwrap();

        // Search for send_message tool calls
        let query = Message::search_tool_calls_query("send_message");
        let messages = query_messages_raw(&db, &query).await.unwrap();

        assert_eq!(messages.len(), 1);
        assert!(messages[0].has_tool_calls);

        // Search for update_memory tool calls
        let query = Message::search_tool_calls_query("update_memory");
        let messages = query_messages_raw(&db, &query).await.unwrap();

        assert_eq!(messages.len(), 1);
    }

    #[tokio::test]
    async fn test_search_by_role_query() {
        let db = client::create_test_db().await.unwrap();

        // Create messages with different roles
        let user_msg = Message::user("User message");
        let assistant_msg = Message::agent("Assistant message");
        let system_msg = Message::system("System prompt");

        user_msg.store_with_relations(&db).await.unwrap();
        assistant_msg.store_with_relations(&db).await.unwrap();
        system_msg.store_with_relations(&db).await.unwrap();

        // Search for assistant messages
        let query = Message::search_by_role_query(&ChatRole::Assistant);
        let messages = query_messages_raw(&db, &query).await.unwrap();

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].role, ChatRole::Assistant);

        // Search for user messages
        let query = Message::search_by_role_query(&ChatRole::User);
        let messages = query_messages_raw(&db, &query).await.unwrap();

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].role, ChatRole::User);
    }

    #[tokio::test]
    async fn test_search_by_date_range() {
        let db = client::create_test_db().await.unwrap();

        // We can't easily control created_at in our constructor,
        // so we'll test with a wide range
        let now = Utc::now();
        let start = now - chrono::Duration::hours(1);
        let end = now + chrono::Duration::hours(1);

        // Create some messages
        let msg1 = Message::user("Message 1");
        let msg2 = Message::user("Message 2");

        msg1.store_with_relations(&db).await.unwrap();
        msg2.store_with_relations(&db).await.unwrap();

        // Search within the date range
        let query = Message::search_by_date_range_query(start, end);
        let messages = query_messages_raw(&db, &query).await.unwrap();

        assert_eq!(messages.len(), 2);

        // Search outside the date range
        let past_start = now - chrono::Duration::days(2);
        let past_end = now - chrono::Duration::days(1);

        let query = Message::search_by_date_range_query(past_start, past_end);
        let messages = query_messages_raw(&db, &query).await.unwrap();

        assert_eq!(messages.len(), 0);
    }

    #[tokio::test]
    async fn test_load_without_embeddings() {
        let db = client::create_test_db().await.unwrap();

        // Create a message with embeddings (384 dimensions as expected by the vector index)
        let mut msg = Message::user("Test message with embeddings");
        msg.embedding = Some(vec![0.1; 384]); // 384 dimensions filled with 0.1
        msg.embedding_model = Some("test-model".to_string());

        let stored = msg.store_with_relations(&db).await.unwrap();

        // Load without embeddings
        let loaded = Message::load_without_embeddings(&db, &stored.id)
            .await
            .unwrap();
        assert!(loaded.is_some());

        let loaded = loaded.unwrap();
        assert_eq!(loaded.id, stored.id);
        assert_eq!(
            loaded.text_content(),
            Some("Test message with embeddings".to_string())
        );
        assert!(loaded.embedding.is_none());
        assert!(loaded.embedding_model.is_none());
    }

    #[tokio::test]
    async fn test_load_and_update_embedding() {
        let db = client::create_test_db().await.unwrap();

        // Create a message without embeddings
        let msg = Message::user("Test message for embedding");
        let stored = msg.store_with_relations(&db).await.unwrap();

        // Update with embeddings (384 dimensions as expected by the vector index)
        let embedding = vec![0.5; 384]; // 384 dimensions filled with 0.5
        let model = "test-embed-model".to_string();
        stored
            .update_embedding(&db, embedding.clone(), model.clone())
            .await
            .unwrap();

        // Load just the embedding
        let loaded_embedding = Message::load_embedding(&db, &stored.id).await.unwrap();
        assert!(loaded_embedding.is_some());

        let (loaded_emb, loaded_model) = loaded_embedding.unwrap();
        assert_eq!(loaded_emb, embedding);
        assert_eq!(loaded_model, model);

        // Verify full message has embeddings
        let full_msg = Message::load_with_relations(&db, &stored.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(full_msg.embedding, Some(embedding));
        assert_eq!(full_msg.embedding_model, Some(model));
    }

    #[tokio::test]
    async fn test_search_complex_query() {
        let db = client::create_test_db().await.unwrap();

        // Create various messages
        let msg1 = Message::user("Hello world");
        let mut msg2 = Message::agent("Goodbye world");
        msg2.has_tool_calls = true; // Simulate tool calls
        let msg3 = Message::user("Hello universe");
        let msg4 = Message::system("System message with world");

        msg1.store_with_relations(&db).await.unwrap();
        msg2.store_with_relations(&db).await.unwrap();
        msg3.store_with_relations(&db).await.unwrap();
        msg4.store_with_relations(&db).await.unwrap();

        // Complex search: text "world", role User, no tool calls
        let query = Message::search_complex_query(
            Some("world"),
            Some(&ChatRole::User),
            Some(false),
            None,
            None,
        );
        let messages = query_messages_raw(&db, &query).await.unwrap();

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].text_content(), Some("Hello world".to_string()));

        // Search for messages with tool calls
        let query = Message::search_complex_query(None, None, Some(true), None, None);
        let messages = query_messages_raw(&db, &query).await.unwrap();

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].role, ChatRole::Assistant);
    }

    #[tokio::test]
    async fn test_message_with_parts_content() {
        let db = client::create_test_db().await.unwrap();

        // Create a message with parts
        let parts = vec![
            ContentPart::Text("Check out this image:".to_string()),
            ContentPart::from_image_base64("image/png", "base64encodeddata"),
            ContentPart::Text("Pretty cool, right?".to_string()),
        ];
        let msg = Message::user(MessageContent::Parts(parts));
        let stored = msg.store_with_relations(&db).await.unwrap();

        // Load it back
        let loaded = Message::load_with_relations(&db, &stored.id)
            .await
            .unwrap()
            .unwrap();

        match &loaded.content {
            MessageContent::Parts(parts) => {
                assert_eq!(parts.len(), 3);
                assert!(matches!(&parts[0], ContentPart::Text(t) if t == "Check out this image:"));
                assert!(matches!(&parts[1], ContentPart::Image { .. }));
                assert!(matches!(&parts[2], ContentPart::Text(t) if t == "Pretty cool, right?"));
            }
            _ => panic!("Expected Parts content"),
        }

        // Now that we have full-text search index created in migrations, we can search
        let query = Message::search_text_query("cool");
        let messages = query_messages_raw(&db, &query).await.unwrap();
        assert_eq!(messages.len(), 1);

        // Verify it found the right message
        assert_eq!(
            messages[0].text_content(),
            Some("Check out this image: Pretty cool, right?".to_string())
        );

        // Test searching for text from different parts
        let query = Message::search_text_query("image");
        let messages = query_messages_raw(&db, &query).await.unwrap();
        assert_eq!(messages.len(), 1);
    }

    #[tokio::test]
    async fn test_search_with_embeddings() {
        let db = client::create_test_db().await.unwrap();

        // Create messages with and without embeddings
        let mut msg1 = Message::user("Message with embedding");
        msg1.embedding = Some(vec![0.1; 384]); // 384 dimensions as expected by the vector index
        msg1.embedding_model = Some("model1".to_string());

        let msg2 = Message::user("Message without embedding");

        msg1.store_with_relations(&db).await.unwrap();
        msg2.store_with_relations(&db).await.unwrap();

        // Search for messages with embeddings
        let query = Message::search_with_embeddings_query();
        let messages = query_messages_raw(&db, &query).await.unwrap();

        // Filter out messages that actually have embeddings since SurrealDB might store empty arrays
        let messages_with_embeddings: Vec<_> = messages
            .into_iter()
            .filter(|msg| msg.embedding.is_some() && !msg.embedding.as_ref().unwrap().is_empty())
            .collect();

        assert_eq!(messages_with_embeddings.len(), 1);
        assert_eq!(
            messages_with_embeddings[0].text_content(),
            Some("Message with embedding".to_string())
        );
    }

    #[test]
    fn test_request_validation_with_orphaned_tool_calls() {
        // Create a request with an orphaned tool call
        let mut request = Request {
            system: Some(vec!["System prompt".to_string()]),
            messages: vec![
                Message::user("Hello"),
                Message::agent(MessageContent::ToolCalls(vec![ToolCall {
                    call_id: "test_call_1".to_string(),
                    fn_name: "test_tool".to_string(),
                    fn_arguments: serde_json::json!({}),
                }])),
                // Missing tool response for test_call_1
                Message::user("What happened?"),
            ],
            tools: None,
        };

        // Should fail validation
        let result = request.validate();
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("orphaned tool call IDs"));
    }

    #[test]
    fn test_request_validation_with_matched_tool_calls() {
        // Create a request with properly matched tool calls
        let mut request = Request {
            system: Some(vec!["System prompt".to_string()]),
            messages: vec![
                Message::user("Hello"),
                Message::agent(MessageContent::ToolCalls(vec![ToolCall {
                    call_id: "test_call_1".to_string(),
                    fn_name: "test_tool".to_string(),
                    fn_arguments: serde_json::json!({}),
                }])),
                Message::tool(vec![ToolResponse {
                    call_id: "test_call_1".to_string(),
                    content: "Tool result".to_string(),
                }]),
                Message::user("What happened?"),
            ],
            tools: None,
        };

        // Should pass validation
        let result = request.validate();
        assert!(result.is_ok());
    }

    #[test]
    fn test_request_validation_with_orphaned_tool_results() {
        // Create a request with an orphaned tool result
        let mut request = Request {
            system: Some(vec!["System prompt".to_string()]),
            messages: vec![
                Message::user("Hello"),
                Message::tool(vec![ToolResponse {
                    call_id: "non_existent_call".to_string(),
                    content: "Tool result".to_string(),
                }]),
            ],
            tools: None,
        };

        // Should fail validation
        let result = request.validate();
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("orphaned tool result IDs"));
    }
}
