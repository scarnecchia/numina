//! LLM provider abstraction for agents

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::fmt::Debug;

use crate::Result;

/// Trait for LLM providers that agents can use to generate responses
#[async_trait]
pub trait LlmProvider: Send + Sync + Debug {
    /// Generate a response given a system prompt and user message
    async fn generate_response(
        &self,
        system_prompt: &str,
        user_message: &str,
        temperature: Option<f32>,
        max_tokens: Option<u32>,
    ) -> Result<String>;

    /// Generate a response with tool calls
    async fn generate_with_tools(
        &self,
        system_prompt: &str,
        user_message: &str,
        tools: Vec<ToolDefinition>,
        temperature: Option<f32>,
        max_tokens: Option<u32>,
    ) -> Result<LlmResponse>;

    /// Get the model name
    fn model_name(&self) -> &str;

    /// Check if the provider supports tool calls
    fn supports_tools(&self) -> bool {
        true
    }

    /// Get the context window size
    fn context_window(&self) -> usize {
        4096 // Conservative default
    }
}

/// Definition of a tool that can be called by the LLM
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
}

/// Response from an LLM that may include tool calls
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmResponse {
    pub content: String,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub tool_calls: Vec<ToolCall>,
}

/// A tool call requested by the LLM
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: serde_json::Value,
}

/// Mock LLM provider for testing
#[derive(Debug)]
pub struct MockLlmProvider {
    pub response: String,
}

#[async_trait]
impl LlmProvider for MockLlmProvider {
    async fn generate_response(
        &self,
        _system_prompt: &str,
        _user_message: &str,
        _temperature: Option<f32>,
        _max_tokens: Option<u32>,
    ) -> Result<String> {
        Ok(self.response.clone())
    }

    async fn generate_with_tools(
        &self,
        _system_prompt: &str,
        _user_message: &str,
        _tools: Vec<ToolDefinition>,
        _temperature: Option<f32>,
        _max_tokens: Option<u32>,
    ) -> Result<LlmResponse> {
        Ok(LlmResponse {
            content: self.response.clone(),
            tool_calls: vec![],
        })
    }

    fn model_name(&self) -> &str {
        "mock"
    }
}
