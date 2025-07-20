//! Integration tests for the entity system
//!
//! These tests verify that our proc macro-based entity system works correctly
//! with SurrealDB, including relations, skip fields, and custom storage types.

use chrono::{DateTime, Utc};
use pattern_core::{
    db::{
        DatabaseError,
        entity::{BaseAgent, BaseAgentType, DbEntity},
    },
    id::{AgentId, MemoryId, MemoryIdType, RelationId, RelationIdType, TaskId, TaskIdType, UserId},
};
use pattern_macros::Entity;
use serde::{Deserialize, Serialize};
use surrealdb::engine::local::{Db, Mem};
use surrealdb::{RecordId, Surreal};

/// Test entity with all field types
#[derive(Debug, Clone, Entity, Serialize, Deserialize)]
#[entity(entity_type = "user", crate_path = "::pattern_core")]
struct TestUser {
    pub id: UserId,
    pub username: String,
    pub email: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,

    // Skip field - computed at runtime
    #[entity(skip)]
    pub is_admin: bool,

    // ID-only relation fields (to avoid recursion)
    #[entity(relation = "owns")]
    pub owned_agent_ids: Vec<AgentId>,

    // Full entity relation fields
    #[entity(relation = "created")]
    pub created_tasks: Vec<TestTask>,
}

impl Default for TestUser {
    fn default() -> Self {
        let now = Utc::now();
        Self {
            id: UserId::generate(),
            username: String::new(),
            email: None,
            created_at: now,
            updated_at: now,
            is_admin: false,
            owned_agent_ids: Vec::new(),
            created_tasks: Vec::new(),
        }
    }
}

/// Test entity with custom storage types
#[derive(Debug, Clone, Entity, Serialize, Deserialize)]
#[entity(entity_type = "task", crate_path = "::pattern_core")]
struct TestTask {
    pub id: TaskId,
    pub title: String,

    // Store as flexible JSON
    pub metadata: serde_json::Value,

    // Store as comma-separated string
    #[entity(db_type = "String")]
    pub tags: Vec<String>,

    pub created_at: DateTime<Utc>,

    // Single ID relation (to avoid recursion with TestUser)
    #[entity(relation = "created_by")]
    pub creator_id: UserId,
}

/// Edge entity for agent-memory relationships with access levels
#[derive(Debug, Clone, Entity, Serialize, Deserialize)]
#[entity(entity_type = "agent_memories", crate_path = "::pattern_core")]
struct AgentMemoryRelation {
    pub id: RelationId,
    pub in_id: AgentId,
    pub out_id: MemoryId,
    pub access_level: String,
    pub created_at: DateTime<Utc>,
}
/// Simple memory entity for edge tests
#[derive(Debug, Clone, Entity, Serialize, Deserialize)]
#[entity(entity_type = "mem", crate_path = "::pattern_core")]
struct TestMemory {
    pub id: MemoryId,
    pub label: String,
    pub value: String,
    pub created_at: DateTime<Utc>,
}

/// Test agent with edge entity relation
#[derive(Debug, Clone, Entity, Serialize, Deserialize)]
#[entity(entity_type = "agent", crate_path = "::pattern_core")]
struct TestAgentWithEdge {
    pub id: AgentId,
    pub name: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,

    #[entity(relation = "agent_memories", edge_entity = "AgentMemoryRelation")]
    pub memories: Vec<(TestMemory, AgentMemoryRelation)>,
}

async fn setup_test_db() -> Result<Surreal<Db>, DatabaseError> {
    let db = Surreal::new::<Mem>(())
        .await
        .map_err(DatabaseError::ConnectionFailed)?;

    db.use_ns("test")
        .use_db("test")
        .await
        .map_err(DatabaseError::ConnectionFailed)?;

    // Create schemas
    let user_schema = TestUser::schema();
    db.query(&user_schema.schema)
        .await
        .map_err(DatabaseError::QueryFailed)?;

    let agent_schema = BaseAgent::schema();
    db.query(&agent_schema.schema)
        .await
        .map_err(DatabaseError::QueryFailed)?;

    let task_schema = TestTask::schema();
    db.query(&task_schema.schema)
        .await
        .map_err(DatabaseError::QueryFailed)?;

    Ok(db)
}

