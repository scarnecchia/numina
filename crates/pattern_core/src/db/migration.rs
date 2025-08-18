//! Simplified database migration system for schema versioning

use super::{DatabaseError, Result};
use crate::db::schema::Schema;
use crate::id::{IdType, MemoryId, TaskId};
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
        tracing::info!(
            "MigrationRunner::run_with_options called with force_update={}",
            force_update
        );

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
            let actually_ran = Self::migrate_v2_message_batching(db, force_update).await?;
            // Only update version if we actually ran the migration
            if actually_ran {
                Self::update_schema_version(db, 2).await?;
                tracing::info!("Migration v2 completed in {:?}", migration_start.elapsed());
            }
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

        // let message_index = Schema::vector_index("msg", "embedding", dimensions);
        // db.query(&message_index)
        //     .await
        //     .map_err(|e| DatabaseError::QueryFailed(e))?;

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
    /// Returns true if migration actually ran, false if skipped
    async fn migrate_v2_message_batching<C: Connection>(
        db: &Surreal<C>,
        force: bool,
    ) -> Result<bool> {
        use crate::agent::AgentRecord;
        use crate::context::state::MessageHistory;
        use crate::db::entity::DbEntity;
        use crate::message::{BatchType, ChatRole, Message, MessageBatch};
        use tokio::time::{Duration, sleep};

        tracing::info!(
            "Starting per-agent message batch migration (force={})",
            force
        );

        // Only run this migration if forced - it's expensive and may not be needed
        if !force {
            tracing::info!("Skipping message batch migration (only runs with --force-migrate)");
            return Ok(false);
        }

        tracing::info!("Force flag set, proceeding with migration");

        // Drop search indexes before bulk updates to avoid corruption
        tracing::info!("Dropping search indexes before migration...");

        let drop_msg_index = "REMOVE INDEX IF EXISTS msg_content_search ON msg";
        db.query(drop_msg_index)
            .await
            .map_err(|e| DatabaseError::QueryFailed(e))?;

        let drop_conv_index = "REMOVE INDEX IF EXISTS idx_agent_conversation_search ON agent";
        db.query(drop_conv_index)
            .await
            .map_err(|e| DatabaseError::QueryFailed(e))?;

        tracing::info!("Search indexes dropped");

        // Query all agent records
        let query = "SELECT * FROM agent";
        let mut result = db
            .query(query)
            .await
            .map_err(|e| DatabaseError::QueryFailed(e))?;

        let agent_records: Vec<<AgentRecord as DbEntity>::DbModel> =
            result.take(0).unwrap_or_default();

        let agents: Vec<AgentRecord> = agent_records
            .into_iter()
            .map(|model| AgentRecord::from_db_model(model).expect("should convert"))
            .collect();

        tracing::info!("Found {} agents to migrate", agents.len());

        for agent in agents {
            tracing::info!("\n=== Processing agent: {} ({})", agent.name, agent.id);

            // Load all messages for this agent
            let messages_with_relations = agent.load_message_history(db, true).await?;

            if messages_with_relations.is_empty() {
                tracing::info!("  No messages for agent {}", agent.name);
                continue;
            }

            tracing::info!("  Found {} messages", messages_with_relations.len());

            // Extract just the messages, preserving order
            let mut messages: Vec<Message> = messages_with_relations
                .iter()
                .map(|(msg, _relation)| msg.clone())
                .collect();

            // Create MessageHistory to hold our batches
            let mut history =
                MessageHistory::new(crate::context::compression::CompressionStrategy::Truncate {
                    keep_recent: 100,
                });
            let mut accumulator: Vec<Message> = Vec::new();
            let mut current_batch_id: Option<crate::agent::SnowflakePosition> = None;
            let mut all_removed_ids: Vec<crate::id::MessageId> = Vec::new();

            let mut last_role: Option<ChatRole> = None;

            for (idx, message) in messages.iter_mut().enumerate() {
                let is_user_message = message.role == ChatRole::User;
                let is_system_message = message.role == ChatRole::System;
                let is_first_message = idx == 0;

                // Start new batch on:
                // - First message
                // - System message (always starts new batch)
                // - User message AFTER a non-user message (not consecutive users)
                let starts_new_batch = is_first_message
                    || is_system_message
                    || (is_user_message && last_role.as_ref() != Some(&ChatRole::User));

                if starts_new_batch {
                    // Create batch from accumulated messages if any
                    if !accumulator.is_empty() {
                        let batch_id = current_batch_id.expect("batch_id should be set");
                        let batch_type =
                            accumulator[0].batch_type.unwrap_or(BatchType::UserRequest);

                        tracing::info!(
                            "  Creating batch {} with {} messages",
                            batch_id,
                            accumulator.len()
                        );
                        for (seq, msg) in accumulator.iter().enumerate() {
                            let content = msg.display_content();
                            let preview = if content.len() > 200 {
                                let start: String = content.chars().take(100).collect();
                                let end: String = content
                                    .chars()
                                    .rev()
                                    .take(100)
                                    .collect::<String>()
                                    .chars()
                                    .rev()
                                    .collect();
                                format!("{}...{}", start, end)
                            } else {
                                content.clone()
                            };
                            tracing::info!("    [{:02}] {} - {}", seq, msg.role, preview);
                        }

                        let mut batch =
                            MessageBatch::from_messages(batch_id, batch_type, accumulator.clone());
                        let removed_ids = batch.finalize(); // Clean up any unpaired tool calls
                        if !removed_ids.is_empty() {
                            tracing::warn!(
                                "  Removing {} unpaired tool call messages from batch",
                                removed_ids.len()
                            );
                            all_removed_ids.extend(removed_ids);
                        }
                        history.add_batch(batch);
                        accumulator.clear();
                    }

                    // Generate new snowflake for this batch
                    let snowflake = crate::agent::get_next_message_position_sync();

                    // Small delay to ensure snowflake uniqueness
                    sleep(Duration::from_millis(10)).await;

                    // Set both position and batch to same snowflake
                    message.position = Some(snowflake);
                    message.batch = Some(snowflake);
                    current_batch_id = Some(snowflake);

                    // Determine batch type
                    message.batch_type = Some(if message.role == ChatRole::System {
                        BatchType::SystemTrigger
                    } else {
                        BatchType::UserRequest
                    });

                    tracing::info!(
                        "  Starting new batch {} at message {} ({})",
                        snowflake,
                        idx,
                        message.role
                    );
                } else {
                    // Continue current batch
                    let snowflake = crate::agent::get_next_message_position_sync();

                    // Small delay for uniqueness
                    sleep(Duration::from_millis(5)).await;

                    message.position = Some(snowflake);
                    message.batch = current_batch_id;
                    message.batch_type = Some(BatchType::UserRequest);
                }

                accumulator.push(message.clone());
                last_role = Some(message.role.clone());
            }

            // Create final batch from remaining messages
            if !accumulator.is_empty() {
                let batch_id = current_batch_id.expect("batch_id should be set");
                let batch_type = accumulator[0].batch_type.unwrap_or(BatchType::UserRequest);

                tracing::info!(
                    "  Creating final batch {} with {} messages",
                    batch_id,
                    accumulator.len()
                );

                let mut batch = MessageBatch::from_messages(batch_id, batch_type, accumulator);
                let removed_ids = batch.finalize();
                if !removed_ids.is_empty() {
                    tracing::warn!(
                        "  Removing {} unpaired tool call messages from final batch",
                        removed_ids.len()
                    );
                    all_removed_ids.extend(removed_ids);
                }
                history.add_batch(batch);
            }

            // Now extract processed batches and update database
            tracing::info!("  Created {} batches total", history.batches.len());

            for batch in &history.batches {
                let status = if batch.is_complete {
                    "✓ complete".to_string()
                } else {
                    let pending = batch.get_pending_tool_calls();
                    let last_role = batch.messages.last().map(|m| &m.role);
                    if !pending.is_empty() {
                        format!("⚠️ INCOMPLETE - {} pending tool calls", pending.len())
                    } else if last_role != Some(&ChatRole::Assistant) {
                        format!("⚠️ INCOMPLETE - ends with {:?}", last_role)
                    } else {
                        "⚠️ INCOMPLETE - unknown reason".to_string()
                    }
                };

                tracing::info!(
                    "  Batch {}: {} messages, {}",
                    batch.id,
                    batch.messages.len(),
                    status
                );

                // For incomplete or unusually long batches, show details
                if !batch.is_complete || batch.messages.len() > 20 {
                    tracing::warn!("    Detailed view of batch {}:", batch.id);

                    // For incomplete batches, show ALL messages to debug tool pairing
                    if !batch.is_complete {
                        for (i, msg) in batch.messages.iter().enumerate() {
                            let content = msg.display_content();
                            let preview: String = content.chars().take(100).collect();

                            // Extract tool call/response IDs if present, also check Blocks
                            let tool_info = match &msg.content {
                                crate::message::MessageContent::ToolCalls(calls) => {
                                    let ids: Vec<String> =
                                        calls.iter().map(|c| c.call_id.clone()).collect();
                                    format!(" [calls: {}]", ids.join(", "))
                                }
                                crate::message::MessageContent::ToolResponses(responses) => {
                                    let ids: Vec<String> =
                                        responses.iter().map(|r| r.call_id.clone()).collect();
                                    format!(" [responses: {}]", ids.join(", "))
                                }
                                crate::message::MessageContent::Blocks(blocks) => {
                                    let mut call_ids = Vec::new();
                                    let mut response_ids = Vec::new();
                                    for block in blocks {
                                        match block {
                                            crate::message::ContentBlock::ToolUse {
                                                id, ..
                                            } => {
                                                call_ids.push(id.clone());
                                            }
                                            crate::message::ContentBlock::ToolResult {
                                                tool_use_id,
                                                ..
                                            } => {
                                                response_ids.push(tool_use_id.clone());
                                            }
                                            _ => {}
                                        }
                                    }
                                    if !call_ids.is_empty() {
                                        format!(" [block calls: {}]", call_ids.join(", "))
                                    } else if !response_ids.is_empty() {
                                        format!(" [block responses: {}]", response_ids.join(", "))
                                    } else {
                                        String::new()
                                    }
                                }
                                _ => String::new(),
                            };

                            tracing::warn!(
                                "      [{:02}] {} - {}{}",
                                i,
                                msg.role,
                                preview,
                                tool_info
                            );
                        }
                    } else {
                        // For long but complete batches, show abbreviated view
                        for (i, msg) in batch.messages.iter().take(3).enumerate() {
                            let content = msg.display_content();
                            let preview: String = content.chars().take(80).collect();
                            tracing::warn!("      [{:02}] {} - {}", i, msg.role, preview);
                        }

                        if batch.messages.len() > 6 {
                            tracing::warn!(
                                "      ... {} messages omitted ...",
                                batch.messages.len() - 6
                            );
                        }

                        let start_idx = batch.messages.len().saturating_sub(3);
                        for (i, msg) in batch.messages.iter().skip(start_idx).enumerate() {
                            let content = msg.display_content();
                            let preview: String = content.chars().take(80).collect();
                            tracing::warn!(
                                "      [{:02}] {} - {}",
                                start_idx + i,
                                msg.role,
                                preview
                            );
                        }
                    }

                    // Show details about why batch is incomplete
                    if !batch.is_complete {
                        let pending = batch.get_pending_tool_calls();
                        if !pending.is_empty() {
                            tracing::warn!("    ⚠️ Pending tool calls: {:?}", pending);
                        } else {
                            let last_role = batch.messages.last().map(|m| &m.role);
                            tracing::warn!(
                                "    ⚠️ Batch ends with {:?} (not Assistant)",
                                last_role
                            );
                        }
                    }
                }

                // Update each message in the database
                for message in &batch.messages {
                    // Update the message itself
                    let update_query = r#"
                        UPDATE $msg_id SET
                            position = $position,
                            batch = $batch,
                            sequence_num = $seq_num,
                            batch_type = $batch_type
                    "#;

                    db.query(update_query)
                        .bind(("msg_id", surrealdb::RecordId::from(&message.id)))
                        .bind(("position", message.position.as_ref().map(|p| p.to_string())))
                        .bind(("batch", message.batch.as_ref().map(|b| b.to_string())))
                        .bind(("seq_num", message.sequence_num))
                        .bind((
                            "batch_type",
                            message.batch_type.as_ref().map(|bt| match bt {
                                BatchType::UserRequest => "user_request",
                                BatchType::AgentToAgent => "agent_to_agent",
                                BatchType::SystemTrigger => "system_trigger",
                                BatchType::Continuation => "continuation",
                            }),
                        ))
                        .await
                        .map_err(|e| DatabaseError::QueryFailed(e))?;

                    // Update the agent_messages relation that points to this message
                    let sync_relation_query = r#"
                        UPDATE agent_messages SET
                            position = out.position,
                            batch = out.batch,
                            sequence_num = out.sequence_num,
                            batch_type = out.batch_type
                        WHERE out = $msg_id
                    "#;

                    db.query(sync_relation_query)
                        .bind(("msg_id", surrealdb::RecordId::from(&message.id)))
                        .await
                        .map_err(|e| DatabaseError::QueryFailed(e))?;
                }
            }

            // Delete any messages that were removed due to unpaired tool calls
            if !all_removed_ids.is_empty() {
                tracing::warn!(
                    "  Found {} unpaired tool call messages that should be deleted",
                    all_removed_ids.len()
                );
                tracing::warn!("  Message IDs to delete manually:");
                for msg_id in &all_removed_ids {
                    tracing::warn!("    DELETE msg:{};", msg_id);
                }
                tracing::warn!("  Skipping deletion to avoid database corruption");
                // TODO: Fix deletion during migration - currently causes corruption
                // for msg_id in all_removed_ids {
                //     let _: Option<<Message as DbEntity>::DbModel> = db
                //         .delete(surrealdb::RecordId::from(msg_id.clone()))
                //         .await
                //         .map_err(|e| DatabaseError::QueryFailed(e))?;
                // }
            }

            tracing::info!("  ✓ Agent {} migration complete", agent.name);
        }

        tracing::info!("\nMessage batch migration completed for all agents");

        // Recreate message-related indexes after all the updates
        tracing::info!("Recreating search indexes after migration...");

        // Recreate the message content search index
        let recreate_msg_index = "DEFINE INDEX IF NOT EXISTS msg_content_search ON msg FIELDS content SEARCH ANALYZER msg_content_analyzer BM25";
        db.query(recreate_msg_index)
            .await
            .map_err(|e| DatabaseError::QueryFailed(e))?;

        // Recreate the agent conversation search index
        let recreate_conv_index = "DEFINE INDEX IF NOT EXISTS idx_agent_conversation_search
              ON TABLE agent
              COLUMNS conversation_history.*.content
              SEARCH ANALYZER msg_content_analyzer
              BM25";
        db.query(recreate_conv_index)
            .await
            .map_err(|e| DatabaseError::QueryFailed(e))?;

        tracing::info!("Search indexes recreated successfully");

        Ok(true)
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
