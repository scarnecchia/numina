use miette::Diagnostic;
use thiserror::Error;

#[derive(Error, Diagnostic, Debug)]
pub enum McpError {
    #[error("Transport initialization failed")]
    #[diagnostic(
        code(pattern::mcp::transport_init_failed),
        help("Failed to initialize {transport_type} transport on {endpoint}")
    )]
    TransportInitFailed {
        transport_type: String,
        endpoint: String,
        #[source]
        cause: Box<dyn std::error::Error + Send + Sync>,
    },

    #[error("Transport connection lost")]
    #[diagnostic(
        code(pattern::mcp::transport_connection_lost),
        help("Connection to {endpoint} was lost. Check network connectivity and retry")
    )]
    TransportConnectionLost {
        endpoint: String,
        transport_type: String,
        duration_since_last_message: std::time::Duration,
        #[source]
        cause: Box<dyn std::error::Error + Send + Sync>,
    },

    #[error("Invalid MCP message")]
    #[diagnostic(
        code(pattern::mcp::invalid_message),
        help("Received invalid MCP message. Expected {expected}, got {actual}")
    )]
    InvalidMessage {
        expected: String,
        actual: String,
        #[source_code]
        raw_message: String,
        #[label("invalid here")]
        span: (usize, usize),
    },

    #[error("Protocol version mismatch")]
    #[diagnostic(
        code(pattern::mcp::protocol_version_mismatch),
        help("Client expects protocol version {client_version}, but server supports {server_version}")
    )]
    ProtocolVersionMismatch {
        client_version: String,
        server_version: String,
        supported_versions: Vec<String>,
    },

    #[error("Tool not registered")]
    #[diagnostic(
        code(pattern::mcp::tool_not_registered),
        help("Tool '{tool_name}' is not registered. Available tools: {}", available_tools.join(", "))
    )]
    ToolNotRegistered {
        tool_name: String,
        available_tools: Vec<String>,
        did_you_mean: Option<String>,
    },

    #[error("Tool execution failed")]
    #[diagnostic(
        code(pattern::mcp::tool_execution_failed),
        help("Tool '{tool_name}' failed during execution")
    )]
    ToolExecutionFailed {
        tool_name: String,
        #[source]
        cause: pattern_core::CoreError,
        execution_time: std::time::Duration,
        partial_result: Option<serde_json::Value>,
    },

    #[error("Invalid tool parameters")]
    #[diagnostic(
        code(pattern::mcp::invalid_tool_parameters),
        help("Tool '{tool_name}' received invalid parameters")
    )]
    InvalidToolParameters {
        tool_name: String,
        #[source_code]
        provided_params: String,
        #[label("parameter validation failed here")]
        error_location: (usize, usize),
        validation_errors: Vec<ValidationError>,
    },

    #[error("Serialization failed")]
    #[diagnostic(
        code(pattern::mcp::serialization_failed),
        help("Failed to serialize {data_type} for MCP protocol")
    )]
    SerializationFailed {
        data_type: String,
        #[source]
        cause: serde_json::Error,
        #[source_code]
        data_sample: String,
    },

    #[error("Deserialization failed")]
    #[diagnostic(
        code(pattern::mcp::deserialization_failed),
        help("Failed to deserialize MCP message as {expected_type}")
    )]
    DeserializationFailed {
        expected_type: String,
        #[source]
        cause: serde_json::Error,
        #[source_code]
        raw_data: String,
        #[label("failed to parse here")]
        error_location: (usize, usize),
    },

    #[error("Server bind failed")]
    #[diagnostic(
        code(pattern::mcp::server_bind_failed),
        help("Failed to bind MCP server to {address}. Is the port already in use?")
    )]
    ServerBindFailed {
        address: String,
        transport_type: String,
        #[source]
        cause: std::io::Error,
        suggestions: Vec<String>,
    },

    #[error("Client handshake failed")]
    #[diagnostic(
        code(pattern::mcp::handshake_failed),
        help("Failed to complete MCP handshake with client")
    )]
    HandshakeFailed {
        client_id: Option<String>,
        stage: HandshakeStage,
        #[source]
        cause: Box<dyn std::error::Error + Send + Sync>,
    },

    #[error("Session not found")]
    #[diagnostic(
        code(pattern::mcp::session_not_found),
        help("No active session found for client {client_id}")
    )]
    SessionNotFound {
        client_id: String,
        active_sessions: Vec<String>,
        session_expired: bool,
    },

    #[error("Rate limit exceeded")]
    #[diagnostic(
        code(pattern::mcp::rate_limit_exceeded),
        help("Client {client_id} exceeded rate limit: {requests} requests in {window:?}")
    )]
    RateLimitExceeded {
        client_id: String,
        requests: usize,
        window: std::time::Duration,
        retry_after: std::time::Duration,
    },

    #[error("Transport write failed")]
    #[diagnostic(
        code(pattern::mcp::transport_write_failed),
        help("Failed to write message to transport")
    )]
    TransportWriteFailed {
        transport_type: String,
        message_size: usize,
        #[source]
        cause: std::io::Error,
    },

    #[error("Transport read failed")]
    #[diagnostic(
        code(pattern::mcp::transport_read_failed),
        help("Failed to read message from transport")
    )]
    TransportReadFailed {
        transport_type: String,
        bytes_read: usize,
        #[source]
        cause: std::io::Error,
    },

    #[error("SSE stream error")]
    #[diagnostic(
        code(pattern::mcp::sse_stream_error),
        help("Server-sent event stream encountered an error")
    )]
    SseStreamError {
        event_type: Option<String>,
        last_event_id: Option<String>,
        #[source]
        cause: Box<dyn std::error::Error + Send + Sync>,
    },

    #[error("HTTP transport error")]
    #[diagnostic(
        code(pattern::mcp::http_transport_error),
        help("HTTP transport error: {status_code} {status_text}")
    )]
    HttpTransportError {
        status_code: u16,
        status_text: String,
        method: String,
        path: String,
        #[source]
        cause: Option<Box<dyn std::error::Error + Send + Sync>>,
    },

    #[error("Tool timeout")]
    #[diagnostic(
        code(pattern::mcp::tool_timeout),
        help("Tool '{tool_name}' execution timed out after {timeout:?}")
    )]
    ToolTimeout {
        tool_name: String,
        timeout: std::time::Duration,
        partial_result: Option<serde_json::Value>,
    },

    #[error("Invalid transport configuration")]
    #[diagnostic(
        code(pattern::mcp::invalid_transport_config),
        help("Transport configuration for {transport_type} is invalid")
    )]
    InvalidTransportConfig {
        transport_type: String,
        config_errors: Vec<String>,
        example_config: String,
    },
}

