use async_trait::async_trait;
use genai::{adapter::AdapterKind, chat::ChatOptions};
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
    async fn complete(&self, options: &ResponseOptions, request: Request) -> Result<Response>;

    /// Check if a model supports a specific capability
    async fn supports_capability(&self, model: &str, capability: ModelCapability) -> bool;

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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResponseOptions {
    pub model_info: ModelInfo,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f64>,
    pub stop_sequences: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub capture_usage: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub capture_content: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub capture_reasoning_content: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub capture_tool_calls: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub capture_raw_body: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_format: Option<genai::chat::ChatResponseFormat>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub normalize_reasoning_content: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_effort: Option<genai::chat::ReasoningEffort>,
}

impl ResponseOptions {
    pub fn to_chat_options_tuple(&self) -> (ModelInfo, ChatOptions) {
        (
            self.model_info.clone(),
            ChatOptions {
                temperature: self.temperature,
                top_p: self.top_p,
                max_tokens: self.max_tokens,
                stop_sequences: self.stop_sequences.clone(),
                capture_content: self.capture_content,
                capture_raw_body: self.capture_raw_body,
                capture_reasoning_content: self.capture_reasoning_content,
                reasoning_effort: self.reasoning_effort.clone(),
                normalize_reasoning_content: self.normalize_reasoning_content,
                response_format: self.response_format.clone(),
                capture_usage: self.capture_usage,
                capture_tool_calls: self.capture_tool_calls,
            },
        )
    }
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

#[derive(Debug, Clone)]
pub struct GenAiClient {
    client: genai::Client,
    available_endpoints: Vec<AdapterKind>,
}

#[async_trait]
impl ModelProvider for GenAiClient {
    fn name(&self) -> &str {
        "genai::Client"
    }

    /// List available models from this provider
    async fn list_models(&self) -> Result<Vec<ModelInfo>> {
        let mut model_strings = Vec::new();
        for endpoint in &self.available_endpoints {
            let models = self
                .client
                .all_model_names(*endpoint)
                .await
                .expect("Fix error handling here");

            for model in models {
                let model_iden = self
                    .client
                    .resolve_service_target(&model)
                    .await
                    .expect("Fix error handling here");
                model_strings.push(ModelInfo {
                    provider: endpoint.to_string(),
                    id: model_iden.model.to_string(),
                    name: model,
                    capabilities: vec![
                        ModelCapability::FunctionCalling,
                        ModelCapability::TextGeneration,
                        ModelCapability::SystemPrompt,
                    ],
                    max_output_tokens: None,
                    cost_per_1k_completion_tokens: None,
                    cost_per_1k_prompt_tokens: None,
                    context_window: 200000,
                });
            }
        }

        Ok(model_strings)
    }

    /// Generate a completion
    async fn complete(&self, options: &ResponseOptions, request: Request) -> Result<Response> {
        let (model_info, chat_options) = options.to_chat_options_tuple();
        let response = self
            .client
            .exec_chat(
                &model_info.id,
                request.as_chat_request(),
                Some(&chat_options),
            )
            .await
            .expect("handle errors for genai");

        Ok(Response::from_chat_response(response))
    }

    /// Check if a model supports a specific capability
    async fn supports_capability(&self, _model: &str, _capability: ModelCapability) -> bool {
        true
    }

    /// Estimate token count for a prompt
    async fn count_tokens(&self, _model: &str, content: &str) -> Result<usize> {
        Ok(content.len() / 4 as usize)
    }
}

/// Mock model provider for testing
#[cfg(test)]
#[derive(Debug, Clone)]
pub struct MockModelProvider {
    pub response: String,
}

#[cfg(test)]
#[async_trait]
impl ModelProvider for MockModelProvider {
    fn name(&self) -> &str {
        "mock"
    }

    async fn list_models(&self) -> Result<Vec<ModelInfo>> {
        Ok(vec![ModelInfo {
            id: "mock-model".to_string(),
            name: "Mock Model".to_string(),
            provider: "mock".to_string(),
            capabilities: vec![
                ModelCapability::TextGeneration,
                ModelCapability::FunctionCalling,
                ModelCapability::SystemPrompt,
            ],
            context_window: 8192,
            max_output_tokens: Some(4096),
            cost_per_1k_prompt_tokens: Some(0.0),
            cost_per_1k_completion_tokens: Some(0.0),
        }])
    }

    async fn complete(&self, _options: &ResponseOptions, _request: Request) -> Result<Response> {
        use genai::chat::MessageContent;

        Ok(Response {
            content: vec![MessageContent::from_text(&self.response)],
            reasoning: None,
            tool_calls: Vec::new(),
            metadata: Default::default(),
        })
    }

    async fn supports_capability(&self, _model: &str, _capability: ModelCapability) -> bool {
        true
    }

    async fn count_tokens(&self, _model: &str, content: &str) -> Result<usize> {
        Ok(content.len() / 4)
    }
}
