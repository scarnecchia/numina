//! Cloud-based embedding providers (OpenAI, Cohere, etc.)

use super::{Embedding, EmbeddingError, EmbeddingProvider, Result, validate_input};
use async_trait::async_trait;
use serde::Deserialize;

/// OpenAI embedding provider
///
#[derive(Clone)]
pub struct OpenAIEmbedder {
    model: String,
    api_key: String,
    dimensions: Option<usize>,
}

impl std::fmt::Debug for OpenAIEmbedder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OpenAIEmbedder")
            .field("model", &self.model)
            .field("dimensions", &self.dimensions)
            .field("api_key", &"[REDACTED]")
            .finish()
    }
}

impl OpenAIEmbedder {
    /// Create a new OpenAI embedder
    pub fn new(model: String, api_key: String, dimensions: Option<usize>) -> Self {
        Self {
            model,
            api_key,
            dimensions,
        }
    }

    /// Get the actual dimensions for the model
    fn get_dimensions(&self) -> usize {
        self.dimensions.unwrap_or(match self.model.as_str() {
            "text-embedding-3-small" => 1536,
            "text-embedding-3-large" => 3072,
            "text-embedding-ada-002" => 1536,
            _ => 1536,
        })
    }
}

#[async_trait]
impl EmbeddingProvider for OpenAIEmbedder {
    async fn embed(&self, text: &str) -> Result<Embedding> {
        if text.trim().is_empty() {
            return Err(EmbeddingError::EmptyInput);
        }

        let embeddings = self.embed_batch(&[text.to_string()]).await?;
        embeddings
            .into_iter()
            .next()
            .ok_or_else(|| EmbeddingError::GenerationFailed("No embedding returned".into()))
    }

    async fn embed_batch(&self, texts: &[String]) -> Result<Vec<Embedding>> {
        validate_input(texts)?;

        if texts.len() > self.max_batch_size() {
            return Err(EmbeddingError::BatchSizeTooLarge {
                size: texts.len(),
                max: self.max_batch_size(),
            });
        }

        let client = reqwest::Client::new();
        let mut request_body = serde_json::json!({
            "model": self.model,
            "input": texts,
        });

        if let Some(dims) = self.dimensions {
            request_body["dimensions"] = serde_json::json!(dims);
        }

        let response = client
            .post("https://api.openai.com/v1/embeddings")
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&request_body)
            .send()
            .await
            .map_err(|e| EmbeddingError::ApiError(format!("Request failed: {}", e)))?;

        if !response.status().is_success() {
            let error_text = response
                .text()
                .await
                .unwrap_or_else(|_| "Unknown error".to_string());
            return Err(EmbeddingError::ApiError(format!(
                "OpenAI API error: {}",
                error_text
            )));
        }

        let response_data: OpenAIEmbeddingResponse = response
            .json()
            .await
            .map_err(|e| EmbeddingError::ApiError(format!("Failed to parse response: {}", e)))?;

        // Sort by index to ensure correct order
        let mut indexed_embeddings: Vec<_> = response_data.data.into_iter().collect();
        indexed_embeddings.sort_by_key(|item| item.index);

        let embeddings = indexed_embeddings
            .into_iter()
            .map(|item| Embedding::new(item.embedding, self.model.clone()))
            .collect();

        Ok(embeddings)
    }

    fn model_id(&self) -> &str {
        &self.model
    }

    fn dimensions(&self) -> usize {
        self.get_dimensions()
    }

    fn max_batch_size(&self) -> usize {
        2048 // OpenAI's batch limit
    }
}

/// Cohere embedding provider
pub struct CohereEmbedder {
    model: String,
    api_key: String,
    input_type: Option<String>,
}

impl std::fmt::Debug for CohereEmbedder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CohereEmbedder")
            .field("model", &self.model)
            .field("input_type", &self.input_type)
            .field("api_key", &"[REDACTED]")
            .finish()
    }
}

impl CohereEmbedder {
    /// Create a new Cohere embedder
    pub fn new(model: String, api_key: String, input_type: Option<String>) -> Self {
        Self {
            model,
            api_key,
            input_type,
        }
    }

    /// Get the dimensions for the model
    fn get_dimensions(&self) -> usize {
        match self.model.as_str() {
            "embed-english-v3.0" => 1024,
            "embed-multilingual-v3.0" => 1024,
            "embed-english-light-v3.0" => 384,
            "embed-multilingual-light-v3.0" => 384,
            _ => 1024,
        }
    }
}

