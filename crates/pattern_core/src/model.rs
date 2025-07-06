use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::fmt::Debug;

use crate::Result;

/// A model provider that can generate completions
#[async_trait]
pub trait ModelProvider: Send + Sync + Debug {
    /// Get the name of this provider
    fn name(&self) -> &str;

    /// List available models from this provider
    async fn list_models(&self) -> Result<Vec<ModelInfo>>;

    /// Generate a completion
    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse>;

    /// Check if a model supports a specific capability
    fn supports_capability(&self, model: &str, capability: ModelCapability) -> bool;

    /// Estimate token count for a prompt
    async fn count_tokens(&self, model: &str, content: &str) -> Result<usize>;
}

/// Information about an available model
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelInfo {
    pub id: String,
    pub name: String,
    pub provider: String,
    pub capabilities: Vec<ModelCapability>,
    pub context_window: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_output_tokens: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cost_per_1k_prompt_tokens: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cost_per_1k_completion_tokens: Option<f64>,
}

/// Capabilities that a model might support
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ModelCapability {
    /// Basic text generation
    TextGeneration,

    /// Can call functions/tools
    FunctionCalling,

    /// Supports system prompts
    SystemPrompt,

    /// Can process images
    VisionInput,

    /// Can generate images
    ImageGeneration,

    /// Supports streaming responses
    Streaming,

    /// Can handle long contexts (>32k tokens)
    LongContext,

    /// Supports JSON mode for structured output
    JsonMode,

    /// Can do web browsing
    WebBrowsing,

    /// Can execute code
    CodeExecution,

    /// Fine-tunable model
    FineTuning,
}

/// A request to generate a completion
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompletionRequest {
    pub model: String,
    pub messages: Vec<ChatMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stop_sequences: Option<Vec<String>>,
    pub stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<ToolDefinition>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_choice: Option<ToolChoice>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_format: Option<ResponseFormat>,
}

/// A chat message in a conversation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: ChatRole,
    pub content: MessageContent,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

/// The role of a chat message
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ChatRole {
    System,
    User,
    Assistant,
    Tool,
}

/// Content of a message (can be text or multimodal)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum MessageContent {
    Text(String),
    Parts(Vec<ContentPart>),
}

/// A part of multimodal content
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentPart {
    Text {
        text: String,
    },
    Image {
        image_url: ImageUrl,
    },
    ToolCall {
        id: String,
        name: String,
        arguments: String,
    },
    ToolResult {
        tool_call_id: String,
        content: String,
    },
}

/// An image URL for multimodal input
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageUrl {
    pub url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<ImageDetail>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ImageDetail {
    Auto,
    Low,
    High,
}

/// Definition of a tool that can be called
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
}

/// How to choose which tool to call
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ToolChoice {
    None,
    Auto,
    Required,
    Tool { name: String },
}

/// Format for the response
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ResponseFormat {
    Text,
    JsonObject,
    JsonSchema { schema: serde_json::Value },
}

/// Response from a completion request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompletionResponse {
    pub id: String,
    pub model: String,
    pub choices: Vec<CompletionChoice>,
    pub usage: TokenUsage,
    pub created: chrono::DateTime<chrono::Utc>,
}

/// A single completion choice
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompletionChoice {
    pub index: usize,
    pub message: ChatMessage,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finish_reason: Option<FinishReason>,
}

/// Why the model stopped generating
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FinishReason {
    Stop,
    Length,
    ToolCalls,
    ContentFilter,
}

/// Token usage statistics
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct TokenUsage {
    pub prompt_tokens: usize,
    pub completion_tokens: usize,
    pub total_tokens: usize,
}

impl Default for CompletionRequest {
    fn default() -> Self {
        Self {
            model: String::new(),
            messages: Vec::new(),
            max_tokens: None,
            temperature: None,
            top_p: None,
            stop_sequences: None,
            stream: false,
            tools: None,
            tool_choice: None,
            response_format: None,
        }
    }
}

impl ChatMessage {
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: ChatRole::System,
            content: MessageContent::Text(content.into()),
            name: None,
        }
    }

    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: ChatRole::User,
            content: MessageContent::Text(content.into()),
            name: None,
        }
    }

    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            role: ChatRole::Assistant,
            content: MessageContent::Text(content.into()),
            name: None,
        }
    }

    pub fn tool(id: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            role: ChatRole::Tool,
            content: MessageContent::Parts(vec![ContentPart::ToolResult {
                tool_call_id: id.into(),
                content: content.into(),
            }]),
            name: None,
        }
    }
}
