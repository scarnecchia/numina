use crate::error::{DatabaseError, Result};
use serde::{Deserialize, Serialize};
use sqlx::sqlite::SqlitePool;
use std::path::Path;
use tracing::{debug, info};

/// Database connection pool wrapper
#[derive(Clone, Debug)]
pub struct Database {
    pool: SqlitePool,
}

impl Database {
    /// Create a new database connection pool
    pub async fn new(db_path: &str) -> Result<Self> {
        // Create directory if it doesn't exist
        if let Some(parent) = Path::new(db_path).parent() {
            std::fs::create_dir_all(parent).map_err(|e| DatabaseError::ConnectionFailed {
                path: db_path.to_string(),
                source: sqlx::Error::Configuration(e.to_string().into()),
            })?;
        }

        info!("Connecting to database at: {}", db_path);

        let pool = SqlitePool::connect(&format!("sqlite://{}?mode=rwc", db_path))
            .await
            .map_err(|e| DatabaseError::ConnectionFailed {
                path: db_path.to_string(),
                source: e,
            })?;

        debug!("Database connection established");

        Ok(Self { pool })
    }

    /// Run database migrations
    pub async fn migrate(&self) -> Result<()> {
        info!("Running database migrations");

        sqlx::migrate!("./migrations")
            .run(&self.pool)
            .await
            .map_err(|e| DatabaseError::MigrationFailed { source: e })?;

        info!("Database migrations completed");
        Ok(())
    }

    /// Get a reference to the connection pool
    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }

    /// Get or create a user by username
    pub async fn get_or_create_user(&self, username: &str) -> Result<User> {
        // Try to get existing user
        let existing = sqlx::query_as::<_, User>(
            "SELECT id, name, discord_id, created_at FROM users WHERE name = ?1",
        )
        .bind(username)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| DatabaseError::QueryFailed {
            context: format!("Failed to fetch user '{}'", username),
            source: e,
        })?;

        if let Some(user) = existing {
            return Ok(user);
        }

        // Create new user
        let id = sqlx::query("INSERT INTO users (name) VALUES (?1)")
            .bind(username)
            .execute(&self.pool)
            .await
            .map_err(|e| DatabaseError::QueryFailed {
                context: format!("Failed to create user '{}'", username),
                source: e,
            })?
            .last_insert_rowid();

        self.get_user_by_id(id).await
    }

    /// Get user by ID
    async fn get_user_by_id(&self, id: i64) -> Result<User> {
        sqlx::query_as::<_, User>(
            "SELECT id, name, discord_id, created_at FROM users WHERE id = ?1",
        )
        .bind(id)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| {
            match e {
                sqlx::Error::RowNotFound => DatabaseError::UserNotFound(id.to_string()),
                _ => DatabaseError::QueryFailed {
                    context: format!("Failed to fetch user with id {}", id),
                    source: e,
                },
            }
            .into()
        })
    }

    /// Get agent for a user
    pub async fn get_agent_for_user(&self, user_id: i64) -> Result<Option<Agent>> {
        sqlx::query_as::<_, Agent>(
            "SELECT id, user_id, letta_agent_id, name, created_at FROM agents WHERE user_id = ?1",
        )
        .bind(user_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| {
            DatabaseError::QueryFailed {
                context: format!("Failed to fetch agent for user {}", user_id),
                source: e,
            }
            .into()
        })
    }

    /// Create a new agent
    pub async fn create_agent(
        &self,
        user_id: i64,
        letta_agent_id: &str,
        name: &str,
    ) -> Result<i64> {
        let id =
            sqlx::query("INSERT INTO agents (user_id, letta_agent_id, name) VALUES (?1, ?2, ?3)")
                .bind(user_id)
                .bind(letta_agent_id)
                .bind(name)
                .execute(&self.pool)
                .await
                .map_err(|e| DatabaseError::QueryFailed {
                    context: format!("Failed to create agent '{}'", name),
                    source: e,
                })?
                .last_insert_rowid();

        Ok(id)
    }

    /// Get shared memory blocks for a user
    pub async fn get_shared_memory(&self, user_id: i64) -> Result<Vec<SharedMemory>> {
        sqlx::query_as::<_, SharedMemory>(
            "SELECT id, user_id, block_name, block_value, max_length, created_at, updated_at
             FROM shared_memory WHERE user_id = ?1",
        )
        .bind(user_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| {
            DatabaseError::QueryFailed {
                context: format!("Failed to fetch shared memory for user {}", user_id),
                source: e,
            }
            .into()
        })
    }

    /// Get a specific shared memory block
    pub async fn get_shared_memory_block(
        &self,
        user_id: i64,
        block_name: &str,
    ) -> Result<Option<SharedMemory>> {
        sqlx::query_as::<_, SharedMemory>(
            "SELECT id, user_id, block_name, block_value, max_length, created_at, updated_at
             FROM shared_memory WHERE user_id = ?1 AND block_name = ?2",
        )
        .bind(user_id)
        .bind(block_name)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| {
            DatabaseError::QueryFailed {
                context: format!("Failed to fetch shared memory block '{}'", block_name),
                source: e,
            }
            .into()
        })
    }

    /// Upsert a shared memory block
    pub async fn upsert_shared_memory(
        &self,
        user_id: i64,
        block_name: &str,
        block_value: &str,
        max_length: i32,
    ) -> Result<()> {
        sqlx::query(
            "INSERT INTO shared_memory (user_id, block_name, block_value, max_length)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(user_id, block_name) DO UPDATE SET
                block_value = excluded.block_value,
                updated_at = CURRENT_TIMESTAMP",
        )
        .bind(user_id)
        .bind(block_name)
        .bind(block_value)
        .bind(max_length)
        .execute(&self.pool)
        .await
        .map_err(|e| DatabaseError::QueryFailed {
            context: format!("Failed to upsert shared memory block '{}'", block_name),
            source: e,
        })?;

        Ok(())
    }
}

