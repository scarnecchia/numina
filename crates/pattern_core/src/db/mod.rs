//! Database backend abstraction for Pattern
//!
//! This module provides traits and implementations for:
//! - Database connectivity (embedded and remote)
//! - Vector storage and similarity search
//! - Schema management and migrations

use async_trait::async_trait;
use miette::Diagnostic;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use thiserror::Error;

pub mod embedded;
pub mod migration;
pub mod ops;
pub mod schema;

/// Core database error type
#[derive(Error, Debug, Diagnostic)]
pub enum DatabaseError {
    #[error("Connection failed")]
    #[diagnostic(help("Check your database configuration and ensure the database is running"))]
    ConnectionFailed(#[source] Box<dyn std::error::Error + Send + Sync>),

    #[error("Query failed")]
    #[diagnostic(help("Check the query syntax and table schema"))]
    QueryFailed(#[source] Box<dyn std::error::Error + Send + Sync>),

    #[error("Transaction failed")]
    TransactionFailed(#[source] Box<dyn std::error::Error + Send + Sync>),

    #[error("Embedding model mismatch: database has {db_model}, config specifies {config_model}")]
    #[diagnostic(help(
        "To change embedding models, you must re-embed all data. Consider creating a new database or running a migration."
    ))]
    EmbeddingModelMismatch {
        db_model: String,
        config_model: String,
    },

    #[error("Schema version mismatch: database is at v{db_version}, code expects v{code_version}")]
    #[diagnostic(help("Run migrations to update the database schema"))]
    SchemaVersionMismatch { db_version: u32, code_version: u32 },

    #[error("Record not found: {entity}")]
    NotFound { entity: String },

    #[error("Invalid vector dimensions: expected {expected}, got {actual}")]
    #[diagnostic(help("Ensure all embeddings use the same model and dimensions"))]
    InvalidVectorDimensions { expected: usize, actual: usize },
}

pub type Result<T> = std::result::Result<T, DatabaseError>;

/// Configuration for database backends
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DatabaseConfig {
    Embedded {
        #[serde(default = "default_db_path")]
        path: String,
        #[serde(default)]
        strict_mode: bool,
    },
    #[cfg(feature = "surreal-remote")]
    Remote {
        url: String,
        #[serde(default)]
        username: Option<String>,
        #[serde(default)]
        password: Option<String>,
        namespace: String,
        database: String,
    },
}

fn default_db_path() -> String {
    "./pattern.db".to_string()
}

impl Default for DatabaseConfig {
    fn default() -> Self {
        DatabaseConfig::Embedded {
            path: default_db_path(),
            strict_mode: false,
        }
    }
}

/// A database query result
#[derive(Debug)]
pub struct QueryResponse {
    pub affected_rows: usize,
    pub data: serde_json::Value,
}

/// Search result from vector similarity search
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VectorSearchResult {
    pub id: String,
    pub score: f32,
    pub data: serde_json::Value,
}

/// Filter for vector searches
#[derive(Debug, Clone, Default)]
pub struct SearchFilter {
    pub where_clause: Option<String>,
    pub params: Vec<(String, serde_json::Value)>,
}

/// Core database operations
#[async_trait]
pub trait DatabaseBackend: Send + Sync {
    /// Connect to the database with the given configuration
    async fn connect(config: DatabaseConfig) -> Result<Arc<Self>>
    where
        Self: Sized;

    /// Execute a raw query
    async fn execute(
        &self,
        query: &str,
        params: Vec<(String, serde_json::Value)>,
    ) -> Result<QueryResponse>;

    /// Check if the database is healthy
    async fn health_check(&self) -> Result<()>;

    /// Get the current schema version
    async fn schema_version(&self) -> Result<u32>;
}

/// Database operations that require generics (not object-safe)
#[async_trait]
pub trait DatabaseOperations: DatabaseBackend {
    /// Begin a transaction
    async fn transaction<F, R>(&self, f: F) -> Result<R>
    where
        F: FnOnce(Arc<dyn Transaction>) -> Result<R> + Send,
        R: Send;
}

/// Transaction operations
#[async_trait]
pub trait Transaction: Send + Sync {
    /// Execute a query within the transaction
    async fn execute(
        &self,
        query: &str,
        params: Vec<(String, serde_json::Value)>,
    ) -> Result<QueryResponse>;

