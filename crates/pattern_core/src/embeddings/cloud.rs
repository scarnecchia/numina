//! Cloud-based embedding providers (OpenAI, Cohere, etc.)

use super::{Embedding, EmbeddingError, EmbeddingProvider, Result, validate_input};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

/// OpenAI embedding provider
pub struct OpenAIEmbedder {
    model: String,
    api_key: String,
    dimensions: Option<usize>,
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

        // TODO: Implement actual OpenAI API call
        // For now, return a dummy embedding
        let dimensions = self.get_dimensions();
        let vector = vec![0.1; dimensions];

        Ok(Embedding::new(vector, self.model.clone()))
    }

    async fn embed_batch(&self, texts: &[String]) -> Result<Vec<Embedding>> {
        validate_input(texts)?;

        if texts.len() > self.max_batch_size() {
            return Err(EmbeddingError::BatchSizeTooLarge {
                size: texts.len(),
                max: self.max_batch_size(),
            });
        }

        // TODO: Implement batch embedding with OpenAI API
        let mut embeddings = Vec::with_capacity(texts.len());
        for text in texts {
            embeddings.push(self.embed(text).await?);
        }

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

        // TODO: Implement actual Cohere API call
        // For now, return a dummy embedding
        let dimensions = self.get_dimensions();
        let vector = vec![0.1; dimensions];

        Ok(Embedding::new(vector, self.model.clone()))
    }

    async fn embed_batch(&self, texts: &[String]) -> Result<Vec<Embedding>> {
        validate_input(texts)?;

        if texts.len() > self.max_batch_size() {
            return Err(EmbeddingError::BatchSizeTooLarge {
                size: texts.len(),
                max: self.max_batch_size(),
            });
        }

        // TODO: Implement batch embedding with Cohere API
        let mut embeddings = Vec::with_capacity(texts.len());
        for text in texts {
            embeddings.push(self.embed(text).await?);
        }

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
#[derive(Debug, Serialize)]
struct OpenAIEmbeddingRequest {
    input: Vec<String>,
    model: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    dimensions: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct OpenAIEmbeddingResponse {
    data: Vec<OpenAIEmbeddingData>,
    usage: OpenAIUsage,
}

#[derive(Debug, Deserialize)]
struct OpenAIEmbeddingData {
    embedding: Vec<f32>,
    index: usize,
}

#[derive(Debug, Deserialize)]
struct OpenAIUsage {
    prompt_tokens: usize,
    total_tokens: usize,
}

#[derive(Debug, Serialize)]
struct CohereEmbeddingRequest {
    texts: Vec<String>,
    model: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    input_type: Option<String>,
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
        let result = embedder.embed("test text").await.unwrap();
        assert_eq!(result.dimensions, 1536);
        assert_eq!(result.vector.len(), 1536);
    }

    #[tokio::test]
    async fn test_cohere_embed() {
        let embedder = CohereEmbedder::new(
            "embed-english-v3.0".to_string(),
            "test-key".to_string(),
            None,
        );
        let result = embedder.embed("test text").await.unwrap();
        assert_eq!(result.dimensions, 1024);
        assert_eq!(result.vector.len(), 1024);
    }
}