/// User entity
#[derive(Debug, Clone, sqlx::FromRow, Serialize, Deserialize)]
pub struct User {
    pub id: i64,
    pub name: String,
    pub discord_id: Option<String>,
    pub created_at: sqlx::types::chrono::DateTime<sqlx::types::chrono::Utc>,
}

/// Agent entity
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Agent {
    pub id: i64,
    pub user_id: i64,
    pub letta_agent_id: String,
    pub name: String,
    pub created_at: sqlx::types::chrono::DateTime<sqlx::types::chrono::Utc>,
}

/// Task entity
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Task {
    pub id: i64,
    pub user_id: i64,
    pub title: String,
    pub description: Option<String>,
    pub status: String,
    pub priority: String,
    pub estimated_minutes: Option<i32>,
    pub actual_minutes: Option<i32>,
    pub created_at: sqlx::types::chrono::DateTime<sqlx::types::chrono::Utc>,
    pub updated_at: sqlx::types::chrono::DateTime<sqlx::types::chrono::Utc>,
    pub completed_at: Option<sqlx::types::chrono::DateTime<sqlx::types::chrono::Utc>>,
}

/// Event entity
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Event {
    pub id: i64,
    pub user_id: i64,
    pub title: String,
    pub description: Option<String>,
    pub start_time: sqlx::types::chrono::DateTime<sqlx::types::chrono::Utc>,
    pub end_time: sqlx::types::chrono::DateTime<sqlx::types::chrono::Utc>,
    pub location: Option<String>,
    pub created_at: sqlx::types::chrono::DateTime<sqlx::types::chrono::Utc>,
    pub updated_at: sqlx::types::chrono::DateTime<sqlx::types::chrono::Utc>,
}

/// Shared memory block for multi-agent coordination
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct SharedMemory {
    pub id: i64,
    pub user_id: i64,
    pub block_name: String,
    pub block_value: String,
    pub max_length: i32,
    pub created_at: sqlx::types::chrono::DateTime<sqlx::types::chrono::Utc>,
    pub updated_at: sqlx::types::chrono::DateTime<sqlx::types::chrono::Utc>,
}

