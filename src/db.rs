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

/// Shared memory block for multi-agent coordination
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct SharedMemory {
    pub id: i64,
    pub user_id: i64,
    pub block_name: String,
    pub block_value: String,
    pub max_length: i32,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

/// Client entity for contract work
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Client {
    pub id: i64,
    pub user_id: i64,
    pub name: String,
    pub company: Option<String>,
    pub email: Option<String>,
    pub phone: Option<String>,
    pub notes: Option<String>,
    pub hourly_rate: Option<f64>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

/// Invoice entity for payment tracking
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Invoice {
    pub id: i64,
    pub user_id: i64,
    pub client_id: i64,
    pub invoice_number: String,
    pub amount: f64,
    pub status: String,
    pub issued_at: Option<chrono::DateTime<chrono::Utc>>,
    pub due_at: Option<chrono::DateTime<chrono::Utc>>,
    pub paid_at: Option<chrono::DateTime<chrono::Utc>>,
    pub notes: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

/// Social contact for relationship tracking
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct SocialContact {
    pub id: i64,
    pub user_id: i64,
    pub name: String,
    pub relationship: Option<String>,
    pub birthday: Option<chrono::NaiveDate>,
    pub anniversary: Option<chrono::NaiveDate>,
    pub notes: Option<String>,
    pub last_contact: Option<chrono::DateTime<chrono::Utc>>,
    pub energy_cost: Option<i32>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

/// Energy state for pattern tracking
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct EnergyState {
    pub id: i64,
    pub user_id: i64,
    pub energy_level: i32,
    pub attention_state: String,
    pub mood: Option<String>,
    pub last_break_minutes: Option<i32>,
    pub notes: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
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

    /// Get or update a shared memory block
    pub async fn upsert_shared_memory(
        &self,
        user_id: i64,
        block_name: &str,
        block_value: &str,
        max_length: i32,
    ) -> Result<SharedMemory> {
        let now = chrono::Utc::now();

        // Try to update existing block
        let updated = sqlx::query(
            "UPDATE shared_memory 
             SET block_value = ?1, max_length = ?2, updated_at = ?3
             WHERE user_id = ?4 AND block_name = ?5",
        )
        .bind(block_value)
        .bind(max_length)
        .bind(&now)
        .bind(user_id)
        .bind(block_name)
        .execute(&self.pool)
        .await
        .into_diagnostic()?;

        if updated.rows_affected() > 0 {
            // Fetch and return updated block
            let block = sqlx::query_as::<_, SharedMemory>(
                "SELECT * FROM shared_memory WHERE user_id = ?1 AND block_name = ?2",
            )
            .bind(user_id)
            .bind(block_name)
            .fetch_one(&self.pool)
            .await
            .into_diagnostic()?;

            Ok(block)
        } else {
            // Create new block
            let id = sqlx::query_scalar(
                "INSERT INTO shared_memory (user_id, block_name, block_value, max_length, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6) RETURNING id"
            )
            .bind(user_id)
            .bind(block_name)
            .bind(block_value)
            .bind(max_length)
            .bind(&now)
            .bind(&now)
            .fetch_one(&self.pool)
            .await
            .into_diagnostic()?;

            Ok(SharedMemory {
                id,
                user_id,
                block_name: block_name.to_string(),
                block_value: block_value.to_string(),
                max_length,
                created_at: now,
                updated_at: now,
            })
        }
    }

    /// Get all shared memory blocks for a user
    pub async fn get_shared_memory(&self, user_id: i64) -> Result<Vec<SharedMemory>> {
        let blocks = sqlx::query_as::<_, SharedMemory>(
            "SELECT * FROM shared_memory WHERE user_id = ?1 ORDER BY block_name",
        )
        .bind(user_id)
        .fetch_all(&self.pool)
        .await
        .into_diagnostic()?;

        Ok(blocks)
    }

    /// Get a specific shared memory block
    pub async fn get_shared_memory_block(
        &self,
        user_id: i64,
        block_name: &str,
    ) -> Result<Option<SharedMemory>> {
        let block = sqlx::query_as::<_, SharedMemory>(
            "SELECT * FROM shared_memory WHERE user_id = ?1 AND block_name = ?2",
        )
        .bind(user_id)
        .bind(block_name)
        .fetch_optional(&self.pool)
        .await
        .into_diagnostic()?;

        Ok(block)
    }

    /// Record an energy state
    pub async fn record_energy_state(
        &self,
        user_id: i64,
        energy_level: i32,
        attention_state: &str,
        mood: Option<&str>,
        last_break_minutes: Option<i32>,
        notes: Option<&str>,
    ) -> Result<EnergyState> {
        let now = chrono::Utc::now();
        let id = sqlx::query_scalar(
            "INSERT INTO energy_states (user_id, energy_level, attention_state, mood, last_break_minutes, notes, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7) RETURNING id"
        )
        .bind(user_id)
        .bind(energy_level)
        .bind(attention_state)
        .bind(mood)
        .bind(last_break_minutes)
        .bind(notes)
        .bind(&now)
        .fetch_one(&self.pool)
        .await
        .into_diagnostic()?;

        Ok(EnergyState {
            id,
            user_id,
            energy_level,
            attention_state: attention_state.to_string(),
            mood: mood.map(|s| s.to_string()),
            last_break_minutes,
            notes: notes.map(|s| s.to_string()),
            created_at: now,
        })
    }

    /// Get recent energy states for pattern analysis
    pub async fn get_recent_energy_states(
        &self,
        user_id: i64,
        limit: i32,
    ) -> Result<Vec<EnergyState>> {
        let states = sqlx::query_as::<_, EnergyState>(
            "SELECT * FROM energy_states 
             WHERE user_id = ?1 
             ORDER BY created_at DESC 
             LIMIT ?2",
        )
        .bind(user_id)
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .into_diagnostic()?;

        Ok(states)
    }
}
