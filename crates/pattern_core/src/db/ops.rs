//! Database operations - direct, simple, no unnecessary abstractions

use super::{DatabaseError, Result, entity::DbEntity};
use serde_json::json;
use surrealdb::{Connection, Surreal};

use crate::MessageId;
use crate::embeddings::EmbeddingProvider;
use crate::id::{AgentId, AgentIdType, IdType, MemoryId, MemoryIdType};
use crate::memory::MemoryBlock;
use crate::message::Message;

use crate::Id;
use crate::utils::debug::ResponseExt;
use chrono::Utc;

use futures::{Stream, StreamExt};
use surrealdb::opt::PatchOp;

use surrealdb::{Action, Notification, RecordId};

// ============================================================================
// Generic Entity Operations
// ============================================================================

/// Create a new entity in the database
pub async fn create_entity<E: DbEntity, C: Connection>(
    conn: &Surreal<C>,
    entity: &E,
) -> Result<E::Domain> {
    let db_model = E::to_db_model(entity);

    // Debug output to see what we're trying to create
    tracing::debug!(
        "Creating entity in table '{}' with model: {:?}",
        E::table_name(),
        serde_json::to_string_pretty(&db_model)
            .unwrap_or_else(|_| "failed to serialize".to_string())
    );

    let created: E::DbModel = conn
        .create((E::table_name(), entity.record_key()))
        .content(db_model)
        .await
        .map_err(|e| DatabaseError::QueryFailed(e))?
        .expect("SurrealDB should return created entity");

    E::from_db_model(created).map_err(DatabaseError::from)
}

/// Get an entity by ID
pub async fn get_entity<E: DbEntity, C: Connection>(
    conn: &Surreal<C>,
    id: &Id<E::Id>,
) -> Result<Option<E::Domain>> {
    let result: Option<E::DbModel> = conn
        .select((E::table_name(), id.uuid().to_string()))
        .await
        .map_err(|e| DatabaseError::QueryFailed(e))?;

    match result {
        Some(db_model) => Ok(Some(E::from_db_model(db_model)?)),
        None => Ok(None),
    }
}

/// Update an entity in the database
pub async fn update_entity<E: DbEntity, C: Connection>(
    conn: &Surreal<C>,
    entity: &E,
) -> Result<E::Domain> {
    let db_model = E::to_db_model(entity);
    let updated: Option<E::DbModel> = conn
        .update((E::table_name(), entity.id().uuid().to_string()))
        .merge(db_model)
        .await
        .map_err(|e| DatabaseError::QueryFailed(e))?;

    match updated {
        Some(db_model) => E::from_db_model(db_model).map_err(DatabaseError::from),
        None => Err(DatabaseError::NotFound {
            entity_type: E::table_name().to_string(),
            id: entity.id().to_string(),
        }),
    }
}

/// Delete an entity from the database
pub async fn delete_entity<E: DbEntity, C: Connection, I>(
    conn: &Surreal<C>,
    id: &Id<E::Id>,
) -> Result<()> {
    let _deleted: Option<E::DbModel> = conn
        .delete(RecordId::from(id))
        .await
        .map_err(|e| DatabaseError::QueryFailed(e))?;

    Ok(())
}

/// List all entities of a given type
pub async fn list_entities<E: DbEntity, C: Connection>(
    conn: &Surreal<C>,
) -> Result<Vec<E::Domain>> {
    let results: Vec<E::DbModel> = conn
        .select(E::table_name())
        .await
        .map_err(|e| DatabaseError::QueryFailed(e))?;

    results
        .into_iter()
        .map(|db_model| E::from_db_model(db_model).map_err(DatabaseError::from))
        .collect()
}

/// Query entities with a WHERE clause
/// Note: This follows SurrealDB's query pattern for proper result handling
pub async fn query_entities<E: DbEntity, C: Connection>(
    conn: &Surreal<C>,
    where_clause: &str,
) -> Result<Vec<E::Domain>> {
    let query = format!("SELECT * FROM {} WHERE {}", E::table_name(), where_clause);

    let mut response = conn
        .query(query)
        .await
        .map_err(|e| DatabaseError::QueryFailed(e))?;

    // SurrealDB returns results wrapped in a response structure
    let results: Vec<E::DbModel> = response
        .take::<Vec<E::DbModel>>(0)
        .map_err(|e| DatabaseError::QueryFailed(e.into()))?;

    results
        .into_iter()
        .map(|db_model| E::from_db_model(db_model).map_err(DatabaseError::from))
        .collect()
}

