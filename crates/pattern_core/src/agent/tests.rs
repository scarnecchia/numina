//! Tests for the agent system

#[cfg(test)]
mod tests {
    use surrealdb::RecordId;
    use tracing_test::traced_test;
    use uuid::Uuid;

    use super::super::*;
    #[allow(unused_imports)]
    use crate::utils::debug::ResponseDebug;
    use crate::{
        db::{DbMemoryBlock, client, models::strip_brackets, ops::SurrealExt},
        embeddings::MockEmbeddingProvider,
        model::MockModelProvider,
        tool::ToolRegistry,
    };
    use std::sync::Arc;
    use tokio::sync::RwLock;

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[traced_test]
    async fn test_shared_memory_between_agents() {
        // Create a test database
        let db = Arc::new(client::create_test_db().await.unwrap());

        // Create a mock model provider
        let model = Arc::new(RwLock::new(MockModelProvider {
            response: "Test response".to_string(),
        }));

        // Create empty tool registry
        let tools = ToolRegistry::new();

        // Create a test user
        let user_id = crate::id::UserId::generate();
        let user = db
            .create_user(Some(user_id), serde_json::json!({}), serde_json::json!({}))
            .await
            .unwrap();

        // Create two agents for the user
        let pattern_agent = db
            .create_agent(
                user.id,
                #[cfg(feature = "nd")]
                AgentType::Pattern,
                #[cfg(not(feature = "nd"))]
                AgentType::Generic,
                "Pattern".to_string(),
                "I am Pattern, the orchestrator".to_string(),
                serde_json::json!({}),
                AgentState::Ready,
            )
            .await
            .unwrap();

        let entropy_agent = db
            .create_agent(
                user.id,
                #[cfg(feature = "nd")]
                AgentType::Entropy,
                #[cfg(not(feature = "nd"))]
                AgentType::Custom("Entropy".to_string()),
                "Entropy".to_string(),
                "I am Entropy, the task specialist".to_string(),
                serde_json::json!({}),
                AgentState::Ready,
            )
            .await
            .unwrap();

        // Create a memory block owned by the user
        let memory = db
            .create_memory(
                None, // No embeddings for this test
                user.id,
                "task_insights".to_string(),
                "User struggles with large tasks".to_string(),
                Some("Insights about task management".to_string()),
                serde_json::json!({}),
            )
            .await
            .unwrap();

        let pattern_id = AgentId::from_uuid(
            Uuid::from_str(strip_brackets(&pattern_agent.id.key().to_string())).unwrap(),
        );
        let entropy_id = AgentId::from_uuid(
            Uuid::from_str(strip_brackets(&entropy_agent.id.key().to_string())).unwrap(),
        );

        // Attach memory to both agents with different access levels
        db.attach_memory_to_agent(pattern_id, memory.id, MemoryAccessLevel::Admin)
            .await
            .unwrap();

        db.attach_memory_to_agent(entropy_id, memory.id, MemoryAccessLevel::Write)
            .await
            .unwrap();

        // Create agent instances
        let pattern = Arc::new(
            DatabaseAgent::new(
                pattern_id,
                db.clone(),
                model.clone(),
                None::<Arc<MockEmbeddingProvider>>,
                tools.clone(),
            )
            .await
            .unwrap(),
        );

        let entropy = Arc::new(
            DatabaseAgent::new(
                entropy_id,
                db.clone(),
                model.clone(),
                None::<Arc<MockEmbeddingProvider>>,
                tools.clone(),
            )
            .await
            .unwrap(),
        );

        // Start memory sync for both agents
        pattern.clone().start_memory_sync().await.unwrap();
        entropy.clone().start_memory_sync().await.unwrap();

        // Give the background tasks time to start
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

        // Both agents should be able to read the memory
        let _pattern_memories = db.get_agent_memories(pattern_id).await.unwrap();

        let pattern_memory = pattern.get_memory("task_insights").await.unwrap();
        assert!(pattern_memory.is_some());
        let pattern_block = pattern_memory.unwrap();
        assert_eq!(pattern_block.content, "User struggles with large tasks");

        let entropy_memory = entropy.get_memory("task_insights").await.unwrap();
        assert!(entropy_memory.is_some());
        let entropy_block = entropy_memory.unwrap();
        assert_eq!(entropy_block.content, "User struggles with large tasks");

        // Entropy updates the memory
        let mut updated_memory = memory;
        updated_memory.content =
            "User struggles with large tasks - break them into smaller pieces".to_string();
        entropy
            .update_memory("task_insights", updated_memory.clone())
            .await
            .unwrap();

        // Give the live query time to propagate the update
        tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;

        // Pattern should see the update through the live query cache update
        let pattern_updated = pattern.get_memory("task_insights").await.unwrap();
        let pattern_updated_block = pattern_updated.unwrap();
        assert_eq!(
            pattern_updated_block.content,
            "User struggles with large tasks - break them into smaller pieces"
        );

        // Test cache deletion through live queries
        // Create a new memory and attach it to Pattern
        let deletable_memory = db
            .create_memory(
                None,
                user.id,
                "temporary_note".to_string(),
                "This will be deleted".to_string(),
                None,
                serde_json::json!({}),
            )
            .await
            .unwrap();

        db.attach_memory_to_agent(
            pattern_agent.id,
            deletable_memory.id,
            MemoryAccessLevel::Read,
        )
        .await
        .unwrap();

        // Debug: Check if the memory has the agent in its agents array using pretty debug
        // let check_query = format!(
        //     "SELECT agents AS agents FROM {} FETCH agents",
        //     RecordId::from(deletable_memory.id)
        // );
        // let mut check_result = db.as_ref().query(&check_query).await.unwrap();
        // tracing::debug!("Response: {:?}", check_result.pretty_debug());
        // let agents_arrays: Vec<Vec<DbAgent>> =
        //     check_result.take("agents").expect("should have agents");
        // tracing::debug!(
        //     "After attach, temporary_note agents array: {:?}",
        //     agents_arrays.concat()
        // );

        // Wait for live query to pick it up
        tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;

        // Verify Pattern can see it
        let temp_memory = pattern.get_memory("temporary_note").await.unwrap();
        assert!(temp_memory.is_some());

        // Delete the memory block
        let _resp: Option<DbMemoryBlock> = db
            .as_ref()
            .delete(RecordId::from(deletable_memory.id))
            .await
            .unwrap();

        // Wait for deletion to propagate
        tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;

        // Pattern should no longer see the deleted memory
        let deleted_memory = pattern.get_memory("temporary_note").await.unwrap();
        assert!(deleted_memory.is_none());
    }