    /// Commit the transaction
    async fn commit(self: Box<Self>) -> Result<()>;

    /// Rollback the transaction
    async fn rollback(self: Box<Self>) -> Result<()>;
}

/// Vector storage and search operations
#[async_trait]
pub trait VectorStore: DatabaseBackend + DatabaseOperations {
    /// Search for similar vectors
    async fn vector_search(
        &self,
        table: &str,
        embedding_field: &str,
        query_vector: &[f32],
        limit: usize,
        filter: Option<SearchFilter>,
    ) -> Result<Vec<VectorSearchResult>>;

    /// Create a vector index
    async fn create_vector_index(
        &self,
        table: &str,
        field: &str,
        dimensions: usize,
        distance_metric: DistanceMetric,
    ) -> Result<()>;

    /// Check if a vector index exists
    async fn vector_index_exists(&self, table: &str, field: &str) -> Result<bool>;
}

/// Distance metrics for vector similarity
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DistanceMetric {
    Cosine,
    Euclidean,
    Manhattan,
}

impl DistanceMetric {
    pub fn as_surreal_string(&self) -> &'static str {
        match self {
            DistanceMetric::Cosine => "COSINE",
            DistanceMetric::Euclidean => "EUCLIDEAN",
            DistanceMetric::Manhattan => "MANHATTAN",
        }
    }
}

/// System metadata stored in the database
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemMetadata {
    pub embedding_model: String,
    pub embedding_dimensions: usize,
    pub schema_version: u32,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

/// Load system metadata from the database
pub async fn load_metadata<DB: DatabaseBackend>(db: &DB) -> Result<Option<SystemMetadata>> {
    let response = db
        .execute("SELECT * FROM system_metadata LIMIT 1", vec![])
        .await?;

    if let Some(data) = response.data.as_array().and_then(|arr| arr.first()) {
        Ok(Some(
            serde_json::from_value(data.clone())
                .map_err(|e| DatabaseError::QueryFailed(Box::new(e)))?,
        ))
    } else {
        Ok(None)
    }
}

/// Initialize or validate the database schema
pub async fn initialize_schema<DB: DatabaseBackend>(
    db: &DB,
    embedding_model: &str,
    embedding_dimensions: usize,
) -> Result<()> {
    let metadata = load_metadata(db).await?;

    if let Some(metadata) = metadata {
        if metadata.embedding_model != embedding_model {
            return Err(DatabaseError::EmbeddingModelMismatch {
                db_model: metadata.embedding_model,
                config_model: embedding_model.to_string(),
            });
        }
        if metadata.embedding_dimensions != embedding_dimensions {
            return Err(DatabaseError::InvalidVectorDimensions {
                expected: metadata.embedding_dimensions,
                actual: embedding_dimensions,
            });
        }
    } else {
        // First time setup
        create_metadata(db, embedding_model, embedding_dimensions).await?;
    }

    Ok(())
}

/// Create initial system metadata
async fn create_metadata<DB: DatabaseBackend>(
    db: &DB,
    embedding_model: &str,
    embedding_dimensions: usize,
) -> Result<()> {
    let now = chrono::Utc::now();
    let metadata = SystemMetadata {
        embedding_model: embedding_model.to_string(),
        embedding_dimensions,
        schema_version: 1,
        created_at: now,
        updated_at: now,
    };

    db.execute(
        "CREATE system_metadata CONTENT $metadata",
        vec![(
            "metadata".to_string(),
            serde_json::to_value(&metadata).unwrap(),
        )],
    )
    .await?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_distance_metric_serialization() {
        let metric = DistanceMetric::Cosine;
        let json = serde_json::to_string(&metric).unwrap();
        assert_eq!(json, "\"cosine\"");

        let parsed: DistanceMetric = serde_json::from_str(&json).unwrap();
        assert!(matches!(parsed, DistanceMetric::Cosine));
    }

    #[test]
    fn test_database_config_default() {
        let config = DatabaseConfig::default();
        match config {
            DatabaseConfig::Embedded { path, strict_mode } => {
                assert_eq!(path, "./pattern.db");
                assert!(!strict_mode);
            }
            #[cfg(feature = "surreal-remote")]
            _ => panic!("Expected embedded config"),
        }
    }
}
