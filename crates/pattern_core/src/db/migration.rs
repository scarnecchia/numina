//! Simplified database migration system for schema versioning

use super::{DatabaseError, Result};
use crate::db::schema::Schema;
use crate::id::{IdType, MemoryId, TaskId};
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

        // Create entity tables using their schema definitions
        use crate::MemoryBlock;
        use crate::agent::AgentRecord;
        use crate::db::entity::{BaseEvent, BaseTask, DbEntity};
        use crate::db::schema::ToolCall;
        use crate::message::Message;
        use crate::users::User;

        // Create all entity tables
        for table_def in [
            User::schema(),
            AgentRecord::schema(),
            BaseTask::schema(),
            MemoryBlock::schema(),
            BaseEvent::schema(),
            Message::schema(),
            ToolCall::schema(),
        ] {
            // Execute table schema
            db.query(&table_def.schema)
                .await
                .map_err(|e| DatabaseError::QueryFailed(e))?;

            // Create indexes
            for index in &table_def.indexes {
                db.query(index)
                    .await
                    .map_err(|e| DatabaseError::QueryFailed(e))?;
            }
        }

        // Create auxiliary tables (system_metadata, etc.)
        for table_def in Schema::tables() {
            // Execute table schema
            db.query(&table_def.schema)
                .await
                .map_err(|e| DatabaseError::QueryFailed(e))?;

            // Create indexes
            for index in &table_def.indexes {
                db.query(index)
                    .await
                    .map_err(|e| DatabaseError::QueryFailed(e))?;
            }
        }

        // Create specialized indices (full-text search, vector indices)
        Self::create_specialized_indices(db).await?;

        // Add more migrations here as needed
        // if current_version < 2 {
        //     tracing::info!("Running migration v2: ...");
        //     Self::migrate_v2(db).await?;
        //     Self::update_schema_version(db, 2).await?;
        // }

        Ok(())
    }

    /// Migration v1: Initial schema - only creates system metadata
    async fn migrate_v1<C: Connection>(db: &Surreal<C>) -> Result<()> {
        // Only create the system metadata table
        let metadata_table = Schema::system_metadata();

        // Execute table schema
        db.query(&metadata_table.schema)
            .await
            .map_err(|e| DatabaseError::QueryFailed(e))?;

        // Create indexes
        for index in &metadata_table.indexes {
            db.query(index)
                .await
                .map_err(|e| DatabaseError::QueryFailed(e))?;
        }

        // Create vector indexes with default dimensions (384 for MiniLM)
        let dimensions = 384;

        // Create vector indexes for tables with embeddings
        let memory_index = Schema::vector_index(MemoryId::PREFIX, "embedding", dimensions);
        db.query(&memory_index)
            .await
            .map_err(|e| DatabaseError::QueryFailed(e))?;

        let message_index = Schema::vector_index("message", "embedding", dimensions);
        db.query(&message_index)
            .await
            .map_err(|e| DatabaseError::QueryFailed(e))?;

        let task_index = Schema::vector_index(TaskId::PREFIX, "embedding", dimensions);
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

    /// Create specialized indices (full-text search, vector indices)
    async fn create_specialized_indices<C: Connection>(db: &Surreal<C>) -> Result<()> {
        use crate::id::{MemoryId, MessageId, TaskId};

        // Create full-text search analyzer and index for messages
        let message_analyzer = format!(
            "DEFINE ANALYZER {}_content_analyzer TOKENIZERS class FILTERS lowercase, snowball(english)",
            MessageId::PREFIX
        );
        db.query(&message_analyzer)
            .await
            .map_err(|e| DatabaseError::QueryFailed(e))?;

        let message_search_index =
            "DEFINE FIELD OVERWRITE conversation_history
                  ON TABLE agent
                  VALUE <future> {
                      (SELECT VALUE ->agent_messages->msg.*
                       FROM ONLY $this)
                  };
            DEFINE INDEX OVERWRITE msg_content_search ON msg FIELDS content SEARCH ANALYZER msg_content_analyzer BM25;
            DEFINE INDEX OVERWRITE idx_agent_conversation_search
              ON TABLE agent
              COLUMNS conversation_history.*.content
              SEARCH ANALYZER msg_content_analyzer
              BM25;
            ".to_string();
        db.query(&message_search_index)
            .await
            .map_err(|e| DatabaseError::QueryFailed(e))?;

        // Create full-text search analyzer and index for memory blocks
        let memory_analyzer = format!(
            "DEFINE ANALYZER {}_value_analyzer TOKENIZERS class FILTERS lowercase, snowball(english)",
            MemoryId::PREFIX
        );
        db.query(&memory_analyzer)
            .await
            .map_err(|e| DatabaseError::QueryFailed(e))?;

        let memory_search_index =
            "DEFINE FIELD OVERWRITE archival_memories
                  ON TABLE agent
                  VALUE <future> {
                      (SELECT VALUE ->agent_memories->(mem WHERE memory_type = 'archival')
                       FROM ONLY $this FETCH mem)
                  };
            DEFINE INDEX OVERWRITE mem_value_search ON mem FIELDS value SEARCH ANALYZER mem_value_analyzer BM25;
            DEFINE INDEX OVERWRITE idx_agent_archival_search
              ON TABLE agent
              FIELDS archival_memories.*.value
              SEARCH ANALYZER mem_value_analyzer
              BM25;".to_string();
        db.query(&memory_search_index)
            .await
            .map_err(|e| DatabaseError::QueryFailed(e))?;

        // Create vector indexes with default dimensions (384 for MiniLM)
        let dimensions = 384;

        let memory_index = Schema::vector_index(MemoryId::PREFIX, "embedding", dimensions);
        db.query(&memory_index)
            .await
            .map_err(|e| DatabaseError::QueryFailed(e))?;

        let message_index = Schema::vector_index(MessageId::PREFIX, "embedding", dimensions);
        db.query(&message_index)
            .await
            .map_err(|e| DatabaseError::QueryFailed(e))?;

        let task_index = Schema::vector_index(TaskId::PREFIX, "embedding", dimensions);
        db.query(&task_index)
            .await
            .map_err(|e| DatabaseError::QueryFailed(e))?;

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