// ============================================================================
// Relationship Operations using RELATE
// ============================================================================

/// Create a typed relationship between two entities using RELATE
pub async fn create_relation_typed<E: DbEntity, C: Connection>(
    conn: &Surreal<C>,
    edge_entity: &E,
) -> Result<E::Domain> {
    // Serialize the edge entity to get all its properties
    let db_model = E::to_db_model(edge_entity);
    let mut properties = serde_json::to_value(&db_model).map_err(DatabaseError::SerdeProblem)?;

    // Debug: print the serialized properties
    tracing::debug!(
        "create_relation_typed: table={}, properties={}",
        E::table_name(),
        serde_json::to_string_pretty(&properties)
            .unwrap_or_else(|_| "failed to serialize".to_string())
    );

    // Extract in and out from the entity
    // This assumes edge entities have in_id and out_id fields
    let obj = properties.as_object_mut().ok_or_else(|| {
        DatabaseError::QueryFailed(surrealdb::Error::Api(surrealdb::error::Api::Query(
            "Edge entity must serialize to an object".into(),
        )))
    })?;

    // Extract and remove in_id and out_id from the properties
    // These will be used in the RELATE clause, not in CONTENT
    let from_value = obj.remove("in").ok_or_else(|| {
        DatabaseError::QueryFailed(surrealdb::Error::Api(surrealdb::error::Api::Query(
            "Edge entity must have in_id field".into(),
        )))
    })?;

    let to_value = obj.remove("out").ok_or_else(|| {
        DatabaseError::QueryFailed(surrealdb::Error::Api(surrealdb::error::Api::Query(
            "Edge entity must have out_id field".into(),
        )))
    })?;

    // Remove the id field as well - SurrealDB generates this automatically for edge entities
    obj.remove("id");

    // Debug: print the raw values
    tracing::debug!("from_value: {:?}", from_value);
    tracing::debug!("to_value: {:?}", to_value);

    let from: surrealdb::RecordId = serde_json::from_value(from_value).unwrap();
    let to: surrealdb::RecordId = serde_json::from_value(to_value).unwrap();

    // Debug: print the extracted IDs
    tracing::debug!(
        "RELATE: from={}, to={}, table={}",
        from,
        to,
        E::table_name()
    );

    // Build the RELATE query
    let mut query = format!(
        "RELATE {from}->{relation_name}->{to}",
        from = from,
        relation_name = E::table_name(),
        to = to,
    );

    // Only add CONTENT if there are other properties besides in_id/out_id
    if !obj.is_empty() {
        query.push_str(" CONTENT ");
        query.push_str(&serde_json::to_string(&obj).map_err(DatabaseError::SerdeProblem)?);
    }

    let mut response = conn
        .query(query)
        .await
        .map_err(|e| DatabaseError::QueryFailed(e))?;

    // Extract the created edge entity
    let created: Vec<E::DbModel> = response
        .take(0)
        .map_err(|e| DatabaseError::QueryFailed(e))?;

    created
        .into_iter()
        .next()
        .ok_or_else(|| {
            DatabaseError::QueryFailed(surrealdb::Error::Api(surrealdb::error::Api::Query(
                "No edge entity returned from RELATE".into(),
            )))
        })
        .and_then(|db_model| E::from_db_model(db_model).map_err(DatabaseError::from))
}

