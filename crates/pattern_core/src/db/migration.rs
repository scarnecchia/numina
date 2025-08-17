//! Simplified database migration system for schema versioning

use super::{DatabaseError, Result};
use crate::agent::get_next_message_position_string;
use crate::db::schema::Schema;
use crate::id::{IdType, MemoryId, TaskId};
use crate::message::BatchType;
use surrealdb::{Connection, Surreal};

/// Database migration runner
pub struct MigrationRunner;

impl MigrationRunner {
    /// Get the compiled schema hash
    fn get_compiled_schema_hash() -> u64 {
        // This is set by build.rs at compile time
        option_env!("PATTERN_SCHEMA_HASH")
            .and_then(|s| s.parse().ok())
            .unwrap_or(0)
    }

    /// Run all migrations
    pub async fn run<C: Connection>(db: &Surreal<C>) -> Result<()> {
        Self::run_with_options(db, false).await
    }

    /// Run migrations with options
    pub async fn run_with_options<C: Connection>(
        db: &Surreal<C>,
        force_update: bool,
    ) -> Result<()> {
        let start = std::time::Instant::now();

        // Check if we can skip schema updates
        if !force_update {
            let compiled_hash = Self::get_compiled_schema_hash();
            if compiled_hash != 0 {
                // Try to get stored schema hash
                if let Ok(Some(stored_hash)) = Self::get_schema_hash(db).await {
                    if stored_hash == compiled_hash {
                        tracing::info!(
                            "Schema unchanged (hash: {}), skipping migrations",
                            compiled_hash
                        );
                        return Ok(());
                    }
                }
            }
        }

        tracing::info!("Starting database migrations...");

        let current_version = Self::get_schema_version(db).await?;
        tracing::info!("Current schema version: {}", current_version);

        if current_version < 1 {
            tracing::info!("Running migration v1: Initial schema");
            let migration_start = std::time::Instant::now();
            Self::migrate_v1(db).await?;

            // Create entity tables using their schema definitions
            use crate::MemoryBlock;
            use crate::agent::AgentRecord;
            use crate::db::entity::{BaseEvent, BaseTask, DbEntity};
            use crate::db::schema::ToolCall;
            use crate::message::Message;
            use crate::users::User;

            // Create all entity tables
            let entity_start = std::time::Instant::now();
            tracing::info!("Creating entity tables...");
            for table_def in [
                User::schema(),
                AgentRecord::schema(),
                BaseTask::schema(),
                MemoryBlock::schema(),
                BaseEvent::schema(),
                Message::schema(),
                ToolCall::schema(),
            ] {
                let table_start = std::time::Instant::now();
                let table_name = table_def
                    .schema
                    .split_whitespace()
                    .nth(2)
                    .unwrap_or("unknown");

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

                tracing::debug!(
                    "Created table {} in {:?}",
                    table_name,
                    table_start.elapsed()
                );
            }
            tracing::info!("Entity tables created in {:?}", entity_start.elapsed());

            // Create auxiliary tables (system_metadata, etc.)
            let aux_start = std::time::Instant::now();
            tracing::info!("Creating auxiliary tables...");
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
            tracing::info!("Auxiliary tables created in {:?}", aux_start.elapsed());

            // Create specialized indices (full-text search, vector indices)
            let indices_start = std::time::Instant::now();
            tracing::info!("Creating specialized indices...");
            Self::create_specialized_indices(db).await?;
            tracing::info!(
                "Specialized indices created in {:?}",
                indices_start.elapsed()
            );

            Self::update_schema_version(db, 1).await?;
            tracing::info!("Migration v1 completed in {:?}", migration_start.elapsed());
        }

        // Add more migrations here as needed
        if current_version < 2 {
            tracing::info!("Running migration v2: Message batching (snowflake IDs)");
            let migration_start = std::time::Instant::now();
            Self::migrate_v2_message_batching(db).await?;
            Self::update_schema_version(db, 2).await?;
            tracing::info!("Migration v2 completed in {:?}", migration_start.elapsed());
        }

        // Store the new schema hash
        let compiled_hash = Self::get_compiled_schema_hash();
        if compiled_hash != 0 {
            Self::update_schema_hash(db, compiled_hash).await?;
        }

        tracing::info!("All database migrations completed in {:?}", start.elapsed());
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

    /// Get schema hash
    async fn get_schema_hash<C: Connection>(db: &Surreal<C>) -> Result<Option<u64>> {
        let mut result = db
            .query("SELECT schema_hash FROM system_metadata LIMIT 1")
            .await
            .map_err(|e| DatabaseError::QueryFailed(e))?;

        #[derive(serde::Deserialize)]
        struct SchemaHash {
            schema_hash: Option<u64>,
        }

        let hashes: Vec<SchemaHash> = result.take(0).unwrap_or_default();

        Ok(hashes.first().and_then(|h| h.schema_hash))
    }

    /// Update schema hash
    async fn update_schema_hash<C: Connection>(db: &Surreal<C>, hash: u64) -> Result<()> {
        db.query("UPDATE system_metadata SET schema_hash = $hash, updated_at = time::now()")
            .bind(("hash", hash))
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

    /// Migration v2: Add snowflake IDs and batch tracking to messages
    async fn migrate_v2_message_batching<C: Connection>(db: &Surreal<C>) -> Result<()> {
        use chrono::Duration;

        tracing::info!("Starting message batch migration...");

        // Query all messages ordered by created_at
        let query = r#"
            SELECT * FROM msg
            ORDER BY created_at ASC
        "#;

        let mut result = db
            .query(query)
            .await
            .map_err(|e| DatabaseError::QueryFailed(e))?;

        let messages: Vec<serde_json::Value> = result.take(0).unwrap_or_default();

        tracing::info!("Found {} messages to migrate", messages.len());

        if messages.is_empty() {
            return Ok(());
        }

        // Process messages to detect batches
        let mut batch_updates = Vec::new();
        let mut current_batch_id: Option<String> = None;
        let mut sequence_num = 0u32;
        let mut last_timestamp = None;
        let mut last_role = None;

        for (idx, msg_value) in messages.iter().enumerate() {
            // Extract fields we need
            let msg_id = msg_value.get("id").and_then(|v| v.as_str()).unwrap_or("");

            let role = msg_value
                .get("role")
                .and_then(|v| v.as_str())
                .unwrap_or("user");

            let created_at = msg_value
                .get("created_at")
                .and_then(|v| v.as_str())
                .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok());

            let has_tool_calls = msg_value
                .get("has_tool_calls")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);

            // Check if this starts a new batch
            let starts_new_batch = if let Some(last_ts) = last_timestamp {
                if let Some(current_ts) = created_at {
                    // New batch if:
                    // 1. User message after non-user (new conversation turn)
                    // 2. Time gap > 30 minutes (new conversation)
                    // 3. First message
                    let time_gap = current_ts.signed_duration_since(last_ts);
                    let is_new_turn = role == "user" && last_role != Some("user");
                    let is_time_gap = time_gap > Duration::minutes(30);

                    is_new_turn || is_time_gap || idx == 0
                } else {
                    false
                }
            } else {
                true // First message always starts a batch
            };

            // Generate snowflake ID for this message
            let snowflake_id = get_next_message_position_string().await;

            // Handle batch assignment
            if starts_new_batch {
                current_batch_id = Some(snowflake_id.clone());
                sequence_num = 0;
            } else {
                sequence_num += 1;
            }

            // Determine batch type
            let batch_type = if role == "user" {
                BatchType::UserRequest
            } else if role == "system" {
                BatchType::SystemTrigger
            } else {
                // Could be more sophisticated here
                BatchType::UserRequest
            };

            // Store update for this message
            batch_updates.push((
                msg_id.to_string(),
                snowflake_id,
                current_batch_id.clone().unwrap_or_default(),
                sequence_num,
                batch_type,
            ));

            // Update tracking variables
            last_timestamp = created_at;
            last_role = Some(role);
        }

        // Apply updates to all messages
        tracing::info!("Applying batch updates to {} messages", batch_updates.len());

        for (msg_id, snowflake_id, batch_id, seq_num, batch_type) in batch_updates {
            let update_query = r#"
                UPDATE $msg_id SET
                    position = $snowflake_id,
                    batch = $batch_id,
                    sequence_num = $seq_num,
                    batch_type = $batch_type
            "#;

            db.query(update_query)
                .bind(("msg_id", msg_id))
                .bind(("snowflake_id", snowflake_id))
                .bind(("batch_id", batch_id))
                .bind(("seq_num", seq_num))
                .bind(("batch_type", format!("{:?}", batch_type).to_lowercase()))
                .await
                .map_err(|e| DatabaseError::QueryFailed(e))?;
        }

        // Update agent_messages relations to sync position field
        tracing::info!("Syncing agent_messages relations...");

        let sync_query = r#"
            UPDATE agent_messages SET
                position = out.position,
                batch = out.batch,
                sequence_num = out.sequence_num,
                batch_type = out.batch_type
            WHERE out.position IS NOT NULL
        "#;

        db.query(sync_query)
            .await
            .map_err(|e| DatabaseError::QueryFailed(e))?;

        tracing::info!("Message batch migration completed");
        Ok(())
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
            "DEFINE FIELD IF NOT EXISTS conversation_history
                  ON TABLE agent
                  VALUE <future> {
                      (SELECT VALUE ->agent_messages->msg.*
                       FROM ONLY $this)
                  };
            DEFINE INDEX IF NOT EXISTS msg_content_search ON msg FIELDS content SEARCH ANALYZER msg_content_analyzer BM25;
            DEFINE INDEX IF NOT EXISTS idx_agent_conversation_search
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
            "DEFINE FIELD IF NOT EXISTS archival_memories
                  ON TABLE agent
                  VALUE <future> {
                      (SELECT VALUE ->agent_memories->(mem WHERE memory_type = 'archival')
                       FROM ONLY $this FETCH mem)
                  };
            DEFINE INDEX IF NOT EXISTS mem_value_search ON mem FIELDS value SEARCH ANALYZER mem_value_analyzer BM25;
            DEFINE INDEX IF NOT EXISTS idx_agent_archival_search
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
