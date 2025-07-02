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

        let pool = SqlitePool::connect(&format!("sqlite://{}", db_path))
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
            "SELECT id, username, created_at FROM users WHERE username = ?1",
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
        let id = sqlx::query("INSERT INTO users (username) VALUES (?1)")
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
        sqlx::query_as::<_, User>("SELECT id, username, created_at FROM users WHERE id = ?1")
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
    pub username: String,
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