#[tokio::test]
async fn test_basic_entity_storage_and_retrieval() {
    let db = setup_test_db().await.expect("Failed to setup database");

    // Create and store a user
    let user = TestUser {
        username: "testuser".to_string(),
        email: Some("test@example.com".to_string()),
        ..Default::default()
    };

    // Store the user with relations
    let stored = user
        .store_with_relations(&db)
        .await
        .expect("Failed to store user");

    println!("converted: {:?}", stored);

    // Retrieve the user with relations
    let retrieved = TestUser::load_with_relations(&db, stored.id)
        .await
        .expect("Failed to load user")
        .expect("User not found");
    assert_eq!(retrieved.username, "testuser");
    assert_eq!(retrieved.email, Some("test@example.com".to_string()));
    assert!(!retrieved.is_admin); // Skip field should be default
}

#[tokio::test]
async fn test_relation_storage_and_loading() {
    let db = setup_test_db().await.expect("Failed to setup database");

    // Create a user
    let mut user = TestUser {
        username: "owner".to_string(),
        ..Default::default()
    };

    // Create some agents
    let agent1 = BaseAgent {
        name: "Agent 1".to_string(),
        agent_type: BaseAgentType::Assistant,
        ..Default::default()
    };
    let agent2 = BaseAgent {
        name: "Agent 2".to_string(),
        agent_type: BaseAgentType::Custom("custom".to_string()),
        ..Default::default()
    };

    // Store agents
    let stored_agent1_db = db
        .create("agent")
        .content(agent1.to_db_model())
        .await
        .expect("Failed to create agent1")
        .unwrap();
    let stored_agent1 =
        BaseAgent::from_db_model(stored_agent1_db).expect("Failed to convert agent1");

    let stored_agent2_db = db
        .create("agent")
        .content(agent2.to_db_model())
        .await
        .expect("Failed to create agent2")
        .unwrap();
    let stored_agent2 =
        BaseAgent::from_db_model(stored_agent2_db).expect("Failed to convert agent2");

    // Update user with owned agent IDs
    user.owned_agent_ids = vec![stored_agent1.id, stored_agent2.id];

    // Create some tasks that the user created
    let task1 = TestTask {
        id: TaskId::generate(),
        title: "Task 1".to_string(),
        metadata: serde_json::json!({"priority": "high"}),
        tags: vec!["todo".to_string()],
        created_at: Utc::now(),
        creator_id: user.id,
    };
    let task2 = TestTask {
        id: TaskId::generate(),
        title: "Task 2".to_string(),
        metadata: serde_json::json!({"priority": "low"}),
        tags: vec!["done".to_string()],
        created_at: Utc::now(),
        creator_id: user.id,
    };

    user.created_tasks = vec![task1.clone(), task2.clone()];

    // Store user with relations
    let stored_user = user
        .store_with_relations(&db)
        .await
        .expect("Failed to store user with relations");

    println!("stored_user: {:?}", stored_user);

    // Retrieve user with relations
    let retrieved = TestUser::load_with_relations(&db, stored_user.id)
        .await
        .expect("Failed to load user")
        .expect("User not found");

    println!("retrieved: {:?}", retrieved);

    // Check ID relations were loaded (just IDs)
    assert_eq!(retrieved.owned_agent_ids.len(), 2);
    assert!(retrieved.owned_agent_ids.contains(&stored_agent1.id));
    assert!(retrieved.owned_agent_ids.contains(&stored_agent2.id));

    // Check entity relations were loaded (full entities)
    assert_eq!(retrieved.created_tasks.len(), 2);
    let task_titles: Vec<&str> = retrieved
        .created_tasks
        .iter()
        .map(|t| t.title.as_str())
        .collect();
    assert!(task_titles.contains(&"Task 1"));
    assert!(task_titles.contains(&"Task 2"));

    // Check metadata is preserved
    let task1 = retrieved
        .created_tasks
        .iter()
        .find(|t| t.title == "Task 1")
        .unwrap();
    let task2 = retrieved
        .created_tasks
        .iter()
        .find(|t| t.title == "Task 2")
        .unwrap();
    assert_eq!(task1.metadata["priority"], "high");
    assert_eq!(task2.metadata["priority"], "low");
}

#[tokio::test]
async fn test_custom_storage_types() {
    let db = setup_test_db().await.expect("Failed to setup database");

    // Create a task with custom storage types
    let task = TestTask {
        id: TaskId::generate(),
        title: "Test Task".to_string(),
        metadata: serde_json::json!({
            "priority": "high",
            "category": "development",
            "custom_field": 42
        }),
        tags: vec![
            "rust".to_string(),
            "async".to_string(),
            "testing".to_string(),
        ],
        created_at: Utc::now(),
        creator_id: UserId::nil(),
    };

    // Store the task
    let stored_db = db
        .create("task")
        .content(task.to_db_model())
        .await
        .expect("Failed to create task")
        .unwrap();
    let stored = TestTask::from_db_model(stored_db).expect("Failed to convert task");

    // Retrieve the task
    let retrieved_db = db
        .select(("task", stored.id.to_record_id()))
        .await
        .expect("Failed to retrieve task");

    let retrieved = TestTask::from_db_model(retrieved_db.expect("Task not found"))
        .expect("Failed to convert task");

    // Check metadata (stored as flexible JSON)
    assert_eq!(retrieved.metadata["priority"], "high");
    assert_eq!(retrieved.metadata["custom_field"], 42);

    // Check tags (stored as comma-separated string, converted back to Vec)
    assert_eq!(retrieved.tags.len(), 3);
    assert_eq!(retrieved.tags, vec!["rust", "async", "testing"]);
}

