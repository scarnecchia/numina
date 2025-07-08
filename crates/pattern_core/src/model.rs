use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::fmt::Debug;

use crate::{
    Result,
    message::{Request, Response},
};

/// A model provider that can generate completions
#[async_trait]
pub trait ModelProvider: Send + Sync + Debug {
    /// Get the name of this provider
    fn name(&self) -> &str;

    /// List available models from this provider
    async fn list_models(&self) -> Result<Vec<ModelInfo>>;

    /// Generate a completion
    async fn complete(&self, request: Request) -> Result<Response>;

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
