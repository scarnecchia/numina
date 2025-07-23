use crate::{
    AgentId,
    db::{DatabaseError, entity::EntityError},
};
use compact_str::CompactString;
use miette::Diagnostic;
use thiserror::Error;

#[derive(Error, Diagnostic, Debug)]
pub enum CoreError {
    #[error("Agent not found")]
    #[diagnostic(
        code(pattern_core::agent_not_found),
        help("Check that the agent ID is correct and the agent has been created")
    )]
    AgentNotFound {
        #[source_code]
        src: String,
        #[label("agent ID: {id}")]
        span: (usize, usize),
        id: String,
    },

    #[error("Agent initialization failed")]
    #[diagnostic(
        code(pattern_core::agent_init_failed),
        help("Check the agent configuration and ensure all required fields are provided")
    )]
    AgentInitFailed {
        agent_type: String,
        #[source]
        cause: Box<dyn std::error::Error + Send + Sync>,
    },

    #[error("Memory block not found")]
    #[diagnostic(
        code(pattern_core::memory_not_found),
        help("The requested memory block doesn't exist for this agent")
    )]
    MemoryNotFound {
        agent_id: String,
        block_name: String,
        available_blocks: Vec<CompactString>,
    },

    #[error("Memory operation failed")]
    #[diagnostic(
        code(pattern_core::memory_operation_failed),
        help("Check database connectivity and permissions")
    )]
    MemoryOperationFailed {
        operation: String,
        agent_id: String,
        #[source]
        cause: Box<dyn std::error::Error + Send + Sync>,
    },

    #[error("Tool not found")]
    #[diagnostic(
        code(pattern_core::tool_not_found),
        help("Available tools: {}", available_tools.join(", "))
    )]
    ToolNotFound {
        tool_name: String,
        available_tools: Vec<String>,
        #[source_code]
        src: String,
        #[label("unknown tool")]
        span: (usize, usize),
    },

    #[error("Tool execution failed")]
    #[diagnostic(
        code(pattern_core::tool_execution_failed),
        help("Check tool parameters and ensure they match the expected schema")
    )]
    ToolExecutionFailed {
        tool_name: String,
        #[source]
        cause: Box<dyn std::error::Error + Send + Sync>,
        parameters: serde_json::Value,
    },

    #[error("Invalid tool parameters")]
    #[diagnostic(
        code(pattern_core::invalid_tool_params),
        help("Expected schema: {expected_schema}")
    )]
    InvalidToolParameters {
        tool_name: String,
        expected_schema: serde_json::Value,
        provided_params: serde_json::Value,
        validation_errors: Vec<String>,
    },

    #[error("Model provider error")]
    #[diagnostic(
        code(pattern_core::model_provider_error),
        help("Check API credentials and rate limits for {provider}")
    )]
    ModelProviderError {
        provider: String,
        model: String,
        #[source]
        cause: Box<dyn std::error::Error + Send + Sync>,
    },

    #[error("Model capability mismatch")]
    #[diagnostic(
        code(pattern_core::model_capability_mismatch),
        help(
            "Model '{model}' doesn't support {required_capability}. Consider using a model with {required_capability} capability"
        )
    )]
    ModelCapabilityMismatch {
        model: String,
        required_capability: String,
        available_capabilities: Vec<String>,
    },

    #[error("Database connection failed")]
    #[diagnostic(
        code(pattern_core::database_connection_failed),
        help("Ensure SurrealDB is running at {connection_string}")
    )]
    DatabaseConnectionFailed {
        connection_string: String,
        #[source]
        cause: surrealdb::Error,
    },

    #[error("Database query failed")]
    #[diagnostic(code(pattern_core::database_query_failed), help("Query: {query}"))]
    DatabaseQueryFailed {
        query: String,
        table: String,
        #[source]
        cause: surrealdb::Error,
    },

    #[error("Serialization error")]
    #[diagnostic(
        code(pattern_core::serialization_error),
        help("Failed to serialize/deserialize {data_type}")
    )]
    SerializationError {
        data_type: String,
        #[source]
        cause: serde_json::Error,
    },

    #[error("Configuration error")]
    #[diagnostic(
        code(pattern_core::configuration_error),
        help("Check configuration file at {config_path}")
    )]
    ConfigurationError {
        config_path: String,
        field: String,
        expected: String,
        #[source]
        cause: Box<dyn std::error::Error + Send + Sync>,
    },

    #[error("Agent coordination failed")]
    #[diagnostic(
        code(pattern_core::coordination_failed),
        help("Coordination pattern '{pattern}' failed for group '{group}'")
    )]
    CoordinationFailed {
        group: String,
        pattern: String,
        participating_agents: Vec<String>,
        #[source]
        cause: Box<dyn std::error::Error + Send + Sync>,
    },

    #[error("Constellation not found")]
    #[diagnostic(
        code(pattern_core::constellation_not_found),
        help("No constellation found for user {user_id}")
    )]
    ConstellationNotFound {
        user_id: String,
        available_constellations: Vec<String>,
    },

    #[error("Invalid agent state")]
    #[diagnostic(
        code(pattern_core::invalid_agent_state),
        help(
            "Agent {agent_id} is in state '{current_state}' but operation requires state '{required_state}'"
        )
    )]
    InvalidAgentState {
        agent_id: String,
        current_state: String,
        required_state: String,
        operation: String,
    },

    #[error("Context window exceeded")]
    #[diagnostic(
        code(pattern_core::context_window_exceeded),
        help(
            "Message history exceeds model's context window. Consider summarizing older messages"
        )
    )]
    ContextWindowExceeded {
        model: String,
        token_count: usize,
        max_tokens: usize,
        message_count: usize,
    },

    #[error("Real-time subscription failed")]
    #[diagnostic(
        code(pattern_core::realtime_subscription_failed),
        help("Failed to establish LIVE query subscription")
    )]
    RealtimeSubscriptionFailed {
        query: String,
        #[source]
        cause: surrealdb::Error,
    },

    #[error("Vector search failed")]
    #[diagnostic(
        code(pattern_core::vector_search_failed),
        help("Failed to perform semantic search on {collection}")
    )]
    VectorSearchFailed {
        collection: String,
        dimension_mismatch: Option<(usize, usize)>,
        #[source]
        cause: Box<dyn std::error::Error + Send + Sync>,
    },

    #[error("Agent group error")]
    #[diagnostic(
        code(pattern_core::agent_group_error),
        help("Operation failed for agent group '{group_name}'")
    )]
    AgentGroupError {
        group_name: String,
        operation: String,
        #[source]
        cause: Box<dyn std::error::Error + Send + Sync>,
    },

    #[error("Permission denied")]
    #[diagnostic(
        code(pattern_core::permission_denied),
        help("User {user_id} doesn't have permission to {action} on {resource}")
    )]
    PermissionDenied {
        user_id: String,
        action: String,
        resource: String,
        required_permission: String,
    },

    #[error("Rate limit exceeded")]
    #[diagnostic(
        code(pattern_core::rate_limit_exceeded),
        help("Wait {retry_after_seconds} seconds before retrying")
    )]
    RateLimitExceeded {
        service: String,
        limit: usize,
        window_seconds: usize,
        retry_after_seconds: usize,
    },

    #[error("Resource exhausted")]
    #[diagnostic(
        code(pattern_core::resource_exhausted),
        help(
            "System resource '{resource}' is exhausted. Current usage: {current_usage}, limit: {limit}"
        )
    )]
    ResourceExhausted {
        resource: String,
        current_usage: String,
        limit: String,
    },
}

