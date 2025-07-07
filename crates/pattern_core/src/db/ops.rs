//! Database operations - direct, simple, no unnecessary abstractions

use super::{DatabaseError, Result, VectorStore, schema::*};
use crate::embeddings::EmbeddingProvider;
use std::sync::Arc;

/// Generate a unique ID with a prefix
pub fn generate_id(prefix: &str) -> String {
    format!("{}:{}", prefix, uuid::Uuid::new_v4())
}

/// Get current UTC timestamp
pub fn now() -> chrono::DateTime<chrono::Utc> {
    chrono::Utc::now()
}

// ===== User Operations =====

/// Create a new user
pub async fn create_user<DB: VectorStore>(
    db: &DB,
    id: Option<String>,
    settings: serde_json::Value,
    metadata: serde_json::Value,
) -> Result<User> {
    let id = id.unwrap_or_else(|| generate_id("user"));
    let now = now();

    let query = r#"
        CREATE users CONTENT {
            id: $id,
            created_at: $created_at,
            updated_at: $updated_at,
            settings: $settings,
            metadata: $metadata
        }
    "#;

    let response = db
        .execute(
            query,
            vec![
                ("id".to_string(), serde_json::json!(id)),
                ("created_at".to_string(), serde_json::json!(now)),
                ("updated_at".to_string(), serde_json::json!(now)),
                ("settings".to_string(), settings),
                ("metadata".to_string(), metadata),
            ],
        )
        .await?;

    response
        .data
        .as_array()
        .and_then(|arr| arr.first())
        .ok_or_else(|| DatabaseError::QueryFailed("Failed to create user".into()))
        .and_then(|data| {
            serde_json::from_value(data.clone())
                .map_err(|e| DatabaseError::QueryFailed(Box::new(e)))
        })
}