/// Create a relationship between two entities using RELATE
pub async fn create_relation<C: Connection>(
    conn: &Surreal<C>,
    from: &RecordId,
    relation_name: &str,
    to: &RecordId,
    properties: Option<serde_json::Value>,
) -> Result<serde_json::Value> {
    let mut query = format!(
        "RELATE {from}->{relation_name}->{to}",
        from = from,
        relation_name = relation_name,
        to = to,
    );

    let keys = properties
        .as_ref()
        .and_then(|v| v.as_object().map(|v| v.keys().cloned().collect::<Vec<_>>()))
        .unwrap_or_default();

    if let Some(props) = properties {
        query.push_str(" CONTENT ");
        query.push_str(&serde_json::to_string(&props).map_err(DatabaseError::SerdeProblem)?);
    }

    let mut response = conn
        .query(query)
        .await
        .map_err(|e| DatabaseError::QueryFailed(e))?;
    println!("response: {:#?}", response);

    let mut output = json!({});

    for key in keys {
        let value: Option<serde_json::Value> = response
            .take(key.as_str())
            .map_err(|e| DatabaseError::QueryFailed(e))?;
        if let Some(value) = value {
            output[key] = value;
        }
    }

    Ok(output)
}

/// Query relationships from an entity
/// Returns the related entity IDs (not the full entities)
pub async fn query_relations_from<C: Connection>(
    conn: &Surreal<C>,
    from: &RecordId,
    relation_name: &str,
    to_table: &str,
) -> Result<Vec<RecordId>> {
    let query = format!(
        "SELECT id, ->{relation_name}->{to_table} AS related_entities FROM $parent ORDER BY id ASC",
        relation_name = relation_name,
        to_table = to_table,
    );

    let mut response = conn
        .query(query)
        .bind(("parent", from.clone()))
        .await
        .map_err(|e| DatabaseError::QueryFailed(e))?;

    // Extract the response
    let result: Vec<serde_json::Value> = response
        .take(0)
        .map_err(|e| DatabaseError::QueryFailed(e.into()))?;

    // Parse the result structure
    if let Some(first) = result.first() {
        if let Some(related) = first.get("related_entities").and_then(|v| v.as_array()) {
            return Ok(related
                .iter()
                .filter_map(|v| serde_json::from_value::<RecordId>(v.clone()).ok())
                .collect());
        }
    }

    Ok(vec![])
}

/// Query relationships to an entity (reverse direction)
/// Returns the related entity IDs (not the full entities)
pub async fn query_relations_to<C: Connection>(
    conn: &Surreal<C>,
    to: &RecordId,
    relation_name: &str,
    from_table: &str,
) -> Result<Vec<RecordId>> {
    let query = format!(
        "SELECT id, <-{relation_name}<-{from_table} AS related_entities FROM $parent ORDER BY id ASC",
        relation_name = relation_name,
        from_table = from_table,
    );

    let mut response = conn
        .query(query)
        .bind(("parent", to.clone()))
        .await
        .map_err(|e| DatabaseError::QueryFailed(e))?;

    // Extract the response
    let result: Vec<serde_json::Value> = response
        .take(0)
        .map_err(|e| DatabaseError::QueryFailed(e.into()))?;

    // Parse the result structure
    if let Some(first) = result.first() {
        if let Some(related) = first.get("related_entities").and_then(|v| v.as_array()) {
            return Ok(related
                .iter()
                .filter_map(|v| serde_json::from_value::<RecordId>(v.clone()).ok())
                .collect());
        }
    }

    Ok(vec![])
}

/// Delete a relationship between two entities
pub async fn delete_relation<C: Connection>(
    conn: &Surreal<C>,
    from: &RecordId,
    relation_name: &str,
    to: &RecordId,
) -> Result<()> {
    let query = format!(
        "DELETE FROM {relation_name} WHERE in = {from} AND out = {to}",
        relation_name = relation_name,
        from = from,
        to = to,
    );

    conn.query(query)
        .await
        .map_err(|e| DatabaseError::QueryFailed(e))?;

    Ok(())
}

// ============================================================================
// Specialized Operations - Vector Search
// ============================================================================

/// Extension trait for vector search operations
pub trait VectorSearchExt<C: Connection> {
    /// Search memories by semantic similarity
    fn search_memories(
        &self,
        embeddings: &dyn EmbeddingProvider,
        agent_id: AgentId,
        query: &str,
        limit: usize,
    ) -> impl Future<Output = Result<Vec<(MemoryBlock, f32)>>>;
}