pub type Result<T> = std::result::Result<T, CoreError>;

impl From<DatabaseError> for CoreError {
    fn from(err: DatabaseError) -> Self {
        match err {
            DatabaseError::ConnectionFailed(e) => Self::DatabaseConnectionFailed {
                connection_string: "embedded".to_string(),
                cause: e,
            },
            DatabaseError::QueryFailed(e) => Self::DatabaseQueryFailed {
                query: "unknown".to_string(),
                table: "unknown".to_string(),
                cause: e,
            },

            DatabaseError::SerdeProblem(e) => Self::SerializationError {
                data_type: "database record".to_string(),
                cause: e,
            },
            DatabaseError::NotFound { entity_type, id } => Self::AgentNotFound {
                src: format!("database: {} with id {}", entity_type, id),
                span: (10, 10 + id.len()),
                id,
            },
            DatabaseError::EmbeddingError(e) => Self::VectorSearchFailed {
                collection: "unknown".to_string(),
                dimension_mismatch: None,
                cause: Box::new(e),
            },
            DatabaseError::EmbeddingModelMismatch {
                db_model,
                config_model,
            } => Self::ConfigurationError {
                config_path: "database".to_string(),
                field: "embedding_model".to_string(),
                expected: db_model.clone(),
                cause: Box::new(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!(
                        "Model mismatch: database has {}, config has {}",
                        db_model, config_model
                    ),
                )),
            },
            DatabaseError::SchemaVersionMismatch {
                db_version,
                code_version,
            } => Self::DatabaseQueryFailed {
                query: "schema version check".to_string(),
                table: "system_metadata".to_string(),
                cause: surrealdb::Error::Db(surrealdb::error::Db::Tx(format!(
                    "Schema version mismatch: database v{}, code v{}",
                    db_version, code_version
                ))),
            },
            DatabaseError::InvalidVectorDimensions { expected, actual } => {
                Self::VectorSearchFailed {
                    collection: "unknown".to_string(),
                    dimension_mismatch: Some((expected, actual)),
                    cause: Box::new(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        format!(
                            "Invalid vector dimensions: expected {}, got {}",
                            expected, actual
                        ),
                    )),
                }
            }
            DatabaseError::TransactionFailed(e) => Self::DatabaseQueryFailed {
                query: "transaction".to_string(),
                table: "unknown".to_string(),
                cause: e,
            },
            DatabaseError::SurrealJsonValueError { original, help } => Self::DatabaseQueryFailed {
                query: help,
                table: "".to_string(),
                cause: original,
            },
            DatabaseError::Other(msg) => Self::DatabaseQueryFailed {
                query: "unknown".to_string(),
                table: "unknown".to_string(),
                cause: surrealdb::Error::Db(surrealdb::error::Db::Tx(msg)),
            },
        }
    }
}