/// Time tracking entry
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct TimeTracking {
    pub id: i64,
    pub user_id: i64,
    pub task_id: Option<i64>,
    pub start_time: sqlx::types::chrono::DateTime<sqlx::types::chrono::Utc>,
    pub end_time: Option<sqlx::types::chrono::DateTime<sqlx::types::chrono::Utc>>,
    pub duration_minutes: Option<i32>,
    pub description: Option<String>,
    pub created_at: sqlx::types::chrono::DateTime<sqlx::types::chrono::Utc>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_database_connection_errors() {
        // Test invalid path should fail
        let result = Database::new("/nonexistent/path/db.sqlite").await;
        assert!(result.is_err());

        // Verify specific error type
        match result.unwrap_err() {
            crate::error::PatternError::Database(DatabaseError::ConnectionFailed {
                path, ..
            }) => {
                assert!(path.contains("/nonexistent/path"));
            }
            _ => panic!("Expected ConnectionFailed error"),
        }
    }

    #[tokio::test]
    async fn test_upsert_shared_memory() {
        let db = Database::new(":memory:").await.unwrap();
        db.migrate().await.unwrap();

        // First create a user
        let user = db.get_or_create_user("testuser").await.unwrap();
        let user_id = user.id;

        // Insert new block
        db.upsert_shared_memory(user_id, "test_block", "initial value", 100)
            .await
            .unwrap();

        let block = db
            .get_shared_memory_block(user_id, "test_block")
            .await
            .unwrap()
            .unwrap();

        assert_eq!(block.block_value, "initial value");
        assert_eq!(block.max_length, 100);
        let first_update = block.updated_at;

        // Sleep longer to ensure timestamp changes in SQLite
        tokio::time::sleep(tokio::time::Duration::from_millis(1000)).await;

        // Update existing block
        db.upsert_shared_memory(user_id, "test_block", "updated value", 100)
            .await
            .unwrap();

        let updated = db
            .get_shared_memory_block(user_id, "test_block")
            .await
            .unwrap()
            .unwrap();

        assert_eq!(updated.block_value, "updated value");
        assert!(updated.updated_at > first_update);
        assert_eq!(updated.created_at, block.created_at); // created_at shouldn't change
    }

    #[tokio::test]
    async fn test_agent_uniqueness() {
        let db = Database::new(":memory:").await.unwrap();
        db.migrate().await.unwrap();

        let user = db.get_or_create_user("testuser").await.unwrap();

        // Create first agent
        let id1 = db
            .create_agent(user.id, "letta_123", "Agent One")
            .await
            .unwrap();

        // Try to create another agent with same letta_agent_id (should fail due to unique constraint)
        let result = db.create_agent(user.id, "letta_123", "Agent Two").await;

        assert!(result.is_err());
        // Verify it's a database constraint error
        match result.unwrap_err() {
            crate::error::PatternError::Database(DatabaseError::QueryFailed {
                context, ..
            }) => {
                assert!(context.contains("Failed to create agent"));
            }
            _ => panic!("Expected QueryFailed error"),
        }

        // Creating with different letta_id should work
        let id2 = db
            .create_agent(user.id, "letta_456", "Agent Two")
            .await
            .unwrap();

        assert_ne!(id1, id2);
    }

    #[tokio::test]
    async fn test_get_or_create_user_idempotent() {
        let db = Database::new(":memory:").await.unwrap();
        db.migrate().await.unwrap();

        // First call creates user
        let user1 = db.get_or_create_user("testuser").await.unwrap();

        // Second call returns same user
        let user2 = db.get_or_create_user("testuser").await.unwrap();

        assert_eq!(user1.id, user2.id);
        assert_eq!(user1.name, user2.name);
        assert_eq!(user1.created_at, user2.created_at);

        // Different username creates different user
        let user3 = db.get_or_create_user("otheruser").await.unwrap();
        assert_ne!(user1.id, user3.id);
    }

    #[tokio::test]
    async fn test_shared_memory_constraints() {
        let db = Database::new(":memory:").await.unwrap();
        db.migrate().await.unwrap();

        // Test with invalid user_id (foreign key constraint)
        let result = db
            .upsert_shared_memory(999, "test_block", "value", 100)
            .await;

        assert!(result.is_err());
        // Verify it's a database error (foreign key constraint)
        match result.unwrap_err() {
            crate::error::PatternError::Database(DatabaseError::QueryFailed { .. }) => {}
            _ => panic!("Expected QueryFailed error for foreign key constraint"),
        }

        // Create user first
        let user = db.get_or_create_user("testuser").await.unwrap();

        // Test very long block name (assuming reasonable limit in schema)
        let long_name = "a".repeat(1000);
        let result = db
            .upsert_shared_memory(user.id, &long_name, "value", 100)
            .await;

        // This might succeed or fail depending on schema - just verify it doesn't panic
        let _ = result;
    }

    #[test]
    fn test_database_is_send_sync() {
        // Compile-time test that Database can be shared across threads
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<Database>();
    }
}
