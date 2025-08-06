use async_trait::async_trait;
use genai::{adapter::AdapterKind, chat::ChatOptions};
use serde::{Deserialize, Serialize};
use std::fmt::Debug;

use crate::{
    Result,
    message::{Request, Response},
};

pub mod defaults;

/// A model provider that can generate completions
#[async_trait]
pub trait ModelProvider: Send + Sync + Debug {
    /// Get the name of this provider
    fn name(&self) -> &str;

    /// List available models from this provider
    async fn list_models(&self) -> Result<Vec<ModelInfo>>;

    /// Generate a completion
    async fn complete(&self, options: &ResponseOptions, mut request: Request) -> Result<Response>;

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

/// Options for configuring model responses
///
/// This struct contains all the parameters that can be used to control
/// how a language model generates its response, including sampling parameters,
/// output format, and what information to capture.
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
    pub fn new(model_info: ModelInfo) -> Self {
        // Calculate appropriate max_tokens based on model
        let max_tokens = Some(defaults::calculate_max_tokens(&model_info, None));

        Self {
            model_info,
            temperature: Some(0.7),
            max_tokens,
            top_p: None,
            stop_sequences: vec![],
            capture_usage: None,
            capture_content: None,
            capture_reasoning_content: None,
            capture_tool_calls: None,
            capture_raw_body: None,
            response_format: None,
            normalize_reasoning_content: None,
            reasoning_effort: None,
        }
    }
    /// Convert ResponseOptions to a tuple of (ModelInfo, ChatOptions) for use with genai
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
                extra_headers: None,
                seed: None,
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

    /// Bash tool
    BashTool,

    /// Can search the web
    WebSearch,

    /// Text editor tool
    TextEdit,

    /// Computer use
    ComputerUse,

    /// Can execute code
    CodeExecution,

    /// Fine-tunable model
    FineTuning,

    /// Extended Thinking
    ExtendedThinking,
}

/// A client for interacting with language models through the genai library
///
/// This wraps the genai::Client and provides a consistent interface for
/// model interactions across different providers (OpenAI, Anthropic, etc.)
#[derive(Debug, Clone)]
pub struct GenAiClient {
    client: genai::Client,
    available_endpoints: Vec<AdapterKind>,
}

impl GenAiClient {
    /// Create a new GenAiClient with the default configuration
    /// This will use environment variables for API keys (OPENAI_API_KEY, ANTHROPIC_API_KEY, etc.)
    pub async fn new() -> Result<Self> {
        // Create default client - OAuth support will be added by the caller if needed
        let client = genai::Client::default();

        // Discover available endpoints based on configured API keys
        let mut available_endpoints = Vec::new();

        // Check which providers have API keys configured
        // Only include Anthropic if API key is available
        // OAuth cases will use with_endpoints() to explicitly add it
        if std::env::var("ANTHROPIC_API_KEY").is_ok() {
            available_endpoints.push(AdapterKind::Anthropic);
        }
        if std::env::var("GEMINI_API_KEY").is_ok() {
            available_endpoints.push(AdapterKind::Gemini);
        }
        if std::env::var("OPENAI_API_KEY").is_ok() {
            available_endpoints.push(AdapterKind::OpenAI);
        }
        if std::env::var("GROQ_API_KEY").is_ok() {
            available_endpoints.push(AdapterKind::Groq);
        }
        if std::env::var("COHERE_API_KEY").is_ok() {
            available_endpoints.push(AdapterKind::Cohere);
        }

        Ok(Self {
            client,
            available_endpoints,
        })
    }

    /// Create a new GenAiClient with specific endpoints
    pub fn with_endpoints(client: genai::Client, endpoints: Vec<AdapterKind>) -> Self {
        Self {
            client,
            available_endpoints: endpoints,
        }
    }
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
            let models = match self.client.all_model_names(*endpoint).await {
                Ok(models) => models,
                Err(e) => {
                    tracing::debug!("Failed to list models for {}: {}", endpoint, e);
                    continue;
                }
            };

            for model in models {
                // Try to resolve the service target - this validates authentication
                match self.client.resolve_service_target(&model).await {
                    Ok(_) => {
                        // Model is accessible, continue
                    }
                    Err(e) => {
                        // Authentication failed for this model, skip it
                        tracing::debug!("Skipping model {} due to auth error: {}", model, e);
                        continue;
                    }
                }

                // Create basic ModelInfo from provider
                let model_info = ModelInfo {
                    provider: endpoint.to_string(),
                    id: model.clone(),
                    name: model,
                    capabilities: vec![],
                    max_output_tokens: None,
                    cost_per_1k_completion_tokens: None,
                    cost_per_1k_prompt_tokens: None,
                    context_window: 0, // Will be fixed by enhance_model_info
                };

                // Enhance with proper defaults
                model_strings.push(defaults::enhance_model_info(model_info));
            }
        }

        Ok(model_strings)
    }

    /// Generate a completion
    async fn complete(&self, options: &ResponseOptions, mut request: Request) -> Result<Response> {
        let (model_info, chat_options) = options.to_chat_options_tuple();

        // Log the full request
        let chat_request = request.as_chat_request()?;
        tracing::debug!("Chat Request:\n{:#?}", chat_request);

        let response = match self
            .client
            .exec_chat(&model_info.id, chat_request, Some(&chat_options))
            .await
        {
            Ok(response) => {
                tracing::debug!("GenAI Response:\n{:#?}", response);
                response
            }
            Err(e) => {
                crate::log_error!("GenAI API error", e);
                return Err(crate::CoreError::model_error("genai", &model_info.id, e));
            }
        };

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
        use crate::message::MessageContent;

        Ok(Response {
            content: vec![MessageContent::from_text(&self.response)],
            reasoning: None,
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