impl<C: Connection> VectorSearchExt<C> for Surreal<C> {
    async fn search_memories(
        &self,
        embeddings: &dyn EmbeddingProvider,
        agent_id: AgentId,
        query: &str,
        limit: usize,
    ) -> Result<Vec<(MemoryBlock, f32)>> {
        // Generate embedding for query
        let query_embedding = embeddings.embed(query).await?;

        // Use SurrealDB's vector search
        let query_str = format!(
            r#"
            SELECT *,
                vector::distance::knn(embedding, $query_embedding) as score
            FROM {}
            WHERE {} IN agents
            ORDER BY score DESC
            LIMIT {}
            "#,
            MemoryIdType::PREFIX,
            RecordId::from(agent_id),
            limit
        );

        let results: Vec<serde_json::Value> = self
            .query(&query_str)
            .bind(("query_embedding", query_embedding.vector))
            .await
            .map_err(|e| DatabaseError::QueryFailed(e))?
            .take(0)
            .map_err(|e| DatabaseError::QueryFailed(e))?;

        // Convert results
        let mut memories = Vec::new();
        for result in results {
            if let Ok(memory) = serde_json::from_value::<MemoryBlock>(result.clone()) {
                let score = result.get("score").and_then(|s| s.as_f64()).unwrap_or(0.0) as f32;
                memories.push((memory, score));
            }
        }
        Ok(memories)
    }
}

// ============================================================================
// Live Query Operations - Free Functions
// ============================================================================

/// Subscribe to memory updates for a specific memory
pub async fn subscribe_to_memory_updates<C: Connection>(
    conn: &Surreal<C>,
    memory_id: MemoryId,
) -> Result<impl Stream<Item = (Action, MemoryBlock)>> {
    let stream = conn
        .select((MemoryIdType::PREFIX, memory_id.uuid().to_string()))
        .live()
        .await
        .map_err(DatabaseError::QueryFailed)?;

    Ok(stream.filter_map(
        |notif: surrealdb::Result<Notification<serde_json::Value>>| async move {
            match notif {
                Ok(Notification { action, data, .. }) => {
                    if let Ok(memory) = serde_json::from_value::<MemoryBlock>(data) {
                        Some((action, memory))
                    } else {
                        None
                    }
                }
                Err(_) => None,
            }
        },
    ))
}

/// Subscribe to all memory updates for an agent
pub async fn subscribe_to_agent_memory_updates<C: Connection>(
    conn: &Surreal<C>,
    agent_id: AgentId,
) -> Result<impl Stream<Item = (Action, MemoryBlock)>> {
    // Watch memory blocks that have this agent in their agents array
    let query = format!(
        "LIVE SELECT * FROM mem WHERE {} IN agents",
        RecordId::from(agent_id)
    );

    let mut result = conn
        .query(query)
        .await
        .map_err(DatabaseError::QueryFailed)?;

    let stream = result
        .stream::<Notification<serde_json::Value>>(0)
        .map_err(DatabaseError::QueryFailed)?;

    Ok(stream.filter_map(
        |notif: surrealdb::Result<Notification<serde_json::Value>>| async move {
            match notif {
                Ok(Notification { action, data, .. }) => {
                    if let Ok(memory) = serde_json::from_value::<MemoryBlock>(data) {
                        Some((action, memory))
                    } else {
                        None
                    }
                }
                Err(_) => None,
            }
        },
    ))
}

// ============================================================================
// Specialized Operations - Memory Management
// ============================================================================

/// Attach a memory block to an agent with specific access level
pub async fn attach_memory_to_agent<C: Connection>(
    conn: &Surreal<C>,
    agent_id: AgentId,
    memory_id: MemoryId,
    access_level: crate::agent::MemoryAccessLevel,
) -> Result<()> {
    use crate::db::entity::AgentMemoryRelation;

    // Create the edge entity (without ID - SurrealDB will generate it)
    let relation = AgentMemoryRelation {
        id: None, // SurrealDB will generate the edge entity ID
        in_id: agent_id,
        out_id: memory_id,
        access_level,
        created_at: chrono::Utc::now(),
    };

    // Create the relation using the typed function
    let _created = create_relation_typed(conn, &relation).await?;
    tracing::debug!("Created agent-memory relation: {:?}", _created);

    Ok(())
}

