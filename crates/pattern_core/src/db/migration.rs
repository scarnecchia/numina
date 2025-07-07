//! Simplified database migration system for schema versioning

use super::{DatabaseBackend, Result, VectorStore};
use std::sync::Arc;

/// Database migration runner
pub struct MigrationRunner;

impl MigrationRunner {
    /// Run all migrations
    pub async fn run_migrations<DB: VectorStore>(db: &Arc<DB>) -> Result<()> {
        let current_version = db.schema_version().await?;

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
    async fn migrate_v1<DB: VectorStore>(db: &Arc<DB>) -> Result<()> {
        use crate::db::schema::Schema;

        // Create all tables
        for table in Schema::tables() {
            // Execute table schema
            db.execute(&table.schema, vec![]).await?;

            // Create indexes
            for index in &table.indexes {
                db.execute(index, vec![]).await?;
            }
        }

        // Create vector indexes with default dimensions (384 for MiniLM)
        db.create_vector_index(
            "memory_blocks",
            "embedding",
            384,
            crate::db::DistanceMetric::Cosine,
        )
        .await?;

        db.create_vector_index(
            "messages",
            "embedding",
            384,
            crate::db::DistanceMetric::Cosine,
        )
        .await?;

        db.create_vector_index("tasks", "embedding", 384, crate::db::DistanceMetric::Cosine)
            .await?;

        Ok(())
    }

    /// Update schema version
    async fn update_schema_version<DB: DatabaseBackend>(db: &Arc<DB>, version: u32) -> Result<()> {
        db.execute(
            "UPDATE system_metadata SET schema_version = $version, updated_at = time::now()",
            vec![("version".to_string(), serde_json::json!(version))],
        )
        .await?;
        Ok(())
    }

    /// Rollback to a specific version
    pub async fn rollback_to<DB: VectorStore>(db: &Arc<DB>, target_version: u32) -> Result<()> {
        let current_version = db.schema_version().await?;

        if current_version > target_version {
            // Rollback migrations in reverse order
            if current_version >= 1 && target_version < 1 {
                tracing::info!("Rolling back migration v1");
                Self::rollback_v1(db).await?;
                Self::update_schema_version(db, 0).await?;
            }

            // Add more rollbacks as needed
        }

        Ok(())
    }

    /// Rollback v1: Drop all tables
    async fn rollback_v1<DB: DatabaseBackend>(db: &Arc<DB>) -> Result<()> {
        let tables = vec![
            "tasks",
            "tool_calls",
            "messages",
            "conversations",
            "memory_blocks",
            "agents",
            "users",
            "system_metadata",
        ];

        for table in tables {
            db.execute(&format!("REMOVE TABLE IF EXISTS {}", table), vec![])
                .await?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{DatabaseConfig, embedded::EmbeddedDatabase};
    use tempfile::tempdir;

    #[tokio::test]
    async fn test_migration_runner() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test.db").to_string_lossy().to_string();

        let config = DatabaseConfig::Embedded {
            path,
            strict_mode: false,
        };

        let db = EmbeddedDatabase::connect(config).await.unwrap();

        // Should run without error
        MigrationRunner::run_migrations(&db).await.unwrap();

        // Check schema version
        let version = db.schema_version().await.unwrap();
        assert_eq!(version, 1);

        // Running again should be idempotent
        MigrationRunner::run_migrations(&db).await.unwrap();
    }
}
