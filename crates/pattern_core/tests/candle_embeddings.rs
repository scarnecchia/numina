#![cfg(feature = "embed-candle")]

use pattern_core::embeddings::EmbeddingProvider;
use pattern_core::embeddings::candle::CandleEmbedder;

#[tokio::test]
async fn test_candle_embedder_creation() {
    let embedder = CandleEmbedder::new("jinaai/jina-embeddings-v2-small-en", None)
        .await
        .unwrap();
    assert_eq!(embedder.dimensions(), 512);
    assert_eq!(embedder.model_id(), "jinaai/jina-embeddings-v2-small-en");
}

#[tokio::test]
async fn test_candle_embed() {
    let embedder = CandleEmbedder::new("jinaai/jina-embeddings-v2-small-en", None)
        .await
        .unwrap();
    let embedding = embedder.embed("test text").await.unwrap();
    assert_eq!(embedding.dimensions, 512);
    assert_eq!(embedding.vector.len(), 512);
}