#[tokio::test]
async fn test_skip_fields_not_stored() {
    let db = setup_test_db().await.expect("Failed to setup database");

    // Create a user with skip field set
    let user = TestUser {
        username: "admin".to_string(),
        is_admin: true, // This should not be stored
        ..Default::default()
    };

    // Store the user with relations
    let stored = user
        .store_with_relations(&db)
        .await
        .expect("Failed to store user");

    // Retrieve the user with relations
    let retrieved = TestUser::load_with_relations(&db, stored.id)
        .await
        .expect("Failed to load user")
        .expect("User not found");

    // Skip field should be default value, not the set value
    assert_eq!(retrieved.username, "admin");
    assert!(!retrieved.is_admin); // Should be false (default), not true
}

#[tokio::test]
async fn test_mixed_relation_types() {
    let db = setup_test_db().await.expect("Failed to setup database");

    // Create a user with both ID and entity relations
    let mut user = TestUser {
        username: "mixed_user".to_string(),
        ..Default::default()
    };

    // Store the user first
    let stored_user_db = db
        .create("user")
        .content(user.to_db_model())
        .await
        .expect("Failed to create user")
        .unwrap();
    let stored_user = TestUser::from_db_model(stored_user_db).expect("Failed to convert user");

    // Create and store an agent
    let agent = BaseAgent {
        name: "Test Agent".to_string(),
        agent_type: BaseAgentType::Assistant,
        ..Default::default()
    };
    let stored_agent_db = db
        .create("agent")
        .content(agent.to_db_model())
        .await
        .expect("Failed to create agent")
        .unwrap();
    let stored_agent = BaseAgent::from_db_model(stored_agent_db).expect("Failed to convert agent");

    // Create a task
    let task = TestTask {
        id: TaskId::generate(),
        title: "Important Task".to_string(),
        metadata: serde_json::json!({"deadline": "tomorrow"}),
        tags: vec!["urgent".to_string()],
        created_at: Utc::now(),
        creator_id: stored_user.id,
    };
    let stored_task_db = db
        .create("task")
        .content(task.to_db_model())
        .await
        .expect("Failed to create task")
        .unwrap();
    let stored_task = TestTask::from_db_model(stored_task_db).expect("Failed to convert task");

    // Update user with relations
    user.id = stored_user.id;
    user.owned_agent_ids = vec![stored_agent.id];
    user.created_tasks = vec![stored_task];

    // Store user with relations
    let stored = user
        .store_with_relations(&db)
        .await
        .expect("Failed to store user with relations");

    // Retrieve fresh user with relations
    let fresh_user = TestUser::load_with_relations(&db, stored.id)
        .await
        .expect("Failed to load user")
        .expect("User not found");

    // Now relations should be populated
    assert_eq!(fresh_user.owned_agent_ids.len(), 1);
    assert_eq!(fresh_user.owned_agent_ids[0], stored_agent.id);
    assert_eq!(fresh_user.created_tasks.len(), 1);
    assert_eq!(fresh_user.created_tasks[0].title, "Important Task");
}

#[tokio::test]
async fn test_schema_generation() {
    // Test that schema generation includes proper field definitions
    let user_schema = TestUser::schema();
    println!("User schema:\n{}", user_schema.schema);
    assert!(user_schema.schema.contains("DEFINE TABLE OVERWRITE user"));
    assert!(
        user_schema
            .schema
            .contains("DEFINE FIELD username ON TABLE user TYPE string")
    );
    assert!(
        user_schema
            .schema
            .contains("DEFINE FIELD email ON TABLE user TYPE option<string>")
    );
    assert!(
        user_schema
            .schema
            .contains("DEFINE FIELD created_at ON TABLE user TYPE datetime")
    );

    // Relation tables should be defined
    assert!(user_schema.schema.contains("DEFINE TABLE OVERWRITE owns"));
    assert!(
        user_schema
            .schema
            .contains("DEFINE TABLE OVERWRITE created")
    );

    let task_schema = TestTask::schema();
    // Check flexible field for serde_json::Value
    assert!(
        task_schema
            .schema
            .contains("DEFINE FIELD metadata ON TABLE task FLEXIBLE TYPE object")
    );
    // Check custom storage type
    assert!(
        task_schema
            .schema
            .contains("DEFINE FIELD tags ON TABLE task TYPE string")
    );
}