    #[tokio::test]
    #[traced_test]
    async fn test_memory_access_levels() {
        let db = Arc::new(client::create_test_db().await.unwrap());

        // Create test data
        let user_id = crate::id::UserId::generate();
        let user = db
            .create_user(Some(user_id), serde_json::json!({}), serde_json::json!({}))
            .await
            .unwrap();

        let agent = db
            .create_agent(
                user.id,
                AgentType::Generic,
                "TestAgent".to_string(),
                "Test agent".to_string(),
                serde_json::json!({}),
                AgentState::Ready,
            )
            .await
            .unwrap();

        let memory = db
            .create_memory(
                None,
                user.id,
                "test_memory".to_string(),
                "Test content".to_string(),
                None,
                serde_json::json!({}),
            )
            .await
            .unwrap();

        // Test different access levels
        db.attach_memory_to_agent(agent.id, memory.id, MemoryAccessLevel::Read)
            .await
            .unwrap();

        let memories = db.get_agent_memories(agent.id).await.unwrap();
        assert_eq!(memories.len(), 1);
        assert_eq!(memories[0].1, MemoryAccessLevel::Read);
        assert_eq!(memories[0].0.label, "test_memory");
    }

    #[tokio::test]
    #[traced_test]
    async fn test_agent_state_transitions() {
        let db = Arc::new(client::create_test_db().await.unwrap());
        let model = Arc::new(RwLock::new(MockModelProvider {
            response: "Test".to_string(),
        }));
        let tools = ToolRegistry::new();

        let user_id = crate::id::UserId::generate();
        let user = db
            .create_user(Some(user_id), serde_json::json!({}), serde_json::json!({}))
            .await
            .unwrap();

        let agent_record = db
            .create_agent(
                user.id,
                AgentType::Generic,
                "Test".to_string(),
                "Test agent".to_string(),
                serde_json::json!({}),
                AgentState::Ready,
            )
            .await
            .unwrap();

        let mut agent = DatabaseAgent::new(
            agent_record.id,
            db.clone(),
            model,
            None::<Arc<MockEmbeddingProvider>>,
            tools,
        )
        .await
        .unwrap();

        // Check initial state
        assert_eq!(agent.state(), AgentState::Ready);

        // Update state
        agent.set_state(AgentState::Suspended).await.unwrap();
        assert_eq!(agent.state(), AgentState::Suspended);

        // Verify in database
        let agents = db.get_user_agents(user.id).await.unwrap();
        assert_eq!(agents[0].state, AgentState::Suspended);
    }
}