#[derive(Debug, Clone)]
pub struct ValidationError {
    pub field: String,
    pub expected: String,
    pub actual: String,
    pub message: String,
}

#[derive(Debug, Clone, Copy)]
pub enum HandshakeStage {
    Initial,
    Negotiation,
    Authentication,
    Completion,
}

impl std::fmt::Display for HandshakeStage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Initial => write!(f, "initial connection"),
            Self::Negotiation => write!(f, "protocol negotiation"),
            Self::Authentication => write!(f, "authentication"),
            Self::Completion => write!(f, "handshake completion"),
        }
    }
}

pub type Result<T> = std::result::Result<T, McpError>;

// Helper functions for creating common errors
impl McpError {
    pub fn tool_not_found(name: impl Into<String>, available: Vec<String>) -> Self {
        let name = name.into();
        // Simple fuzzy matching for suggestions
        let did_you_mean = available
            .iter()
            .find(|tool| tool.to_lowercase().contains(&name.to_lowercase()))
            .cloned();

        Self::ToolNotRegistered {
            tool_name: name,
            available_tools: available,
            did_you_mean,
        }
    }

    pub fn invalid_params(
        tool_name: impl Into<String>,
        params: &serde_json::Value,
        errors: Vec<ValidationError>,
    ) -> Self {
        let params_str = serde_json::to_string_pretty(params).unwrap_or_default();
        let error_location = if let Some(first_error) = errors.first() {
            if let Some(pos) = params_str.find(&first_error.field) {
                (pos, pos + first_error.field.len())
            } else {
                (0, params_str.len())
            }
        } else {
            (0, params_str.len())
        };

        Self::InvalidToolParameters {
            tool_name: tool_name.into(),
            provided_params: params_str,
            error_location,
            validation_errors: errors,
        }
    }

    pub fn transport_init(
        transport_type: impl Into<String>,
        endpoint: impl Into<String>,
        cause: impl std::error::Error + Send + Sync + 'static,
    ) -> Self {
        Self::TransportInitFailed {
            transport_type: transport_type.into(),
            endpoint: endpoint.into(),
            cause: Box::new(cause),
        }
    }

    pub fn server_bind(
        address: impl Into<String>,
        transport_type: impl Into<String>,
        cause: std::io::Error,
    ) -> Self {
        let addr = address.into();
        let suggestions = vec![
            format!("Check if another process is using the port"),
            format!("Try a different port number"),
            format!("Ensure you have permission to bind to {}", addr),
        ];

        Self::ServerBindFailed {
            address: addr,
            transport_type: transport_type.into(),
            cause,
            suggestions,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use miette::Report;

    #[test]
    fn test_tool_not_found_with_suggestion() {
        let error = McpError::tool_not_found(
            "get_memeory",
            vec!["get_memory".to_string(), "set_memory".to_string()],
        );

        if let McpError::ToolNotRegistered { did_you_mean, .. } = &error {
            assert_eq!(did_you_mean.as_deref(), Some("get_memory"));
        } else {
            panic!("Wrong error type");
        }
    }

    #[test]
    fn test_validation_error_display() {
        let validation_errors = vec![
            ValidationError {
                field: "name".to_string(),
                expected: "non-empty string".to_string(),
                actual: "empty string".to_string(),
                message: "Name is required".to_string(),
            },
            ValidationError {
                field: "age".to_string(),
                expected: "positive integer".to_string(),
                actual: "-5".to_string(),
                message: "Age must be positive".to_string(),
            },
        ];

        let params = serde_json::json!({
            "name": "",
            "age": -5
        });

        let error = McpError::invalid_params("create_user", &params, validation_errors);
        let report = Report::new(error);
        let output = format!("{:?}", report);

        assert!(output.contains("create_user"));
        assert!(output.contains("parameter validation failed"));
    }
}
