//! Database operations - direct, simple, no unnecessary abstractions

use super::{DatabaseError, Result, models::*, schema::*};

use crate::context::state::AgentContextMetadata;
use crate::context::{CompressionStrategy, ContextConfig};
use crate::embeddings::EmbeddingProvider;
use crate::id::{AgentId, AgentIdType, IdType, MemoryId, MemoryIdType, UserId, UserIdType};
use crate::memory::MemoryBlock;
use crate::message::Message;
use crate::utils::debug::ResponseExt;
use chrono::Utc;

use futures::{Stream, StreamExt};
use serde::Serialize;
use surrealdb::opt::PatchOp;

use surrealdb::{Action, Notification, RecordId};

pub trait SurrealExt<C> {
    fn create_user(
        &self,
        id: Option<UserId>,
        settings: serde_json::Value,
        metadata: serde_json::Value,
    ) -> impl Future<Output = Result<User>>;

    fn get_user(&self, id: UserId) -> impl Future<Output = Result<Option<User>>>;

    fn create_agent(
        &self,
        user_id: UserId,
        agent_type: crate::agent::AgentType,
        name: String,
        system_prompt: String,
        config: serde_json::Value,
        state: crate::agent::AgentState,
    ) -> impl Future<Output = Result<DbAgent>>;

    fn get_user_agents(&self, user_id: UserId) -> impl Future<Output = Result<Vec<DbAgent>>>;

    fn update_agent_state(
        &self,
        id: AgentId,
        state: crate::agent::AgentState,
    ) -> impl Future<Output = Result<()>>;

    fn create_memory(
        &self,
        embeddings: Option<&dyn EmbeddingProvider>,
        owner_id: UserId,
        label: String,
        content: String,
        description: Option<String>,
        metadata: serde_json::Value,
    ) -> impl Future<Output = Result<MemoryBlock>>;

    fn attach_memory_to_agent(
        &self,
        agent_id: AgentId,
        memory_id: MemoryId,
        access_level: crate::agent::MemoryAccessLevel,
    ) -> impl Future<Output = Result<()>>;

    fn get_agent_memories(
        &self,
        agent_id: AgentId,
    ) -> impl Future<Output = Result<Vec<(MemoryBlock, crate::agent::MemoryAccessLevel)>>>;

    fn search_memories(
        &self,
        embeddings: &dyn EmbeddingProvider,
        agent_id: AgentId,
        query: &str,
        limit: usize,
    ) -> impl Future<Output = Result<Vec<(MemoryBlock, f32)>>>;

    fn update_memory_content(
        &self,
        memory_id: MemoryId,
        content: String,
        embeddings: Option<&impl EmbeddingProvider>,
    ) -> impl Future<Output = Result<()>>;

    fn get_memory_by_label(
        &self,
        agent_id: AgentId,
        label: &str,
    ) -> impl Future<Output = Result<Option<MemoryBlock>>>;

    fn subscribe_to_memory_updates(
        &self,
        memory_id: MemoryId,
    ) -> impl Future<Output = Result<impl Stream<Item = (Action, MemoryBlock)>>>;

    fn subscribe_to_agent_memory_updates(
        &self,
        agent_id: AgentId,
    ) -> impl Future<Output = Result<impl Stream<Item = (Action, MemoryBlock)>>>;

    fn get_user_with_agents(
        &self,
        user_id: UserId,
    ) -> impl Future<Output = Result<Option<(User, Vec<DbAgent>)>>>;

    fn update_agent_context(
        &self,
        agent_id: AgentId,
        memory_blocks: Vec<MemoryBlock>,
        messages: Vec<Message>,
        archived_messages: Vec<Message>,
        message_summary: Option<String>,
        context_config: ContextConfig,
        compression_strategy: CompressionStrategy,
        metadata: AgentContextMetadata,
    ) -> impl Future<Output = Result<()>>;
}