/// Get all memories accessible to an agent
pub async fn get_agent_memories<C: Connection>(
    conn: &Surreal<C>,
    agent_id: AgentId,
) -> Result<Vec<(MemoryBlock, crate::agent::MemoryAccessLevel)>> {
    use crate::db::entity::AgentMemoryRelation;

    // Query the edge entities for this agent
    let query = r#"
        SELECT * FROM agent_memories
        WHERE in = $agent_id
    "#;

    let mut result = conn
        .query(query)
        .bind(("agent_id", RecordId::from(agent_id)))
        .await
        .map_err(DatabaseError::QueryFailed)?;

    // Take the DB models (which have the serde rename attributes)
    let relation_db_models: Vec<<AgentMemoryRelation as DbEntity>::DbModel> =
        result.take(0).map_err(DatabaseError::QueryFailed)?;

    // Convert DB models to domain types
    let relations: Vec<AgentMemoryRelation> = relation_db_models
        .into_iter()
        .map(|db_model| AgentMemoryRelation::from_db_model(db_model).map_err(DatabaseError::from))
        .collect::<Result<Vec<_>>>()?;

    // Now fetch the actual memory blocks
    let mut memories = Vec::new();
    for relation in relations {
        let memory_db: Option<<MemoryBlock as DbEntity>::DbModel> = conn
            .select(RecordId::from(relation.out_id))
            .await
            .map_err(DatabaseError::QueryFailed)?;

        if let Some(db_model) = memory_db {
            let memory = MemoryBlock::from_db_model(db_model)?;
            memories.push((memory, relation.access_level));
        }
    }

    Ok(memories)
}

/// Get memory by label for an agent
pub async fn get_memory_by_label<C: Connection>(
    conn: &Surreal<C>,
    agent_id: AgentId,
    label: &str,
) -> Result<Option<MemoryBlock>> {
    // First find memory blocks accessible to this agent through the junction table
    let query = r#"
        SELECT *, out.* AS memory_data
        FROM agent_memories
        WHERE in = $agent_id
        AND out.label = $label
    "#;

    let mut result = conn
        .query(query)
        .bind(("agent_id", RecordId::from(agent_id)))
        .bind(("label", label.to_string()))
        .await
        .map_err(|e| DatabaseError::QueryFailed(e))?;

    tracing::trace!("memory label query result: {:?}", result.pretty_debug());

    let records: Vec<serde_json::Value> = result
        .take("memory_data")
        .map_err(|e| DatabaseError::QueryFailed(e))?;

    // Extract the memory from the result
    if let Some(record) = records.into_iter().next() {
        tracing::trace!("record: {:?}", record);
        Ok(serde_json::from_value(record).ok())
    } else {
        Ok(None)
    }
}

/// Update memory content with optional re-embedding
pub async fn update_memory_content<C: Connection>(
    conn: &Surreal<C>,
    memory_id: MemoryId,
    content: String,
    embeddings: Option<&impl EmbeddingProvider>,
) -> Result<()> {
    // Generate embeddings if provider is available
    let (embedding, model_name) = if let Some(provider) = embeddings {
        let emb = provider.embed_query(&content).await.map_err(|e| {
            DatabaseError::QueryFailed(surrealdb::Error::Db(surrealdb::error::Db::Tx(format!(
                "Embedding generation failed: {}",
                e
            ))))
        })?;
        (Some(emb), Some(provider.model_name().to_string()))
    } else {
        (Some(vec![0.0; 384]), Some("none".to_string()))
    };

    let _: Option<serde_json::Value> = conn
        .update(RecordId::from(memory_id))
        .patch(PatchOp::replace("/content", content))
        .patch(PatchOp::replace("/embedding", embedding))
        .patch(PatchOp::replace("/embedding_model", model_name))
        .patch(PatchOp::replace(
            "/updated_at",
            surrealdb::Datetime::from(Utc::now()),
        ))
        .await
        .map_err(DatabaseError::QueryFailed)?;

    Ok(())
}

// ============================================================================
// Agent-Message Persistence Operations
// ============================================================================

