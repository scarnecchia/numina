//! Database operations - direct, simple, no unnecessary abstractions

use super::{DatabaseError, Result, VectorStore, schema::*, serde_helpers::unwrap_surreal_value};
use crate::embeddings::EmbeddingProvider;
use crate::id::{AgentId, MemoryId, UserId};

// ===== User Operations =====

/// Create a new user
pub async fn create_user<DB: VectorStore>(
    db: &DB,
    id: Option<UserId>,
    settings: serde_json::Value,
    metadata: serde_json::Value,
) -> Result<User> {
    let id = id.unwrap_or_else(|| UserId::generate());

    let query = format!(
        r#"
        CREATE users:`{}` CONTENT {{
            created_at: time::now(),
            updated_at: time::now(),
            settings: $settings,
            metadata: $metadata
        }}
    "#,
        id.to_string()
    );

    let response = db
        .execute(
            &query,
            vec![
                ("settings".to_string(), settings),
                ("metadata".to_string(), metadata),
            ],
        )
        .await?;

    // Debug: Print what we got back
    response
        .data
        .as_array()
        .and_then(|arr| arr.first())
        .ok_or_else(|| DatabaseError::QueryFailed("Failed to create user".into()))
        .and_then(|data| {
            let unwrapped = unwrap_surreal_value(data);
            serde_json::from_value(unwrapped).map_err(|e| DatabaseError::QueryFailed(Box::new(e)))
        })
}

/// Get a user by ID
pub async fn get_user<DB: VectorStore>(db: &DB, id: UserId) -> Result<Option<User>> {
    let response = db
        .execute(
            &format!("SELECT * FROM users:`{}` LIMIT 1", id.to_string()),
            vec![],
        )
        .await?;

    if let Some(data) = response.data.as_array().and_then(|arr| arr.first()) {
        let unwrapped = unwrap_surreal_value(data);
        Ok(Some(
            serde_json::from_value(unwrapped)
                .map_err(|e| DatabaseError::QueryFailed(Box::new(e)))?,
        ))
    } else {
        Ok(None)
    }
}

// ===== Agent Operations =====

/// Create a new agent
pub async fn create_agent<DB: VectorStore>(
    db: &DB,
    user_id: UserId,
    agent_type: crate::agent::AgentType,
    name: String,
    system_prompt: String,
    config: serde_json::Value,
    state: crate::agent::AgentState,
) -> Result<Agent> {
    let id = AgentId::generate();

    let query = format!(
        r#"
        CREATE agents:`{}` CONTENT {{
            user_id: users:`{}`,
            agent_type: $agent_type,
            name: $name,
            system_prompt: $system_prompt,
            config: $config,
            state: $state,
            created_at: time::now(),
            updated_at: time::now(),
            is_active: true
        }}
    "#,
        id.to_string(),
        user_id.to_string()
    );

    let response = db
        .execute(
            &query,
            vec![
                ("agent_type".to_string(), serde_json::json!(agent_type)),
                ("name".to_string(), serde_json::json!(name)),
                (
                    "system_prompt".to_string(),
                    serde_json::json!(system_prompt),
                ),
                ("config".to_string(), config),
                ("state".to_string(), serde_json::json!(state)),
            ],
        )
        .await?;

    response
        .data
        .as_array()
        .and_then(|arr| arr.first())
        .ok_or_else(|| DatabaseError::QueryFailed("Failed to create agent".into()))
        .and_then(|data| {
            let unwrapped = unwrap_surreal_value(data);
            serde_json::from_value(unwrapped).map_err(|e| DatabaseError::QueryFailed(Box::new(e)))
        })
}

/// Get agents for a user
pub async fn get_user_agents<DB: VectorStore>(db: &DB, user_id: UserId) -> Result<Vec<Agent>> {
    let response = db
        .execute(
            &format!("SELECT * FROM agents WHERE user_id = users:`{}` AND is_active = true ORDER BY created_at", user_id.to_string()),
            vec![],
        )
        .await?;

    let agents = response
        .data
        .as_array()
        .ok_or_else(|| DatabaseError::QueryFailed("Expected array response".into()))?
        .iter()
        .filter_map(|v| {
            let unwrapped = unwrap_surreal_value(v);
            serde_json::from_value(unwrapped).ok()
        })
        .collect();

    Ok(agents)
}

/// Update agent state
pub async fn update_agent_state<DB: VectorStore>(
    db: &DB,
    id: AgentId,
    state: crate::agent::AgentState,
) -> Result<()> {
    db.execute(
        &format!(
            "UPDATE agents:`{}` SET state = $state, updated_at = time::now()",
            id.to_string()
        ),
        vec![("state".to_string(), serde_json::json!(state))],
    )
    .await?;

    Ok(())
}