/// Get a user by ID
pub async fn get_user<DB: VectorStore>(db: &DB, id: &str) -> Result<Option<User>> {
    let response = db
        .execute(
            "SELECT * FROM users WHERE id = $id LIMIT 1",
            vec![("id".to_string(), serde_json::json!(id))],
        )
        .await?;

    if let Some(data) = response.data.as_array().and_then(|arr| arr.first()) {
        Ok(Some(
            serde_json::from_value(data.clone())
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
    user_id: String,
    agent_type: String,
    name: String,
    system_prompt: String,
    config: serde_json::Value,
    state: serde_json::Value,
) -> Result<Agent> {
    let id = generate_id("agent");
    let now = now();

    let query = r#"
        CREATE agents CONTENT {
            id: $id,
            user_id: $user_id,
            agent_type: $agent_type,
            name: $name,
            system_prompt: $system_prompt,
            config: $config,
            state: $state,
            created_at: $created_at,
            updated_at: $updated_at,
            is_active: true
        }
    "#;

    let response = db
        .execute(
            query,
            vec![
                ("id".to_string(), serde_json::json!(id)),
                ("user_id".to_string(), serde_json::json!(user_id)),
                ("agent_type".to_string(), serde_json::json!(agent_type)),
                ("name".to_string(), serde_json::json!(name)),
                (
                    "system_prompt".to_string(),
                    serde_json::json!(system_prompt),
                ),
                ("config".to_string(), config),
                ("state".to_string(), state),
                ("created_at".to_string(), serde_json::json!(now)),
                ("updated_at".to_string(), serde_json::json!(now)),
            ],
        )
        .await?;

    response
        .data
        .as_array()
        .and_then(|arr| arr.first())
        .ok_or_else(|| DatabaseError::QueryFailed("Failed to create agent".into()))
        .and_then(|data| {
            serde_json::from_value(data.clone())
                .map_err(|e| DatabaseError::QueryFailed(Box::new(e)))
        })
}

/// Get agents for a user
pub async fn get_user_agents<DB: VectorStore>(db: &DB, user_id: &str) -> Result<Vec<Agent>> {
    let response = db
        .execute(
            "SELECT * FROM agents WHERE user_id = $user_id AND is_active = true ORDER BY created_at",
            vec![("user_id".to_string(), serde_json::json!(user_id))],
        )
        .await?;

    let agents = response
        .data
        .as_array()
        .ok_or_else(|| DatabaseError::QueryFailed("Expected array response".into()))?
        .iter()
        .filter_map(|v| serde_json::from_value(v.clone()).ok())
        .collect();

    Ok(agents)
}

/// Update agent state
pub async fn update_agent_state<DB: VectorStore>(
    db: &DB,
    id: &str,
    state: serde_json::Value,
) -> Result<()> {
    db.execute(
        "UPDATE agents SET state = $state, updated_at = time::now() WHERE id = $id",
        vec![
            ("id".to_string(), serde_json::json!(id)),
            ("state".to_string(), state),
        ],
    )
    .await?;

    Ok(())
}

// ===== Memory Operations =====

/// Create a memory block with optional embedding
pub async fn create_memory<DB: VectorStore>(
    db: &DB,
    embeddings: Option<&dyn EmbeddingProvider>,
    agent_id: String,
    label: String,
    content: String,
    description: Option<String>,
    metadata: serde_json::Value,
) -> Result<MemoryBlock> {
    let id = generate_id("memory");
    let now = now();

    // Generate embedding if provider is available
    let (embedding, embedding_model) = if let Some(provider) = embeddings {
        let emb = provider
            .embed(&content)
            .await
            .map_err(|e| DatabaseError::QueryFailed(Box::new(e)))?;
        (emb.vector, emb.model)
    } else {
        (vec![], "none".to_string())
    };

    let query = r#"
        CREATE memory_blocks CONTENT {
            id: $id,
            agent_id: $agent_id,
            label: $label,
            content: $content,
            description: $description,
            embedding: $embedding,
            embedding_model: $embedding_model,
            metadata: $metadata,
            created_at: $created_at,
            updated_at: $updated_at,
            is_active: true
        }
    "#;

    let response = db
        .execute(
            query,
            vec![
                ("id".to_string(), serde_json::json!(id)),
                ("agent_id".to_string(), serde_json::json!(agent_id)),
                ("label".to_string(), serde_json::json!(label)),
                ("content".to_string(), serde_json::json!(content)),
                ("description".to_string(), serde_json::json!(description)),
                ("embedding".to_string(), serde_json::json!(embedding)),
                (
                    "embedding_model".to_string(),
                    serde_json::json!(embedding_model),
                ),
                ("metadata".to_string(), metadata),
                ("created_at".to_string(), serde_json::json!(now)),
                ("updated_at".to_string(), serde_json::json!(now)),
            ],
        )
        .await?;

    response
        .data
        .as_array()
        .and_then(|arr| arr.first())
        .ok_or_else(|| DatabaseError::QueryFailed("Failed to create memory".into()))
        .and_then(|data| {
            serde_json::from_value(data.clone())
                .map_err(|e| DatabaseError::QueryFailed(Box::new(e)))
        })
}

/// Search memories by semantic similarity
pub async fn search_memories<DB: VectorStore>(
    db: &DB,
    embeddings: &dyn EmbeddingProvider,
    agent_id: &str,
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
        where_clause: Some("agent_id = $agent_id AND is_active = true".to_string()),
        params: vec![("agent_id".to_string(), serde_json::json!(agent_id))],
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
            let memory: MemoryBlock = serde_json::from_value(result.data)
                .map_err(|e| DatabaseError::QueryFailed(Box::new(e)))?;
            Ok((memory, result.score))
        })
        .collect()
}

/// Get memory by label
pub async fn get_memory_by_label<DB: VectorStore>(
    db: &DB,
    agent_id: &str,
    label: &str,
) -> Result<Option<MemoryBlock>> {
    let response = db
        .execute(
            "SELECT * FROM memory_blocks WHERE agent_id = $agent_id AND label = $label AND is_active = true LIMIT 1",
            vec![
                ("agent_id".to_string(), serde_json::json!(agent_id)),
                ("label".to_string(), serde_json::json!(label)),
            ],
        )
        .await?;

    if let Some(data) = response.data.as_array().and_then(|arr| arr.first()) {
        Ok(Some(
            serde_json::from_value(data.clone())
                .map_err(|e| DatabaseError::QueryFailed(Box::new(e)))?,
        ))
    } else {
        Ok(None)
    }
}

/// Get user with agents in one query
pub async fn get_user_with_agents<DB: VectorStore>(
    db: &DB,
    id: &str,
) -> Result<Option<(User, Vec<Agent>)>> {
    let query = r#"
        SELECT *,
            (SELECT * FROM agents WHERE user_id = $id AND is_active = true) as agents
        FROM users
        WHERE id = $id
        LIMIT 1
    "#;

    let response = db
        .execute(query, vec![("id".to_string(), serde_json::json!(id))])
        .await?;

    if let Some(data) = response.data.as_array().and_then(|arr| arr.first()) {
        let user: User = serde_json::from_value(data.clone())
            .map_err(|e| DatabaseError::QueryFailed(Box::new(e)))?;

        let agents = data
            .get("agents")
            .and_then(|a| a.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| serde_json::from_value(v.clone()).ok())
                    .collect()
            })
            .unwrap_or_default();

        Ok(Some((user, agents)))
    } else {
        Ok(None)
    }
}
