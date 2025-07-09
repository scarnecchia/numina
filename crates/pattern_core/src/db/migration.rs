//! Simplified database migration system for schema versioning

use super::{DatabaseError, Result};
use crate::db::schema::Schema;
use crate::id::{IdType, MemoryIdType, MessageId as MessageIdType, TaskIdType};
use surrealdb::{Connection, Surreal};

/// Database migration runner
pub struct MigrationRunner;

impl MigrationRunner {
    /// Run all migrations
    pub async fn run<C: Connection>(db: &Surreal<C>) -> Result<()> {
        let current_version = Self::get_schema_version(db).await?;

        if current_version < 1 {
            tracing::info!("Running migration v1: Initial schema");
            Self::migrate_v1(db).await?;
            Self::update_schema_version(db, 1).await?;
        }

        // Add more migrations here as needed
        // if current_version < 2 {
        //     tracing::info!("Running migration v2: ...");
        //     Self::migrate_v2(db).await?;
        //     Self::update_schema_version(db, 2).await?;
        // }

        Ok(())
    }

    /// Migration v1: Initial schema
    async fn migrate_v1<C: Connection>(db: &Surreal<C>) -> Result<()> {
        // Create all tables
        for table in Schema::tables() {
            // Execute table schema
            db.query(&table.schema)
                .await
                .map_err(|e| DatabaseError::QueryFailed(e))?;

            // Create indexes
            // Execute indexes
            for index in &table.indexes {
                db.query(index)
                    .await
                    .map_err(|e| DatabaseError::QueryFailed(e))?;
            }
        }

        // Create vector indexes with default dimensions (384 for MiniLM)
        let dimensions = 384;

        // Create vector indexes for tables with embeddings
        let memory_index = Schema::vector_index(MemoryIdType::PREFIX, "embedding", dimensions);
        db.query(&memory_index)
            .await
            .map_err(|e| DatabaseError::QueryFailed(e))?;

        let message_index = Schema::vector_index(MessageIdType::PREFIX, "embedding", dimensions);
        db.query(&message_index)
            .await
            .map_err(|e| DatabaseError::QueryFailed(e))?;

        let task_index = Schema::vector_index(TaskIdType::PREFIX, "embedding", dimensions);
        db.query(&task_index)
            .await
            .map_err(|e| DatabaseError::QueryFailed(e))?;

        Ok(())
    }

    /// Get schema version
    async fn get_schema_version<C: Connection>(db: &Surreal<C>) -> Result<u32> {
        let mut result = db
            .query("SELECT schema_version FROM system_metadata LIMIT 1")
            .await
            .map_err(|e| DatabaseError::QueryFailed(e))?;

        #[derive(serde::Deserialize)]
        struct SchemaVersion {
            schema_version: u32,
        }

        let versions: Vec<SchemaVersion> = result.take(0).unwrap_or_default();

        Ok(versions.first().map(|v| v.schema_version).unwrap_or(0))
    }

    /// Update schema version
    async fn update_schema_version<C: Connection>(db: &Surreal<C>, version: u32) -> Result<()> {
        // Try to update existing record first
        let updated: Vec<serde_json::Value> = db
            .query("UPDATE system_metadata SET schema_version = $version, updated_at = time::now()")
            .bind(("version", version))
            .await
            .map_err(|e| DatabaseError::QueryFailed(e))?
            .take(0)
            .unwrap_or_default();

        // If no record was updated, create a new one
        if updated.is_empty() {
            db.query("CREATE system_metadata SET embedding_model = $embedding_model, embedding_dimensions = $embedding_dimensions, schema_version = $schema_version, created_at = time::now(), updated_at = time::now()")
                .bind(("embedding_model", "none"))
                .bind(("embedding_dimensions", 0))
                .bind(("schema_version", version))
                .await
                .map_err(|e| DatabaseError::QueryFailed(e))?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {

    use super::*;
    use crate::db::client;

    #[tokio::test]

    async fn test_migration_runner() {
        // Initialize the database (which runs migrations)
        let db = client::create_test_db().await.unwrap();

        // Check schema version
        let version = MigrationRunner::get_schema_version(&db).await.unwrap();
        assert_eq!(version, 1);

        // Running migrations again should be idempotent
        MigrationRunner::run(&db).await.unwrap();
    }
}