/// Persist a message and create agent-message relation
pub async fn persist_agent_message<C: Connection>(
    conn: &Surreal<C>,
    agent_id: AgentId,
    message: &Message,
    message_type: crate::message::MessageRelationType,
) -> Result<()> {
    tracing::debug!(
        "Persisting message for agent {}: message_id={:?}, type={:?}",
        agent_id,
        message.id,
        message_type
    );

    // First, store the message
    let stored_message = message.store_with_relations(conn).await?;
    tracing::debug!("Stored message with id: {:?}", stored_message.id);

    // Then create the agent-message relation with position
    let position = crate::agent::get_next_message_position().await;

    let relation = crate::message::AgentMessageRelation {
        id: None,
        in_id: agent_id,
        out_id: stored_message.id.clone(),
        message_type,
        position: position.clone(),
        added_at: Utc::now(),
    };

    tracing::debug!(
        "Creating agent_messages relation: agent={}, message={:?}, position={}",
        agent_id,
        stored_message.id,
        position
    );

    let created_relation = create_relation_typed(conn, &relation).await?;
    tracing::debug!("Created relation: {:?}", created_relation);

    Ok(())
}

/// Archive messages by updating their relation type to Archived
pub async fn archive_agent_messages<C: Connection>(
    conn: &Surreal<C>,
    agent_id: AgentId,
    message_ids: &[MessageId],
) -> Result<()> {
    // Update all relations for these messages to Archived type
    let query = r#"
        UPDATE agent_messages
        SET message_type = $message_type
        WHERE in = $agent_id
        AND out IN $message_ids
    "#;

    conn.query(query)
        .bind(("agent_id", RecordId::from(agent_id)))
        .bind((
            "message_ids",
            message_ids
                .iter()
                .map(|id| RecordId::from(id.clone()))
                .collect::<Vec<_>>(),
        ))
        .bind(("message_type", "archived"))
        .await
        .map_err(DatabaseError::QueryFailed)?;

    Ok(())
}

/// Persist or update an agent's memory block with relation
pub async fn persist_agent_memory<C: Connection>(
    conn: &Surreal<C>,
    agent_id: AgentId,
    memory: &MemoryBlock,
    access_level: crate::agent::MemoryAccessLevel,
) -> Result<()> {
    // Store or update the memory block
    let stored_memory = memory.store_with_relations(conn).await?;

    // Check if relation already exists
    let existing_query = r#"
        SELECT * FROM agent_memories
        WHERE in = $agent_id AND out = $memory_id
    "#;

    let mut result = conn
        .query(existing_query)
        .bind(("agent_id", RecordId::from(agent_id)))
        .bind(("memory_id", RecordId::from(stored_memory.id)))
        .await
        .map_err(DatabaseError::QueryFailed)?;

    let existing: Vec<serde_json::Value> = result.take(0).map_err(DatabaseError::QueryFailed)?;

    if existing.is_empty() {
        // Create new relation
        let relation = crate::db::entity::AgentMemoryRelation {
            id: None,
            in_id: agent_id,
            out_id: stored_memory.id,
            access_level,
            created_at: Utc::now(),
        };

        create_relation_typed(conn, &relation).await?;
    }

    Ok(())
}

/// Update only agent runtime statistics
pub async fn update_agent_stats<C: Connection>(
    conn: &Surreal<C>,
    agent_id: AgentId,
    stats: &crate::context::state::AgentStats,
) -> Result<()> {
    // Use query builder to properly handle datetime types
    let query = r#"
        UPDATE agent
        SET
            total_messages = $total_messages,
            total_tool_calls = $total_tool_calls,
            last_active = $last_active,
            updated_at = $updated_at
        WHERE id = $id
    "#;

    let resp: surrealdb::Response = conn
        .query(query)
        .bind(("id", RecordId::from(agent_id)))
        .bind(("total_messages", stats.total_messages))
        .bind(("total_tool_calls", stats.total_tool_calls))
        .bind(("last_active", surrealdb::Datetime::from(stats.last_active)))
        .bind(("updated_at", surrealdb::Datetime::from(Utc::now())))
        .await
        .map_err(DatabaseError::QueryFailed)?;

    tracing::debug!("stats updated {:?}", resp.pretty_debug());

    Ok(())
}

