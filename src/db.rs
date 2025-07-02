use miette::{IntoDiagnostic, Result};
use sqlx::{sqlite::SqlitePoolOptions, types::chrono, SqlitePool};
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
        // Ensure the parent directory exists
        if let Some(parent) = Path::new(db_path).parent() {
            std::fs::create_dir_all(parent).into_diagnostic()?;
        }

        info!("Connecting to database at: {}", db_path);

        let pool = SqlitePoolOptions::new()
            .max_connections(5)
            .connect(&format!("sqlite://{}", db_path))
            .await
            .into_diagnostic()?;

        debug!("Database connection established");

        Ok(Self { pool })
    }

    /// Run database migrations
    pub async fn migrate(&self) -> Result<()> {
        info!("Running database migrations");

        sqlx::migrate!("./migrations")
            .run(&self.pool)
            .await
            .into_diagnostic()?;

        info!("Database migrations completed");
        Ok(())
    }

    /// Get a reference to the connection pool
    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }
}

/// User entity
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct User {
    pub id: i64,
    pub discord_id: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

/// Agent entity
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Agent {
    pub id: i64,
    pub user_id: i64,
    pub letta_agent_id: String,
    pub name: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
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
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
    pub completed_at: Option<chrono::DateTime<chrono::Utc>>,
}

/// Event entity
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Event {
    pub id: i64,
    pub user_id: i64,
    pub title: String,
    pub description: Option<String>,
    pub start_time: chrono::DateTime<chrono::Utc>,
    pub end_time: chrono::DateTime<chrono::Utc>,
    pub location: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

impl Database {
    /// Find or create a user by discord ID
    pub async fn get_or_create_user(&self, discord_id: &str) -> Result<User> {
        // Try to find existing user
        let existing = sqlx::query_as::<_, User>("SELECT * FROM users WHERE discord_id = ?1")
            .bind(discord_id)
            .fetch_optional(&self.pool)
            .await
            .into_diagnostic()?;

        if let Some(user) = existing {
            Ok(user)
        } else {
            // Create new user
            let now = chrono::Utc::now();
            let id = sqlx::query_scalar(
                "INSERT INTO users (discord_id, created_at, updated_at) VALUES (?1, ?2, ?3) RETURNING id"
            )
            .bind(discord_id)
            .bind(&now)
            .bind(&now)
            .fetch_one(&self.pool)
            .await
            .into_diagnostic()?;

            Ok(User {
                id,
                discord_id: Some(discord_id.to_string()),
                created_at: now,
                updated_at: now,
            })
        }
    }

    /// Get agent for a user
    pub async fn get_agent_for_user(&self, user_id: i64) -> Result<Option<Agent>> {
        let agent = sqlx::query_as::<_, Agent>(
            "SELECT * FROM agents WHERE user_id = ?1 ORDER BY created_at DESC LIMIT 1",
        )
        .bind(user_id)
        .fetch_optional(&self.pool)
        .await
        .into_diagnostic()?;

        Ok(agent)
    }

    /// Create a new agent for a user
    pub async fn create_agent(
        &self,
        user_id: i64,
        letta_agent_id: &str,
        name: &str,
    ) -> Result<Agent> {
        let now = chrono::Utc::now();
        let id = sqlx::query_scalar(
            "INSERT INTO agents (user_id, letta_agent_id, name, created_at, updated_at) 
             VALUES (?1, ?2, ?3, ?4, ?5) RETURNING id",
        )
        .bind(user_id)
        .bind(letta_agent_id)
        .bind(name)
        .bind(&now)
        .bind(&now)
        .fetch_one(&self.pool)
        .await
        .into_diagnostic()?;

        Ok(Agent {
            id,
            user_id,
            letta_agent_id: letta_agent_id.to_string(),
            name: name.to_string(),
            created_at: now,
            updated_at: now,
        })
    }
}