#[tokio::test]
async fn test_nil_relations() {
    let db = setup_test_db().await.expect("Failed to setup database");

    // Create a task with nil creator
    let task = TestTask {
        id: TaskId::generate(),
        title: "Orphan Task".to_string(),
        metadata: serde_json::json!({}),
        tags: vec![],
        created_at: Utc::now(),
        creator_id: UserId::nil(), // Nil relation
    };

    // Store the task with relations (should skip nil relations)
    let stored = task
        .store_with_relations(&db)
        .await
        .expect("Failed to store task");

    // Retrieve and load relations
    let retrieved = TestTask::load_with_relations(&db, stored.id)
        .await
        .expect("Failed to load task")
        .expect("Task not found");

    // Creator should still be nil
    assert!(retrieved.creator_id.is_nil());
}

#[tokio::test]
async fn test_edge_entity_relations() {
    let db = setup_test_db().await.expect("Failed to setup database");

    // Create schemas
    let agent_schema = TestAgentWithEdge::schema();
    db.query(&agent_schema.schema)
        .await
        .map_err(DatabaseError::QueryFailed)
        .expect("Failed to create agent schema");

    let memory_schema = TestMemory::schema();
    db.query(&memory_schema.schema)
        .await
        .map_err(DatabaseError::QueryFailed)
        .expect("Failed to create memory schema");

    let edge_schema = AgentMemoryRelation::schema();
    db.query(&edge_schema.schema)
        .await
        .map_err(DatabaseError::QueryFailed)
        .expect("Failed to create edge schema");

    // Create an agent
    let agent = TestAgentWithEdge {
        id: AgentId::generate(),
        name: "Test Agent".to_string(),
        created_at: Utc::now(),
        updated_at: Utc::now(),
        memories: vec![],
    };

    // Create some memories
    let memory1 = TestMemory {
        id: MemoryId::generate(),
        label: "persona".to_string(),
        value: "I am a helpful assistant".to_string(),
        created_at: Utc::now(),
    };

    let memory2 = TestMemory {
        id: MemoryId::generate(),
        label: "context".to_string(),
        value: "Current conversation context".to_string(),
        created_at: Utc::now(),
    };

    // Store memories first
    let stored_memory1 = memory1
        .store_with_relations(&db)
        .await
        .expect("Failed to store memory1");
    let stored_memory2 = memory2
        .store_with_relations(&db)
        .await
        .expect("Failed to store memory2");

    // Create edge entities for the memories
    let edge1 = AgentMemoryRelation {
        id: RelationId::generate(),
        in_id: agent.id,
        out_id: stored_memory1.id,
        access_level: "read".to_string(),
        created_at: Utc::now(),
    };

    let edge2 = AgentMemoryRelation {
        id: RelationId::generate(),
        in_id: agent.id,
        out_id: stored_memory2.id,
        access_level: "write".to_string(),
        created_at: Utc::now(),
    };

    // Update agent with memories and edge data
    let mut agent_with_memories = agent.clone();
    agent_with_memories.memories = vec![(stored_memory1, edge1), (stored_memory2, edge2)];

    // Store agent with edge entity relations
    let stored_agent = agent_with_memories
        .store_with_relations(&db)
        .await
        .expect("Failed to store agent with memories");

    println!("Stored agent: {:?}", stored_agent);

    // Load agent with edge entity relations
    let loaded_agent = TestAgentWithEdge::load_with_relations(&db, stored_agent.id)
        .await
        .expect("Failed to load agent")
        .expect("Agent not found");

    println!("Loaded agent: {:?}", loaded_agent);

    // Check that memories were loaded with edge data
    assert_eq!(loaded_agent.memories.len(), 2);

    // Check memory labels
    let memory_labels: Vec<&str> = loaded_agent
        .memories
        .iter()
        .map(|(m, _)| m.label.as_str())
        .collect();
    assert!(memory_labels.contains(&"persona"));
    assert!(memory_labels.contains(&"context"));

    // Check edge entity data (access levels)
    for (memory, edge) in &loaded_agent.memories {
        if memory.label == "persona" {
            assert_eq!(edge.access_level, "read");
        } else if memory.label == "context" {
            assert_eq!(edge.access_level, "write");
        }
    }

    // TODO: Once edge entity fields are fully implemented,
    // we should be able to query the edge table for access levels
}