#[async_trait]
impl EmbeddingProvider for CohereEmbedder {
    async fn embed(&self, text: &str) -> Result<Embedding> {
        if text.trim().is_empty() {
            return Err(EmbeddingError::EmptyInput);
        }

        let embeddings = self.embed_batch(&[text.to_string()]).await?;
        embeddings
            .into_iter()
            .next()
            .ok_or_else(|| EmbeddingError::GenerationFailed("No embedding returned".into()))
    }

    async fn embed_batch(&self, texts: &[String]) -> Result<Vec<Embedding>> {
        validate_input(texts)?;

        if texts.len() > self.max_batch_size() {
            return Err(EmbeddingError::BatchSizeTooLarge {
                size: texts.len(),
                max: self.max_batch_size(),
            });
        }

        let client = reqwest::Client::new();
        let mut request_body = serde_json::json!({
            "model": self.model,
            "texts": texts,
        });

        if let Some(ref input_type) = self.input_type {
            request_body["input_type"] = serde_json::json!(input_type);
        }

        let response = client
            .post("https://api.cohere.ai/v1/embed")
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&request_body)
            .send()
            .await
            .map_err(|e| EmbeddingError::ApiError(format!("Request failed: {}", e)))?;

        if !response.status().is_success() {
            let error_text = response
                .text()
                .await
                .unwrap_or_else(|_| "Unknown error".to_string());
            return Err(EmbeddingError::ApiError(format!(
                "Cohere API error: {}",
                error_text
            )));
        }

        let response_data: CohereEmbeddingResponse = response
            .json()
            .await
            .map_err(|e| EmbeddingError::ApiError(format!("Failed to parse response: {}", e)))?;

        let embeddings = response_data
            .embeddings
            .into_iter()
            .map(|embedding| Embedding::new(embedding, self.model.clone()))
            .collect();

        Ok(embeddings)
    }

    fn model_id(&self) -> &str {
        &self.model
    }

    fn dimensions(&self) -> usize {
        self.get_dimensions()
    }

    fn max_batch_size(&self) -> usize {
        96 // Cohere's batch limit
    }
}

/// Request/response types for API calls

#[derive(Debug, Deserialize)]
struct OpenAIEmbeddingResponse {
    data: Vec<OpenAIEmbeddingData>,
    #[allow(dead_code)]
    usage: OpenAIUsage,
}

#[derive(Debug, Deserialize)]
struct OpenAIEmbeddingData {
    embedding: Vec<f32>,
    index: usize,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct OpenAIUsage {
    prompt_tokens: usize,
    total_tokens: usize,
}

#[derive(Debug, Deserialize)]
struct CohereEmbeddingResponse {
    embeddings: Vec<Vec<f32>>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_openai_dimensions() {
        let embedder = OpenAIEmbedder::new(
            "text-embedding-3-small".to_string(),
            "test-key".to_string(),
            None,
        );
        assert_eq!(embedder.dimensions(), 1536);

        let embedder_custom = OpenAIEmbedder::new(
            "text-embedding-3-small".to_string(),
            "test-key".to_string(),
            Some(768),
        );
        assert_eq!(embedder_custom.dimensions(), 768);
    }

    #[test]
    fn test_cohere_dimensions() {
        let embedder = CohereEmbedder::new(
            "embed-english-v3.0".to_string(),
            "test-key".to_string(),
            None,
        );
        assert_eq!(embedder.dimensions(), 1024);

        let embedder_light = CohereEmbedder::new(
            "embed-english-light-v3.0".to_string(),
            "test-key".to_string(),
            None,
        );
        assert_eq!(embedder_light.dimensions(), 384);
    }

    #[tokio::test]
    async fn test_openai_embed() {
        let embedder = OpenAIEmbedder::new(
            "text-embedding-3-small".to_string(),
            "test-key".to_string(),
            None,
        );
        // Don't actually call the API in tests
        assert_eq!(embedder.model_id(), "text-embedding-3-small");
        assert_eq!(embedder.dimensions(), 1536);
    }

    #[tokio::test]
    async fn test_cohere_embed() {
        let embedder = CohereEmbedder::new(
            "embed-english-v3.0".to_string(),
            "test-key".to_string(),
            None,
        );
        // Don't actually call the API in tests
        assert_eq!(embedder.model_id(), "embed-english-v3.0");
        assert_eq!(embedder.dimensions(), 1024);
    }
}
