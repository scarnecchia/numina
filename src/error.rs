use crate::types::{AgentId, MemoryBlockId};
use miette::Diagnostic;
use thiserror::Error;

/// Main error type for Pattern operations
#[derive(Error, Debug, Diagnostic)]
pub enum PatternError {
    #[error("Database error")]
    #[diagnostic(help("Check database connection and permissions"))]
    Database(#[from] DatabaseError),

    #[error("Agent error")]
    #[diagnostic(help("Check agent configuration and Letta connection"))]
    Agent(#[from] AgentError),

    #[error("Discord error")]
    #[diagnostic(help("Check Discord bot token and permissions"))]
    #[cfg(feature = "discord")]
    Discord(#[from] DiscordError),

    #[error("Configuration error")]
    #[diagnostic(help("Check your configuration file"))]
    Config(#[from] ConfigError),

    #[error("Validation error")]
    #[diagnostic(help("Check input format and constraints"))]
    Validation(#[from] crate::types::ValidationError),
}

/// Database-specific errors
#[derive(Error, Debug, Diagnostic)]
pub enum DatabaseError {
    #[error("Failed to connect to database at {path}")]
    #[diagnostic(
        code(pattern::db::connection_failed),
        help("Ensure the database path exists and is writable")
    )]
    ConnectionFailed {
        path: String,
        #[source]
        source: sqlx::Error,
    },

    #[error("Migration failed")]
    #[diagnostic(
        code(pattern::db::migration_failed),
        help("Check migration files and database schema")
    )]
    MigrationFailed {
        #[source]
        source: sqlx::migrate::MigrateError,
    },

    #[error("User not found: {0}")]
    #[diagnostic(
        code(pattern::db::user_not_found),
        help("User may need to be initialized first")
    )]
    UserNotFound(String),

    #[error("Agent not found for user {user_id}")]
    #[diagnostic(
        code(pattern::db::agent_not_found),
        help("Initialize the multi-agent system for this user")
    )]
    AgentNotFound { user_id: i64 },

    #[error("Memory block '{block}' not found for user {user_id}")]
    #[diagnostic(
        code(pattern::db::memory_block_not_found),
        help("Initialize shared memory for this user")
    )]
    MemoryBlockNotFound { user_id: i64, block: String },

    #[error("Query failed")]
    #[diagnostic(code(pattern::db::query_failed))]
    QueryFailed {
        context: String,
        #[source]
        source: sqlx::Error,
    },

    #[error("DB Error: {0}")]
    #[diagnostic()]
    Other(sqlx::Error),
}

/// Agent-specific errors
#[derive(Error, Debug, Diagnostic)]
pub enum AgentError {
    #[error("Failed to connect to Letta at {url}")]
    #[diagnostic(
        code(pattern::agent::letta_connection_failed),
        help("Check Letta server is running and accessible")
    )]
    LettaConnectionFailed {
        url: String,
        #[source]
        source: letta::LettaError,
    },

    #[error("Agent '{agent}' not found in configuration")]
    #[diagnostic(
        code(pattern::agent::unknown_agent),
        help("Available agents: pattern, entropy, flux, archive, momentum, anchor")
    )]
    UnknownAgent { agent: AgentId },

    #[error("Failed to create agent '{name}'")]
    #[diagnostic(
        code(pattern::agent::creation_failed),
        help("Check Letta connection and agent configuration")
    )]
    CreationFailed {
        name: String,
        #[source]
        source: letta::LettaError,
    },

    #[error("Failed to send message to agent")]
    #[diagnostic(
        code(pattern::agent::message_failed),
        help("Agent may be overloaded or misconfigured")
    )]
    MessageFailed {
        agent: AgentId,
        #[source]
        source: letta::LettaError,
    },

    #[error("Memory block '{block}' exceeds max length of {max} (got {actual})")]
    #[diagnostic(
        code(pattern::agent::memory_too_long),
        help("Shorten the memory content or increase the limit")
    )]
    MemoryTooLong {
        block: MemoryBlockId,
        max: usize,
        actual: usize,
    },

    #[error("Invalid Letta agent ID: {0}")]
    #[diagnostic(
        code(pattern::agent::invalid_letta_id),
        help("Letta IDs should be valid UUIDs")
    )]
    InvalidLettaId(String),
    #[error("Letta Error: {0}")]
    #[diagnostic()]
    Other(#[from] letta::LettaError),
}