impl From<EntityError> for CoreError {
    fn from(err: EntityError) -> Self {
        // Convert EntityError to DatabaseError, then to CoreError
        let db_err: DatabaseError = err.into();
        db_err.into()
    }
}

// Helper functions for creating common errors with context
impl CoreError {
    pub fn agent_not_found(id: impl Into<String>) -> Self {
        let id = id.into();
        Self::AgentNotFound {
            src: format!("agent_id: {}", id),
            span: (10, 10 + id.len()),
            id,
        }
    }

    pub fn memory_not_found(
        agent_id: &AgentId,
        block_name: impl Into<String>,
        available_blocks: Vec<CompactString>,
    ) -> Self {
        Self::MemoryNotFound {
            agent_id: agent_id.to_string(),
            block_name: block_name.into(),
            available_blocks,
        }
    }

    pub fn tool_not_found(name: impl Into<String>, available: Vec<String>) -> Self {
        let name = name.into();
        Self::ToolNotFound {
            tool_name: name.clone(),
            available_tools: available.to_vec(),
            src: format!("tool: {}", name),
            span: (6, 6 + name.len()),
        }
    }

    pub fn database_connection_failed(
        connection_string: impl Into<String>,
        cause: surrealdb::Error,
    ) -> Self {
        Self::DatabaseConnectionFailed {
            connection_string: connection_string.into(),
            cause,
        }
    }

    pub fn model_error(
        provider: impl Into<String>,
        model: impl Into<String>,
        cause: impl std::error::Error + Send + Sync + 'static,
    ) -> Self {
        Self::ModelProviderError {
            provider: provider.into(),
            model: model.into(),
            cause: Box::new(cause),
        }
    }

    pub fn context_exceeded(
        model: impl Into<String>,
        token_count: usize,
        max_tokens: usize,
        message_count: usize,
    ) -> Self {
        Self::ContextWindowExceeded {
            model: model.into(),
            token_count,
            max_tokens,
            message_count,
        }
    }

    pub fn tool_validation_error(tool_name: impl Into<String>, error: impl Into<String>) -> Self {
        let tool_name = tool_name.into();
        Self::InvalidToolParameters {
            tool_name,
            expected_schema: serde_json::Value::Null,
            provided_params: serde_json::Value::Null,
            validation_errors: vec![error.into()],
        }
    }

    pub fn tool_execution_error(tool_name: impl Into<String>, error: impl Into<String>) -> Self {
        #[derive(Debug, Error)]
        #[error("{0}")]
        struct StringError(String);

        Self::ToolExecutionFailed {
            tool_name: tool_name.into(),
            cause: Box::new(StringError(error.into())),
            parameters: serde_json::Value::Null,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use miette::Report;

    #[test]
    fn test_agent_not_found_error() {
        let error = CoreError::agent_not_found("test_agent_123");
        let report = Report::new(error);
        let output = format!("{:?}", report);
        assert!(output.contains("agent_not_found"));
        assert!(output.contains("test_agent_123"));
    }

    #[test]
    fn test_tool_not_found_with_suggestions() {
        let error = CoreError::tool_not_found(
            "unknown_tool",
            vec![
                "tool1".to_string(),
                "tool2".to_string(),
                "tool3".to_string(),
            ],
        );
        let report = Report::new(error);
        let output = format!("{:?}", report);
        assert!(output.contains("Available tools: tool1, tool2, tool3"));
    }
}