/// Subscribe to agent stats updates
pub async fn subscribe_to_agent_stats<C: Connection>(
    conn: &Surreal<C>,
    agent_id: AgentId,
) -> Result<impl Stream<Item = (Action, crate::agent::AgentRecord)>> {
    let stream = conn
        .select((AgentIdType::PREFIX, agent_id.uuid().to_string()))
        .live()
        .await
        .map_err(DatabaseError::QueryFailed)?;

    Ok(stream.filter_map(
        |notif: surrealdb::Result<Notification<serde_json::Value>>| async move {
            match notif {
                Ok(Notification { action, data, .. }) => {
                    if let Ok(agent) = serde_json::from_value::<crate::agent::AgentRecord>(data) {
                        Some((action, agent))
                    } else {
                        None
                    }
                }
                Err(_) => None,
            }
        },
    ))
}

/// Load full agent state from database
pub async fn load_agent_state<C: Connection>(
    conn: &Surreal<C>,
    agent_id: AgentId,
) -> Result<(crate::agent::AgentRecord, Vec<Message>, Vec<MemoryBlock>)> {
    // Load the agent record with relations
    let agent = crate::agent::AgentRecord::load_with_relations(conn, agent_id)
        .await?
        .ok_or_else(|| DatabaseError::NotFound {
            entity_type: "agent".to_string(),
            id: agent_id.to_string(),
        })?;

    // Messages are loaded via the relation
    let messages: Vec<Message> = agent
        .messages
        .iter()
        .filter(|(_, rel)| rel.message_type == crate::message::MessageRelationType::Active)
        .map(|(msg, _)| msg.clone())
        .collect();

    // Memory blocks are loaded via the relation
    let memories: Vec<MemoryBlock> = agent.memories.iter().map(|(mem, _)| mem.clone()).collect();

    Ok((agent, messages, memories))
}

// ============================================================================
// Message Query Operations
// ============================================================================

/// Query messages using a custom query builder function
///
/// This function executes a query and deserializes the results from the database
/// model format to the domain model format.
///
/// # Example
/// ```no_run
/// use pattern_core::db::ops::query_messages;
/// use pattern_core::message::{Message, ChatRole};
///
/// let messages = query_messages(&db, |_| {
///     Message::search_by_role_query(&ChatRole::User)
/// }).await?;
/// ```
pub async fn query_messages<C, F>(db: &Surreal<C>, query_builder: F) -> Result<Vec<Message>>
where
    C: Connection,
    F: FnOnce(&Surreal<C>) -> String,
{
    let query = query_builder(db);
    let mut result = db.query(&query).await.map_err(DatabaseError::QueryFailed)?;

    let db_messages: Vec<<Message as DbEntity>::DbModel> =
        result.take(0).map_err(DatabaseError::QueryFailed)?;

    let messages: Result<Vec<_>> = db_messages
        .into_iter()
        .map(|db_msg| <Message as DbEntity>::from_db_model(db_msg).map_err(|e| e.into()))
        .collect();

    messages
}

