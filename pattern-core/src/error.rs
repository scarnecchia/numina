use thiserror::Error;

#[derive(Error, Debug)]
pub enum CoreError {
    #[error("Agent error: {0}")]
    Agent(String),

    #[error("Memory error: {0}")]
    Memory(String),

    #[error("Tool error: {0}")]
    Tool(String),

    #[error("Model error: {0}")]
    Model(String),

    #[error("Database error: {0}")]
    Database(#[from] surrealdb::Error),

    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Configuration error: {0}")]
    Configuration(String),

    #[error("Not found: {0}")]
    NotFound(String),

    #[error("Invalid state: {0}")]
    InvalidState(String),

    #[error("Coordination error: {0}")]
    Coordination(String),

    #[error("Other error: {0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, CoreError>;
