//! Database operations - direct, simple, no unnecessary abstractions

use super::{DatabaseError, Result, models::*, schema::*};
use crate::embeddings::EmbeddingProvider;
use crate::id::{AgentId, AgentIdType, IdType, MemoryId, MemoryIdType, UserId, UserIdType};
use serde::Serialize;
use surrealdb::RecordId;

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
    ) -> impl Future<Output = Result<Agent>>;

    fn get_user_agents(&self, user_id: UserId) -> impl Future<Output = Result<Vec<Agent>>>;

    fn update_agent_state(
        &self,
        id: AgentId,
        state: crate::agent::AgentState,
    ) -> impl Future<Output = Result<()>>;

    fn create_memory(
        &self,
        embeddings: Option<&dyn EmbeddingProvider>,
        agent_id: AgentId,
        label: String,
        content: String,
        description: Option<String>,
        metadata: serde_json::Value,
    ) -> impl Future<Output = Result<MemoryBlock>>;

    fn search_memories(
        &self,
        embeddings: &dyn EmbeddingProvider,
        agent_id: AgentId,
        query: &str,
        limit: usize,
    ) -> impl Future<Output = Result<Vec<(MemoryBlock, f32)>>>;

    fn get_memory_by_label(
        &self,
        agent_id: AgentId,
        label: &str,
    ) -> impl Future<Output = Result<Option<MemoryBlock>>>;

    fn get_user_with_agents(
        &self,
        user_id: UserId,
    ) -> impl Future<Output = Result<Option<(User, Vec<Agent>)>>>;
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
                    .map_err(|_| DatabaseError::Other("Failed to parse user ID".into()))
            })
            .ok_or_else(|| DatabaseError::Other("Failed to create user".into()))?
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
    ) -> Result<Agent> {
        let id = AgentId::generate();
        let now = chrono::Utc::now();

        // Create DB wrapper type directly
        let db_agent = DbAgent {
            id: RecordId::from(id),
            user_id: user_id.into(),
            agent_type,
            name,
            system_prompt,
            config,
            state,
            created_at: now.into(),
            updated_at: now.into(),
            is_active: true,
        };

        let created: Option<DbAgent> = self
            .as_ref()
            .create((AgentIdType::PREFIX, id.uuid().to_string()))
            .content(db_agent)
            .await
            .map_err(|e| DatabaseError::QueryFailed(e))?;

        created
            .map(|db| {
                db.try_into()
                    .map_err(|_| DatabaseError::Other("Failed to parse agent ID".into()))
            })
            .ok_or_else(|| DatabaseError::Other("Failed to create agent".into()))?
    }

    /// Get agents for a user
    async fn get_user_agents(&self, user_id: UserId) -> Result<Vec<Agent>> {
        let query = format!(
            "SELECT * FROM {} WHERE user_id = {}:`{}` AND is_active = true",
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
        agent_id: AgentId,
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

        // Create DB wrapper type directly
        let db_memory = DbMemoryBlock {
            id: RecordId::from(id),
            agent_id: agent_id.into(),
            label,
            content,
            description,
            embedding,
            embedding_model,
            metadata,
            created_at: now.into(),
            updated_at: now.into(),
        };

        let created: Option<DbMemoryBlock> = self
            .as_ref()
            .create((MemoryIdType::PREFIX, id.uuid().to_string()))
            .content(db_memory)
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
                vector::similarity::cosine(embedding, $embedding) as score
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
            .bind(("embedding", query_embedding.vector))
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

    /// Get memory by label
    /// Get memory by label for an agent
    async fn get_memory_by_label(
        &self,
        agent_id: AgentId,
        label: &str,
    ) -> Result<Option<MemoryBlock>> {
        let query = format!(
            "SELECT * FROM {} WHERE agent_id = {}:`{}` AND label = $label AND is_active = true LIMIT 1",
            MemoryIdType::PREFIX,
            AgentIdType::PREFIX,
            agent_id.uuid()
        );

        let mut result = self
            .as_ref()
            .query(&query)
            .bind(("label", label.to_string()))
            .await
            .map_err(|e| DatabaseError::QueryFailed(e))?;

        let db_memories: Vec<DbMemoryBlock> =
            result.take(0).map_err(|e| DatabaseError::QueryFailed(e))?;

        // Convert the first memory if found
        Ok(db_memories.into_iter().next().and_then(|db| {
            // We need to reconstruct the RecordId from the query result
            db.try_into().ok()
        }))
    }

    /// Get user with agents in one query
    /// Get user with their agents
    async fn get_user_with_agents(&self, user_id: UserId) -> Result<Option<(User, Vec<Agent>)>> {
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::{AgentState, AgentType};
    use crate::db::client;
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

        // Verify the ID is properly typed
        assert!(user.id.to_string().starts_with("user-"));

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

        // Verify the agent ID is properly typed
        assert!(agent.id.to_string().starts_with("agent-"));
        assert_eq!(agent.user_id, user.id);
        assert_eq!(agent.agent_type, AgentType::Generic);
        assert_eq!(agent.state, AgentState::Ready);

        // Get agents for the user
        let agents = db.get_user_agents(user_id).await.unwrap();
        assert_eq!(agents.len(), 1);
        assert_eq!(agents[0].id, agent.id);

        // Update agent state
        db.update_agent_state(agent.id, AgentState::Processing)
            .await
            .unwrap();

        // Create a memory block
        let memory = db
            .create_memory(
                None, // No embedding provider for test
                agent.id,
                "test_memory".to_string(),
                "Test content".to_string(),
                Some("Test description".to_string()),
                serde_json::json!({"test": true}),
            )
            .await
            .unwrap();

        // Verify memory ID is properly typed
        assert!(memory.id.to_string().starts_with("mem-"));
        assert_eq!(memory.agent_id, agent.id);

        // Get memory by label
        let retrieved_memory = db
            .get_memory_by_label(agent.id, "test_memory")
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
}