/// Query messages with a pre-built query string
///
/// This is a simpler version when you already have the query string.
pub async fn query_messages_raw<C: Connection>(
    db: &Surreal<C>,
    query: &str,
) -> Result<Vec<Message>> {
    query_messages(db, |_| query.to_string()).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::{AgentRecord, AgentState, AgentType};
    use crate::db::{
        client,
        entity::{BaseTask, BaseTaskPriority, BaseTaskStatus},
    };
    use crate::id::{TaskId, UserId};
    use crate::users::User;

    #[tokio::test]
    async fn test_create_relation_with_properties() {
        let db = client::create_test_db().await.unwrap();

        // Create a user and agent first
        let user = User {
            id: UserId::generate(),
            ..Default::default()
        };
        let agent = AgentRecord {
            id: AgentId::generate(),
            name: "Test Agent".to_string(),
            agent_type: AgentType::Generic,
            owner_id: user.id,
            ..Default::default()
        };

        let created_user = create_entity::<User, _>(&db, &user).await.unwrap();
        let created_agent = create_entity::<AgentRecord, _>(&db, &agent).await.unwrap();

        // Test creating a relation with properties
        // Using string for access_level since it's an enum that serializes to lowercase strings
        let properties = serde_json::json!({
            "access_level": "write",
            "created_at": chrono::Utc::now(),
            "priority": 5
        });

        let relation = create_relation(
            &db,
            &RecordId::from(created_user.id),
            "manages",
            &RecordId::from(created_agent.id),
            Some(properties.clone()),
        )
        .await
        .unwrap();

        println!("Created relation: {:?}", relation);

        // Verify the relation was created with properties
        let query = r#"
            SELECT * FROM manages
            WHERE in = $user_id AND out = $agent_id
        "#;

        let mut result = db
            .query(query)
            .bind(("user_id", RecordId::from(created_user.id)))
            .bind(("agent_id", RecordId::from(created_agent.id)))
            .await
            .unwrap();
        println!("result: {:?}", result.pretty_debug());
        // Extract the fields we care about directly using take() with field paths
        let access_levels: Vec<String> = result.take("access_level").unwrap();
        let priorities: Vec<i64> = result.take("priority").unwrap();

        // Combine into relation objects
        let relations: Vec<serde_json::Value> = access_levels
            .into_iter()
            .zip(priorities.into_iter())
            .map(|(access_level, priority)| {
                serde_json::json!({
                    "access_level": access_level,
                    "priority": priority
                })
            })
            .collect();
        assert_eq!(relations.len(), 1);

        let rel = &relations[0];
        assert_eq!(rel["access_level"], "write");
        assert_eq!(rel["priority"], 5);
    }

    #[tokio::test]
    async fn test_entity_operations() {
        // Initialize the database
        let db = client::create_test_db().await.unwrap();

        // Create a user using generic operations
        let user = User {
            id: UserId::generate(),
            discord_id: Some("123456789".to_string()),
            ..Default::default()
        };

        let created_user = create_entity::<User, _>(&db, &user).await.unwrap();
        assert_eq!(created_user.id, user.id);

        // Get the user by ID
        let retrieved_user = get_entity::<User, _>(&db, &user.id).await.unwrap();
        assert!(retrieved_user.is_some());
        assert_eq!(retrieved_user.unwrap().id, user.id);

        // Create an agent
        let agent = AgentRecord {
            id: AgentId::generate(),
            name: "Test Agent".to_string(),
            agent_type: AgentType::Generic,
            state: AgentState::Ready,
            model_id: None,
            owner_id: user.id,
            ..Default::default()
        };

        let _created_agent = create_entity::<AgentRecord, _>(&db, &agent).await.unwrap();

        // Create ownership relationship using entity system
        let mut user_with_agent = user.clone();
        user_with_agent.owned_agent_ids = vec![agent.id];
        user_with_agent.store_relations(&db).await.unwrap();

        // Get agents for the user using entity system
        let retrieved_user = User::load_with_relations(&db, user.id)
            .await
            .unwrap()
            .unwrap();
        assert!(!retrieved_user.owned_agent_ids.is_empty());

        // Create a task
        let task = BaseTask {
            id: TaskId::generate(),
            title: "Test Task".to_string(),
            description: Some("A test task".to_string()),
            status: BaseTaskStatus::Pending,
            priority: BaseTaskPriority::Medium,
            ..Default::default()
        };

        let _created_task = create_entity::<BaseTask, _>(&db, &task).await.unwrap();

        // Create relationships using entity system
        let mut user_with_task = retrieved_user;
        user_with_task.created_task_ids = vec![task.id];
        user_with_task.store_relations(&db).await.unwrap();

        let mut agent_with_task = agent.clone();
        agent_with_task.assigned_task_ids = vec![task.id];
        agent_with_task.store_relations(&db).await.unwrap();

        // Query relationships using entity system
        let final_user = User::load_with_relations(&db, user.id)
            .await
            .unwrap()
            .unwrap();
        assert!(!final_user.created_task_ids.is_empty());

        let final_agent = AgentRecord::load_with_relations(&db, agent.id)
            .await
            .unwrap()
            .unwrap();
        assert!(!final_agent.assigned_task_ids.is_empty());
    }

    #[tokio::test]
    async fn test_simple_user_creation() {
        // Initialize the database
        let db = client::create_test_db().await.unwrap();

        // Create a simple user
        let user = User {
            id: UserId::generate(),
            ..Default::default()
        };

        // Debug: print what we're about to create
        println!("Creating user: {:?}", user);

        // Try to create the user
        let created_user = create_entity::<User, _>(&db, &user).await.unwrap();
        assert_eq!(created_user.id, user.id);
    }
}