// ===== Memory Operations =====

/// Create a memory block with optional embedding
pub async fn create_memory<DB: VectorStore>(
    db: &DB,
    embeddings: Option<&dyn EmbeddingProvider>,
    agent_id: AgentId,
    label: String,
    content: String,
    description: Option<String>,
    metadata: serde_json::Value,
) -> Result<MemoryBlock> {
    let id = MemoryId::generate();

    // Generate embedding if provider is available
    let (embedding, embedding_model) = if let Some(provider) = embeddings {
        let emb = provider
            .embed(&content)
            .await
            .map_err(|e| DatabaseError::QueryFailed(Box::new(e)))?;
        (emb.vector, emb.model)
    } else {
        // Create dummy embedding with expected dimensions (384 for vector index)
        (vec![0.0; 384], "none".to_string())
    };

    let query = format!(
        r#"
        CREATE memory_blocks:`{}` CONTENT {{
            agent_id: agents:`{}`,
            label: $label,
            content: $content,
            description: $description,
            embedding: $embedding,
            embedding_model: $embedding_model,
            metadata: $metadata,
            created_at: time::now(),
            updated_at: time::now(),
            is_active: true
        }}
    "#,
        id.to_string(),
        agent_id.to_string()
    );

    let response = db
        .execute(
            &query,
            vec![
                ("label".to_string(), serde_json::json!(label)),
                ("content".to_string(), serde_json::json!(content)),
                ("description".to_string(), serde_json::json!(description)),
                ("embedding".to_string(), serde_json::json!(embedding)),
                (
                    "embedding_model".to_string(),
                    serde_json::json!(embedding_model),
                ),
                ("metadata".to_string(), metadata),
            ],
        )
        .await?;

    response
        .data
        .as_array()
        .and_then(|arr| arr.first())
        .ok_or_else(|| DatabaseError::QueryFailed("Failed to create memory".into()))
        .and_then(|data| {
            let unwrapped = unwrap_surreal_value(data);
            serde_json::from_value(unwrapped).map_err(|e| DatabaseError::QueryFailed(Box::new(e)))
        })
}

/// Search memories by semantic similarity
pub async fn search_memories<DB: VectorStore>(
    db: &DB,
    embeddings: &dyn EmbeddingProvider,
    agent_id: AgentId,
    query: &str,
    limit: usize,
) -> Result<Vec<(MemoryBlock, f32)>> {
    // Generate embedding for query
    let query_embedding = embeddings
        .embed(query)
        .await
        .map_err(|e| DatabaseError::QueryFailed(Box::new(e)))?;

    // Create filter for agent_id
    let filter = super::SearchFilter {
        where_clause: Some(format!(
            "agent_id = agents:`{}` AND is_active = true",
            agent_id.to_string()
        )),
        params: vec![],
    };

    // Perform vector search
    let results = db
        .vector_search(
            "memory_blocks",
            "embedding",
            &query_embedding.vector,
            limit,
            Some(filter),
        )
        .await?;

    // Convert to memory blocks with scores
    results
        .into_iter()
        .map(|result| {
            let unwrapped = unwrap_surreal_value(&result.data);
            let memory: MemoryBlock = serde_json::from_value(unwrapped)
                .map_err(|e| DatabaseError::QueryFailed(Box::new(e)))?;
            Ok((memory, result.score))
        })
        .collect()
}

/// Get memory by label
pub async fn get_memory_by_label<DB: VectorStore>(
    db: &DB,
    agent_id: AgentId,
    label: &str,
) -> Result<Option<MemoryBlock>> {
    let response = db
        .execute(
            &format!("SELECT * FROM memory_blocks WHERE agent_id = agents:`{}` AND label = $label AND is_active = true LIMIT 1", agent_id.to_string()),
            vec![
                ("label".to_string(), serde_json::json!(label)),
            ],
        )
        .await?;

    if let Some(data) = response.data.as_array().and_then(|arr| arr.first()) {
        let unwrapped = unwrap_surreal_value(data);
        Ok(Some(
            serde_json::from_value(unwrapped)
                .map_err(|e| DatabaseError::QueryFailed(Box::new(e)))?,
        ))
    } else {
        Ok(None)
    }
}