impl<T, C> SurrealExt<C> for T
where
    T: AsRef<surrealdb::Surreal<C>>,
    C: surrealdb::Connection,
{
    // ===== User Operations =====
    /// Create a new user
    async fn create_user(
        &self,
        id: Option<UserId>,
        settings: serde_json::Value,
        metadata: serde_json::Value,
    ) -> Result<User> {
        let id = id.unwrap_or_else(|| UserId::generate());
        let now = chrono::Utc::now();

        // Create DB wrapper type directly
        let db_user = DbUser {
            id: RecordId::from(id),
            created_at: now.into(),
            updated_at: now.into(),
            settings: serde_json::from_value(settings).unwrap_or_default(),
            metadata: serde_json::from_value(metadata).unwrap_or_default(),
        };

        let created: Option<DbUser> = self
            .as_ref()
            .create(UserIdType::PREFIX)
            .content(db_user)
            .await
            .map_err(|e| DatabaseError::QueryFailed(e))?;

        created
            .map(|db| {
                db.try_into()
                    .map_err(|_| DatabaseError::Other("Failed to parse agent ID".into()))
            })
            .ok_or_else(|| DatabaseError::Other("Failed to create agent".into()))?
    }

    /// Get a user by ID
    async fn get_user(&self, id: UserId) -> Result<Option<User>> {
        let db_user: Option<DbUser> = self
            .as_ref()
            .select((UserIdType::PREFIX, id.uuid().to_string()))
            .await
            .map_err(|e| DatabaseError::QueryFailed(e))?;

        Ok(db_user.map(|db| db.try_into().ok()).flatten())
    }

    // ===== Agent Operations =====

    /// Create a new agent
    async fn create_agent(
        &self,
        user_id: UserId,
        agent_type: crate::agent::AgentType,
        name: String,
        system_prompt: String,
        config: serde_json::Value,
        state: crate::agent::AgentState,
    ) -> Result<DbAgent> {
        let id = AgentId::generate();
        let now = chrono::Utc::now();

        // Create DB wrapper type directly
        let db_agent = DbAgent {
            id: RecordId::from(id),
            user_id: user_id.into(),
            agent_type,
            name,
            system_prompt,
            state,
            memory_blocks: vec![],
            messages: vec![],
            archived_messages: vec![],
            message_summary: None,
            context_config: serde_json::to_value(ContextConfig::default())
                .expect("ContextConfig should serialize"),
            compression_strategy: serde_json::to_value(CompressionStrategy::default())
                .expect("CompressionStrategy should serialize"),
            created_at: now.into(),
            last_active: now.into(),
            total_messages: 0,
            total_tool_calls: 0,
            context_rebuilds: 0,
            compression_events: 0,
            config,
            updated_at: now.into(),
            is_active: true,
        };

        let created: DbAgent = self
            .as_ref()
            .create((AgentIdType::PREFIX, id.uuid().to_string()))
            .content(db_agent)
            .await
            .map_err(|e| DatabaseError::QueryFailed(e))?
            .expect("agent should have bee n created");

        println!("agent: {:?}", created);
        Ok(created)
    }

    /// Get agents for a user
    async fn get_user_agents(&self, user_id: UserId) -> Result<Vec<DbAgent>> {
        let query = format!(
            "SELECT * FROM {} WHERE user_id = {}:`{}` AND is_active = true
                FETCH memory_blocks, messages, archived_messages",
            AgentIdType::PREFIX,
            UserIdType::PREFIX,
            user_id.uuid()
        );

        let db_agents: Vec<DbAgent> = self
            .as_ref()
            .query(&query)
            .await
            .map_err(|e| DatabaseError::QueryFailed(e))?
            .take(0)
            .map_err(|e| DatabaseError::QueryFailed(e))?;

        // Convert DbAgent results back to domain types
        db_agents
            .into_iter()
            .map(|db| db.try_into())
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|_| DatabaseError::Other("Failed to parse agents".into()))
    }

    /// Update agent state
    async fn update_agent_state(&self, id: AgentId, state: crate::agent::AgentState) -> Result<()> {
        #[derive(Serialize)]
        struct AgentUpdate {
            state: crate::agent::AgentState,
            updated_at: surrealdb::Datetime,
        }

        let update = AgentUpdate {
            state,
            updated_at: chrono::Utc::now().into(),
        };

        let _: Option<DbAgent> = self
            .as_ref()
            .update((AgentIdType::PREFIX, id.uuid().to_string()))
            .merge(update)
            .await
            .map_err(|e| DatabaseError::QueryFailed(e))?;
        Ok(())
    }

    // ===== Memory Operations =====

    /// Create a memory block with optional embedding
    async fn create_memory(
        &self,
        embeddings: Option<&dyn EmbeddingProvider>,
        owner_id: UserId,
        label: String,
        content: String,
        description: Option<String>,
        metadata: serde_json::Value,
    ) -> Result<MemoryBlock> {
        let id = MemoryId::generate();
        let now = chrono::Utc::now();

        // Generate embedding if provider is available
        let (embedding, embedding_model) = if let Some(provider) = embeddings {
            let embedding = provider.embed(&content).await?;
            (
                Some(embedding.vector),
                Some(provider.model_id().to_string()),
            )
        } else {
            // Create a dummy embedding with 384 dimensions (matching jina-embeddings-v2-small-en)
            (Some(vec![0.0; 384]), Some("none".to_string()))
        };

        let db_memory_block = DbMemoryBlock {
            id: RecordId::from(id),
            owner_id: owner_id.into(),
            label,
            content,
            description,
            embedding,
            embedding_model,
            agents: Vec::new(),
            metadata,
            created_at: now.into(),
            updated_at: now.into(),
        };

        let created: Option<DbMemoryBlock> = self
            .as_ref()
            .create((MemoryIdType::PREFIX, id.uuid().to_string()))
            .content(db_memory_block)
            .await
            .map_err(|e| DatabaseError::QueryFailed(e))?;

        created
            .map(|db| {
                db.try_into()
                    .map_err(|_| DatabaseError::Other("Failed to parse memory ID".into()))
            })
            .ok_or_else(|| DatabaseError::Other("Failed to create memory block".into()))?
    }

    /// Search memories by semantic similarity
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
            WHERE agent_id = {}:`{}`
            ORDER BY score DESC
            LIMIT {}
            "#,
            MemoryIdType::PREFIX,
            AgentIdType::PREFIX,
            agent_id.uuid(),
            limit
        );

        let results: Vec<DbMemoryBlock> = self
            .as_ref()
            .query(&query_str)
            .bind(("query_embedding", query_embedding.vector))
            .await
            .map_err(|e| DatabaseError::QueryFailed(e))?
            .take(0)
            .map_err(|e| DatabaseError::QueryFailed(e))?;

        // Convert from DB types and return with dummy scores until we figure out how to extract them
        let mut memories = Vec::new();
        for db_memory in results.into_iter() {
            // We need to reconstruct the RecordId from the query result
            match db_memory.try_into() {
                Ok(memory) => memories.push((memory, 1.0)),
                Err(_) => continue, // Skip memories that fail to parse
            }
        }
        Ok(memories)
    }

    /// Get memory by label for an agent
    async fn get_memory_by_label(
        &self,
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

        let mut result = self
            .as_ref()
            .query(query)
            .bind(("agent_id", RecordId::from(agent_id)))
            .bind(("label", label.to_string()))
            .await
            .map_err(|e| DatabaseError::QueryFailed(e))?;

        tracing::trace!("memory label query result: {:?}", result.pretty_debug());

        let records: Vec<DbMemoryBlock> = result
            .take("memory_data")
            .map_err(|e| DatabaseError::QueryFailed(e))?;

        // Extract the memory from the result
        if let Some(record) = records.into_iter().next() {
            tracing::trace!("record: {:?}", record);
            Ok(Some(MemoryBlock::try_from(record)?))
        } else {
            Ok(None)
        }
    }

    /// Get user with agents in one query
    /// Get user with their agents
    async fn get_user_with_agents(&self, user_id: UserId) -> Result<Option<(User, Vec<DbAgent>)>> {
        // Get user first
        let user = self.get_user(user_id).await?;

        match user {
            Some(u) => {
                // Get agents for this user
                let agents = self.get_user_agents(user_id).await?;
                Ok(Some((u, agents)))
            }
            None => Ok(None),
        }
    }

    async fn attach_memory_to_agent(
        &self,
        agent_id: AgentId,
        memory_id: MemoryId,
        access_level: crate::agent::MemoryAccessLevel,
    ) -> Result<()> {
        let access_str = match access_level {
            crate::agent::MemoryAccessLevel::Read => "read",
            crate::agent::MemoryAccessLevel::Write => "write",
            crate::agent::MemoryAccessLevel::Admin => "admin",
        };

        let query = format!(
            "
            RELATE $agent_id->agent_memories->$memory_id
                SET access_level = $access_level,
                    created_at = time::now()
            ;
            UPDATE $memory_id SET agents += $agent_id WHERE $agent_id NOT IN agents;
            UPDATE $agent_id SET memory_blocks += $memory_id;
            "
        );

        let result = self
            .as_ref()
            .query(query)
            .bind(("agent_id", RecordId::from(agent_id)))
            .bind(("memory_id", RecordId::from(memory_id)))
            .bind(("access_level", access_str))
            .await
            .map_err(DatabaseError::QueryFailed)?;

        tracing::trace!("attach result: {:?}", result.pretty_debug());

        Ok(())
    }

    async fn get_agent_memories(
        &self,
        agent_id: AgentId,
    ) -> Result<Vec<(MemoryBlock, crate::agent::MemoryAccessLevel)>> {
        let query = r#"
            SELECT *, out.* AS memory_data
            FROM agent_memories
            WHERE in = $agent_id
        "#;

        let mut result = self
            .as_ref()
            .query(query)
            .bind(("agent_id", RecordId::from(agent_id)))
            .await
            .map_err(DatabaseError::QueryFailed)?;

        tracing::trace!("query_result: {:?}", result.pretty_debug());

        let records: Vec<AccessBlock> = result.take(0).map_err(DatabaseError::QueryFailed)?;

        let mut memories = Vec::new();
        for record in records {
            let memory = MemoryBlock::try_from(record.memory_data).expect("should be a memory?");

            memories.push((memory, record.access_level));
        }

        Ok(memories)
    }

    async fn update_memory_content(
        &self,
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

        println!("memory_id {}", RecordId::from(memory_id));

        let block: Option<DbMemoryBlock> = self
            .as_ref()
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

        println!("updated block {:?}", block);

        Ok(())
    }

    async fn subscribe_to_memory_updates(
        &self,
        memory_id: MemoryId,
    ) -> Result<impl Stream<Item = (Action, MemoryBlock)>> {
        let stream = self
            .as_ref()
            .select((MemoryIdType::PREFIX, memory_id.uuid().to_string()))
            .live()
            .await
            .map_err(DatabaseError::QueryFailed)?;

        Ok(stream.filter_map(
            |notif: surrealdb::Result<Notification<DbMemoryBlock>>| async move {
                match notif {
                    Ok(Notification {
                        query_id: _,
                        action,
                        data,
                        ..
                    }) => Some((
                        action,
                        MemoryBlock::try_from(data)
                            .expect("memory block conversion should succeed"),
                    )),
                    Err(_) => None,
                }
            },
        ))
    }

    async fn subscribe_to_agent_memory_updates(
        &self,
        agent_id: AgentId,
    ) -> Result<impl Stream<Item = (Action, MemoryBlock)>> {
        // Watch memory blocks that have this agent in their agents array
        let query = format!(
            "LIVE SELECT * FROM mem WHERE {} IN agents",
            RecordId::from(agent_id)
        );

        let mut result = self
            .as_ref()
            .query(query)
            .await
            .map_err(DatabaseError::QueryFailed)?;

        let stream = result
            .stream::<Notification<DbMemoryBlock>>(0)
            .map_err(DatabaseError::QueryFailed)?;

        Ok(stream.filter_map(
            |notif: surrealdb::Result<Notification<DbMemoryBlock>>| async move {
                match notif {
                    Ok(Notification {
                        query_id: _,
                        action,
                        data,
                        ..
                    }) => Some((
                        action,
                        MemoryBlock::try_from(data)
                            .expect("memory block conversion should succeed"),
                    )),
                    Err(_) => None,
                }
            },
        ))
    }

    async fn update_agent_context(
        &self,
        agent_id: AgentId,
        memory_blocks: Vec<MemoryBlock>,
        messages: Vec<Message>,
        archived_messages: Vec<Message>,
        message_summary: Option<String>,
        context_config: ContextConfig,
        compression_strategy: CompressionStrategy,
        metadata: AgentContextMetadata,
    ) -> Result<()> {
        #[derive(Serialize)]
        struct ContextUpdate {
            memory_blocks: Vec<RecordId>,
            messages: Vec<RecordId>,
            archived_messages: Vec<RecordId>,
            message_summary: Option<String>,
            context_config: serde_json::Value,
            compression_strategy: serde_json::Value,
            last_active: surrealdb::Datetime,
            total_messages: usize,
            total_tool_calls: usize,
            context_rebuilds: usize,
            compression_events: usize,
            updated_at: surrealdb::Datetime,
        }
        let mut block_ids = Vec::with_capacity(memory_blocks.capacity());
        let mut archived_msgs = Vec::with_capacity(archived_messages.capacity());

        let mut msgs = Vec::with_capacity(messages.capacity());

        for block in &memory_blocks {
            block_ids.push(RecordId::from(block.id));
            // let db_block: DbMemoryBlock = block.clone().into();
            // let _: Option<DbMemoryBlock> = self
            //     .as_ref()
            //     .upsert(RecordId::from(block.id))
            //     .content(db_block)
            //     .await
            //     .map_err(|e| DatabaseError::QueryFailed(e))?;
        }

        for msg in &archived_messages {
            archived_msgs.push(RecordId::from(msg.id.clone()));
            // let _: Option<Message> = self
            //     .as_ref()
            //     .upsert(RecordId::from(msg.id))
            //     .content(msg.clone())
            //     .await
            //     .map_err(|e| DatabaseError::QueryFailed(e))?;
        }

        for msg in &messages {
            msgs.push(RecordId::from(msg.id.clone()));
            // let _: Option<Message> = self
            //     .as_ref()
            //     .upsert(RecordId::from(msg.id))
            //     .content(msg.clone())
            //     .await
            //     .map_err(|e| DatabaseError::QueryFailed(e))?;
        }

        let update = ContextUpdate {
            memory_blocks: block_ids,
            messages: msgs,
            archived_messages: archived_msgs,
            message_summary,
            context_config: serde_json::to_value(context_config)
                .expect("ContextConfig should serialize"),
            compression_strategy: serde_json::to_value(compression_strategy)
                .expect("CompressionStrategy should serialize"),
            last_active: metadata.last_active.into(),
            total_messages: metadata.total_messages,
            total_tool_calls: metadata.total_tool_calls,
            context_rebuilds: metadata.context_rebuilds,
            compression_events: metadata.compression_events,
            updated_at: chrono::Utc::now().into(),
        };

        let update_query = format!(
            "UPDATE {} MERGE $agent_update RETURN NONE",
            RecordId::from(agent_id)
        );

        let _: surrealdb::Response = self
            .as_ref()
            .query(update_query)
            .bind(("agent_update", update))
            .await
            .map_err(|e| DatabaseError::QueryFailed(e))?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use uuid::Uuid;

    use super::*;
    use crate::agent::{AgentState, AgentType};
    use crate::db::client;
    use std::str::FromStr;
    use std::sync::Arc;

    #[tokio::test]
    async fn test_database_operations() {
        // Initialize the database (which runs migrations)
        let db = Arc::new(client::create_test_db().await.unwrap());

        // Test 1: Typed IDs in database

        // Create a user with typed IDs
        let user_id = UserId::generate();
        let user = db
            .create_user(
                Some(user_id),
                serde_json::json!({"theme": "dark"}),
                serde_json::json!({"source": "test"}),
            )
            .await
            .unwrap();

        // Get the user by ID
        let retrieved_user = db.get_user(user_id).await.unwrap().unwrap();
        assert_eq!(retrieved_user.id, user.id);

        // Create an agent with typed IDs
        let agent = db
            .create_agent(
                user.id,
                AgentType::Generic,
                "Test Agent".to_string(),
                "You are a test agent".to_string(),
                serde_json::json!({}),
                AgentState::Ready,
            )
            .await
            .unwrap();

        assert_eq!(agent.user_id, RecordId::from(user.id));
        assert_eq!(agent.agent_type, AgentType::Generic);
        assert_eq!(agent.state, AgentState::Ready);

        // Get agents for the user
        let agents = db.get_user_agents(user_id).await.unwrap();
        assert_eq!(agents.len(), 1);
        assert_eq!(agents[0].id, agent.id);

        // Update agent state
        db.update_agent_state(
            AgentId::from_uuid(
                Uuid::from_str(strip_brackets(&agent.id.key().to_string())).unwrap(),
            ),
            AgentState::Processing,
        )
        .await
        .unwrap();

        // Create a memory block
        let memory = db
            .create_memory(
                None, // No embedding provider for test
                user_id,
                "test_memory".to_string(),
                "Test content".to_string(),
                Some("Test description".to_string()),
                serde_json::json!({"test": true}),
            )
            .await
            .unwrap();

        // Verify memory ID is properly typed
        assert!(memory.id.to_string().starts_with("mem_"));
        assert_eq!(memory.owner_id, user_id);

        // Attach memory to agent
        db.attach_memory_to_agent(
            AgentId::from_uuid(
                Uuid::from_str(strip_brackets(&agent.id.key().to_string())).unwrap(),
            ),
            memory.id,
            crate::agent::MemoryAccessLevel::Read,
        )
        .await
        .unwrap();

        // Get memory by label
        let retrieved_memory = db
            .get_memory_by_label(
                AgentId::from_uuid(
                    Uuid::from_str(strip_brackets(&agent.id.key().to_string())).unwrap(),
                ),
                "test_memory",
            )
            .await
            .unwrap()
            .unwrap();
        assert_eq!(retrieved_memory.id, memory.id);

        // Get user with agents
        let (user_with_agents, agents) = db.get_user_with_agents(user.id).await.unwrap().unwrap();
        assert_eq!(user_with_agents.id, user.id);
        assert_eq!(agents.len(), 1);
        assert_eq!(agents[0].id, agent.id);

        // Test 2: Agent type serialization

        let user = db
            .create_user(None, serde_json::json!({}), serde_json::json!({}))
            .await
            .unwrap();

        // Test custom agent type
        let _agent = db
            .create_agent(
                user.id,
                AgentType::Custom("special".to_string()),
                "Special Agent".to_string(),
                "You are special".to_string(),
                serde_json::json!({}),
                AgentState::Ready,
            )
            .await
            .unwrap();

        // Get agents and verify custom type is serialized correctly
        let agents = db.get_user_agents(user.id).await.unwrap();
        assert_eq!(agents.len(), 1);
        match &agents[0].agent_type {
            AgentType::Custom(name) => assert_eq!(name, "special"),
            _ => panic!("Expected custom agent type"),
        }
    }
    #[test]
    fn test_context_config_serialization() {
        use crate::context::{CompressionStrategy, ContextConfig};

        // Test that default values serialize correctly
        let config = ContextConfig::default();
        let config_json = serde_json::to_value(&config).unwrap();
        assert!(config_json.is_object());
        assert!(config_json["base_instructions"].is_string());

        // Test that we can deserialize it back
        let _config2: ContextConfig = serde_json::from_value(config_json).unwrap();

        // Test compression strategy
        let strategy = CompressionStrategy::default();
        let strategy_json = serde_json::to_value(&strategy).unwrap();
        assert!(strategy_json.is_object());
        assert_eq!(strategy_json["type"], "truncate");
        assert_eq!(strategy_json["keep_recent"], 50);

        // Test that we can deserialize it back
        let _strategy2: CompressionStrategy = serde_json::from_value(strategy_json).unwrap();
    }
}
