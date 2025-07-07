//! Embedded SurrealDB implementation

use super::*;
use async_trait::async_trait;
use std::sync::Arc;
use surrealdb::Surreal;
use surrealdb::engine::local::{Db, SurrealKv};
use tokio::sync::RwLock;

/// Embedded SurrealDB backend using SurrealKV (pure Rust)
pub struct EmbeddedDatabase {
    client: Surreal<Db>,
    #[allow(dead_code)]
    path: String,
}

impl EmbeddedDatabase {
    async fn new(path: String, _strict_mode: bool) -> Result<Self> {
        let client = Surreal::new::<SurrealKv>(path.as_str())
            .await
            .map_err(|e| DatabaseError::ConnectionFailed(Box::new(e)))?;

        // Use a namespace and database
        client
            .use_ns("pattern")
            .use_db("main")
            .await
            .map_err(|e| DatabaseError::ConnectionFailed(Box::new(e)))?;

        Ok(Self { client, path })
    }
}

#[async_trait]
impl DatabaseBackend for EmbeddedDatabase {
    async fn connect(config: DatabaseConfig) -> Result<Arc<Self>>
    where
        Self: Sized,
    {
        match config {
            DatabaseConfig::Embedded { path, strict_mode } => {
                Ok(Arc::new(Self::new(path, strict_mode).await?))
            }
            #[cfg(feature = "surreal-remote")]
            _ => Err(DatabaseError::ConnectionFailed(
                "Expected embedded configuration".into(),
            )),
        }
    }

    async fn execute(
        &self,
        query: &str,
        params: Vec<(String, serde_json::Value)>,
    ) -> Result<QueryResponse> {
        let mut surrealdb_query = self.client.query(query);

        // Bind parameters
        for (name, value) in params {
            surrealdb_query = surrealdb_query.bind((name, value));
        }

        let mut response = surrealdb_query
            .await
            .map_err(|e| DatabaseError::QueryFailed(Box::new(e)))?;

        // Get the first statement result
        let data: surrealdb::Value = response
            .take(0)
            .map_err(|e| DatabaseError::QueryFailed(Box::new(e)))?;

        let data =
            serde_json::to_value(&data).map_err(|e| DatabaseError::QueryFailed(Box::new(e)))?;

        Ok(QueryResponse {
            affected_rows: data.as_array().map(|a| a.len()).unwrap_or(0),
            data,
        })
    }

    async fn health_check(&self) -> Result<()> {
        self.execute("INFO FOR DB", vec![]).await.map(|_| ())
    }

    async fn schema_version(&self) -> Result<u32> {
        let response = self
            .execute("SELECT schema_version FROM system_metadata LIMIT 1", vec![])
            .await?;

        if let Some(version) = response
            .data
            .as_array()
            .and_then(|arr| arr.first())
            .and_then(|obj| obj.get("schema_version"))
            .and_then(|v| v.as_u64())
        {
            Ok(version as u32)
        } else {
            Ok(0) // No schema version found, assume 0
        }
    }
}

#[async_trait]
impl DatabaseOperations for EmbeddedDatabase {
    async fn transaction<F, R>(&self, f: F) -> Result<R>
    where
        F: FnOnce(Arc<dyn Transaction>) -> Result<R> + Send,
        R: Send,
    {
        // SurrealDB handles transactions differently
        // For now, we'll execute operations directly
        // TODO: Implement proper transaction support when available
        let tx = Arc::new(EmbeddedTransaction {
            client: self.client.clone(),
            operations: Arc::new(RwLock::new(Vec::new())),
        });

        f(tx)
    }
}