/// Get user with agents in one query
pub async fn get_user_with_agents<DB: VectorStore>(
    db: &DB,
    id: UserId,
) -> Result<Option<(User, Vec<Agent>)>> {
    let query = format!(
        r#"
        SELECT *,
            (SELECT * FROM agents WHERE user_id = users:`{}` AND is_active = true) as agents
        FROM users:`{}`
        LIMIT 1
    "#,
        id.to_string(),
        id.to_string()
    );

    let response = db.execute(&query, vec![]).await?;

    if let Some(data) = response.data.as_array().and_then(|arr| arr.first()) {
        let unwrapped = unwrap_surreal_value(data);
        let user: User = serde_json::from_value(unwrapped.clone())
            .map_err(|e| DatabaseError::QueryFailed(Box::new(e)))?;

        let agents = unwrapped
            .get("agents")
            .and_then(|a| a.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| {
                        let agent_unwrapped = unwrap_surreal_value(v);
                        serde_json::from_value(agent_unwrapped).ok()
                    })
                    .collect()
            })
            .unwrap_or_default();

        Ok(Some((user, agents)))
    } else {
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::{AgentState, AgentType};
    use crate::db::{DatabaseBackend, DatabaseConfig, embedded::EmbeddedDatabase};
    use tempfile::tempdir;

    #[tokio::test]
    async fn test_typed_ids_in_database() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test.db").to_string_lossy().to_string();

        let config = DatabaseConfig::Embedded {
            path,
            strict_mode: false,
        };

        let db = EmbeddedDatabase::connect(config).await.unwrap();

        // Run migrations
        crate::db::migration::MigrationRunner::run_migrations(&db)
            .await
            .unwrap();

        // Create a user with typed ID
        let user = create_user(
            &*db,
            None,
            serde_json::json!({"theme": "dark"}),
            serde_json::json!({"source": "test"}),
        )
        .await
        .unwrap();

        // Verify the ID is properly typed
        assert!(user.id.to_string().starts_with("user-"));

        // Get the user back
        let retrieved_user = get_user(&*db, user.id).await.unwrap().unwrap();
        assert_eq!(retrieved_user.id, user.id);

        // Create an agent with typed IDs
        let agent = create_agent(
            &*db,
            user.id,
            AgentType::Generic,
            "Test Agent".to_string(),
            "You are a test agent".to_string(),
            serde_json::json!({"model": "test"}),
            AgentState::Ready,
        )
        .await
        .unwrap();

        // Verify the agent ID is properly typed
        assert!(agent.id.to_string().starts_with("agent-"));
        assert_eq!(agent.user_id, user.id);
        assert_eq!(agent.agent_type, AgentType::Generic);
        assert_eq!(agent.state, AgentState::Ready);

        // Get agents for user
        let agents = get_user_agents(&*db, user.id).await.unwrap();
        assert_eq!(agents.len(), 1);
        assert_eq!(agents[0].id, agent.id);

        // Update agent state
        update_agent_state(&*db, agent.id, AgentState::Processing)
            .await
            .unwrap();

        // Create a memory block
        let memory = create_memory(
            &*db,
            None,
            agent.id,
            "test_memory".to_string(),
            "Test content".to_string(),
            Some("Test description".to_string()),
            serde_json::json!({}),
        )
        .await
        .unwrap();

        // Verify memory ID is properly typed
        assert!(memory.id.to_string().starts_with("mem-"));
        assert_eq!(memory.agent_id, agent.id);

        // Get memory by label
        let retrieved_memory = get_memory_by_label(&*db, agent.id, "test_memory")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(retrieved_memory.id, memory.id);

        // Get user with agents
        let (user_with_agents, agents) =
            get_user_with_agents(&*db, user.id).await.unwrap().unwrap();
        assert_eq!(user_with_agents.id, user.id);
        assert_eq!(agents.len(), 1);
        assert_eq!(agents[0].id, agent.id);
    }

    #[tokio::test]
    async fn test_agent_type_serialization() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test.db").to_string_lossy().to_string();

        let config = DatabaseConfig::Embedded {
            path,
            strict_mode: false,
        };

        let db = EmbeddedDatabase::connect(config).await.unwrap();
        crate::db::migration::MigrationRunner::run_migrations(&db)
            .await
            .unwrap();

        let user = create_user(&*db, None, serde_json::json!({}), serde_json::json!({}))
            .await
            .unwrap();

        // Test custom agent type
        let _custom_agent = create_agent(
            &*db,
            user.id,
            AgentType::Custom("special".to_string()),
            "Special Agent".to_string(),
            "You are special".to_string(),
            serde_json::json!({}),
            AgentState::Ready,
        )
        .await
        .unwrap();

        // Retrieve and verify
        let agents = get_user_agents(&*db, user.id).await.unwrap();
        assert_eq!(agents.len(), 1);
        match &agents[0].agent_type {
            AgentType::Custom(name) => assert_eq!(name, "special"),
            _ => panic!("Expected custom agent type"),
        }
    }
}