/// Discord-specific errors
#[cfg(feature = "discord")]
#[derive(Error, Debug, Diagnostic)]
pub enum DiscordError {
    #[error("Discord bot token not configured")]
    #[diagnostic(
        code(pattern::discord::no_token),
        help("Set DISCORD_TOKEN in .env or config file")
    )]
    NoToken,

    #[error("Failed to connect to Discord")]
    #[diagnostic(
        code(pattern::discord::connection_failed),
        help("Check bot token and network connection")
    )]
    ConnectionFailed {
        #[source]
        source: serenity::Error,
    },

    #[error("Failed to send message to channel {channel_id}")]
    #[diagnostic(
        code(pattern::discord::send_failed),
        help("Check bot permissions in the channel")
    )]
    SendFailed {
        channel_id: u64,
        #[source]
        source: serenity::Error,
    },

    #[error("Command interaction failed")]
    #[diagnostic(
        code(pattern::discord::interaction_failed),
        help("Command may have timed out or permissions missing")
    )]
    InteractionFailed {
        command: String,
        #[source]
        source: serenity::Error,
    },
    #[error("Discord error: {0}")]
    #[diagnostic()]
    Other(#[from] serenity::Error),
}

/// Configuration errors
#[derive(Error, Debug, Diagnostic)]
pub enum ConfigError {
    #[error("Configuration file not found at {path}")]
    #[diagnostic(
        code(pattern::config::not_found),
        help("Create a config file or use environment variables")
    )]
    NotFound { path: String },

    #[error("Invalid configuration")]
    #[diagnostic(
        code(pattern::config::invalid),
        help("Check configuration format and required fields")
    )]
    Invalid { field: String, reason: String },

    #[error("Failed to parse configuration")]
    #[diagnostic(
        code(pattern::config::parse_failed),
        help("Check TOML syntax and field types")
    )]
    ParseFailed {
        #[source]
        source: toml::de::Error,
    },
}

/// Type alias for Results in Pattern
pub type Result<T> = std::result::Result<T, PatternError>;

/// Extension trait for adding context to errors
pub trait ErrorContext<T> {
    fn context(self, msg: &str) -> Result<T>;
    fn with_context<F>(self, f: F) -> Result<T>
    where
        F: FnOnce() -> String;
}

impl<T, E> ErrorContext<T> for std::result::Result<T, E>
where
    E: Into<PatternError>,
{
    fn context(self, _msg: &str) -> Result<T> {
        self.map_err(|e| {
            let base_error = e.into();
            // Add context to the error chain
            base_error
        })
    }

    fn with_context<F>(self, f: F) -> Result<T>
    where
        F: FnOnce() -> String,
    {
        self.map_err(|e| {
            let _context = f();
            e.into()
        })
    }
}

/// Convert SQLx errors to our domain errors
impl From<sqlx::Error> for DatabaseError {
    fn from(err: sqlx::Error) -> Self {
        match err {
            sqlx::Error::RowNotFound => {
                // This will be caught and converted to more specific errors
                DatabaseError::QueryFailed {
                    context: "Row not found".to_string(),
                    source: err,
                }
            }
            sqlx::Error::ColumnNotFound(ref s) => DatabaseError::QueryFailed {
                context: format!("Column {s} not found"),
                source: err,
            },
            // Add some more specific options here potentially
            _ => DatabaseError::Other(err),
        }
    }
}