#[async_trait]
impl VectorStore for EmbeddedDatabase {
    async fn vector_search(
        &self,
        table: &str,
        embedding_field: &str,
        query_vector: &[f32],
        limit: usize,
        filter: Option<SearchFilter>,
    ) -> Result<Vec<VectorSearchResult>> {
        let vector_str = format!(
            "[{}]",
            query_vector
                .iter()
                .map(|f| f.to_string())
                .collect::<Vec<_>>()
                .join(",")
        );

        let mut query = format!(
            "SELECT *, vector::distance::knn() AS score FROM {} WHERE {} <|{}|> {}",
            table, embedding_field, limit, vector_str
        );

        let mut params = vec![];

        if let Some(filter) = filter {
            if let Some(where_clause) = filter.where_clause {
                query.push_str(&format!(" AND {}", where_clause));
                params = filter.params;
            }
        }

        query.push_str(" ORDER BY score");

        let response = self.execute(&query, params).await?;

        let results = response
            .data
            .as_array()
            .ok_or_else(|| DatabaseError::QueryFailed("Expected array response".into()))?
            .iter()
            .filter_map(|item| {
                let score = item.get("score")?.as_f64()? as f32;
                let id = item
                    .get("id")?
                    .as_object()?
                    .get("id")?
                    .as_str()?
                    .to_string();

                Some(VectorSearchResult {
                    id,
                    score,
                    data: item.clone(),
                })
            })
            .collect();

        Ok(results)
    }

    async fn create_vector_index(
        &self,
        table: &str,
        field: &str,
        dimensions: usize,
        distance_metric: DistanceMetric,
    ) -> Result<()> {
        let query = format!(
            "DEFINE INDEX {}_embedding_idx ON {} FIELDS {} HNSW DIMENSION {} DIST {}",
            table,
            table,
            field,
            dimensions,
            distance_metric.as_surreal_string()
        );

        self.execute(&query, vec![]).await?;
        Ok(())
    }

    async fn vector_index_exists(&self, table: &str, field: &str) -> Result<bool> {
        let query = format!("INFO FOR TABLE {}", table);
        let response = self.execute(&query, vec![]).await?;

        // Check if the response contains index information
        // This is a simplified check - you might need to parse the response more thoroughly
        let has_index = response
            .data
            .to_string()
            .contains(&format!("{}_embedding_idx", table));

        Ok(has_index)
    }
}

/// Transaction implementation for embedded database
struct EmbeddedTransaction {
    client: Surreal<Db>,
    operations: Arc<RwLock<Vec<(String, Vec<(String, serde_json::Value)>)>>>,
}

#[async_trait]
impl Transaction for EmbeddedTransaction {
    async fn execute(
        &self,
        query: &str,
        params: Vec<(String, serde_json::Value)>,
    ) -> Result<QueryResponse> {
        // Store the operation for later execution
        self.operations
            .write()
            .await
            .push((query.to_string(), params.clone()));

        // Execute immediately for now (until proper transaction support)
        let mut surrealdb_query = self.client.query(query);

        for (name, value) in params {
            surrealdb_query = surrealdb_query.bind((name, value));
        }

        let mut response = surrealdb_query
            .await
            .map_err(|e| DatabaseError::QueryFailed(Box::new(e)))?;

        let data: surrealdb::Value = response
            .take(0)
            .map_err(|e| DatabaseError::QueryFailed(Box::new(e)))?;

        let data =
            serde_json::to_value(&data).map_err(|e| DatabaseError::QueryFailed(Box::new(e)))?;

        Ok(QueryResponse {
            affected_rows: data.as_array().map(|a| a.len()).unwrap_or(0),
            data,
        })
    }

    async fn commit(self: Box<Self>) -> Result<()> {
        // Operations are already executed
        Ok(())
    }

    async fn rollback(self: Box<Self>) -> Result<()> {
        // Can't rollback already executed operations
        // This is a limitation of the current implementation
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn test_embedded_connection() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test.db").to_string_lossy().to_string();

        let config = DatabaseConfig::Embedded {
            path,
            strict_mode: false,
        };

        let db = EmbeddedDatabase::connect(config).await.unwrap();
        db.health_check().await.unwrap();
    }

    #[tokio::test]
    async fn test_execute_query() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test.db").to_string_lossy().to_string();

        let config = DatabaseConfig::Embedded {
            path,
            strict_mode: false,
        };

        let db = EmbeddedDatabase::connect(config).await.unwrap();

        // Create a table
        db.execute("DEFINE TABLE test SCHEMAFULL", vec![])
            .await
            .unwrap();

        // Insert data
        let result = db
            .execute(
                "CREATE test SET name = $name",
                vec![("name".to_string(), serde_json::json!("test_name"))],
            )
            .await
            .unwrap();

        assert_eq!(result.affected_rows, 1);
    }
}
