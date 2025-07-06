use miette::Diagnostic;
use std::path::PathBuf;
use thiserror::Error;

#[derive(Error, Diagnostic, Debug)]
pub enum PatternError {
    #[error("Configuration file not found")]
    #[diagnostic(
        code(pattern::main::config_not_found),
        help("Create a configuration file at {} or specify one with --config", expected_path.display())
    )]
    ConfigFileNotFound {
        expected_path: PathBuf,
        search_paths: Vec<PathBuf>,
        current_dir: PathBuf,
    },

    #[error("Configuration parse error")]
    #[diagnostic(
        code(pattern::main::config_parse_error),
        help("Fix the syntax error in {}", config_path.display())
    )]
    ConfigParseError {
        #[source_code]
        config_content: String,
        #[label("error occurred here")]
        span: (usize, usize),
        #[source]
        cause: toml::de::Error,
        config_path: PathBuf,
    },

    #[error("Invalid configuration")]
    #[diagnostic(code(pattern::main::config_invalid), help("{suggestion}"))]
    ConfigInvalid {
        field_path: String,
        expected: String,
        actual: String,
        suggestion: String,
        #[source_code]
        config_section: String,
        #[label("invalid value here")]
        span: (usize, usize),
    },

    #[error("Database initialization failed")]
    #[diagnostic(
        code(pattern::main::database_init_failed),
        help("Ensure SurrealDB is installed and accessible")
    )]
    DatabaseInitFailed {
        connection_string: String,
        #[source]
        cause: surrealdb::Error,
        suggestions: Vec<String>,
    },

    #[error("Service startup failed")]
    #[diagnostic(
        code(pattern::main::service_startup_failed),
        help("Service '{service_name}' failed to start")
    )]
    ServiceStartupFailed {
        service_name: String,
        #[source]
        cause: Box<dyn std::error::Error + Send + Sync>,
        dependencies_status: Vec<ServiceStatus>,
    },

    #[error("Migration failed")]
    #[diagnostic(
        code(pattern::main::migration_failed),
        help("Database migration {migration_name} failed to apply")
    )]
    MigrationFailed {
        migration_name: String,
        migration_version: String,
        #[source]
        cause: Box<dyn std::error::Error + Send + Sync>,
        applied_migrations: Vec<String>,
        pending_migrations: Vec<String>,
    },

    #[error("Environment variable missing")]
    #[diagnostic(
        code(pattern::main::env_var_missing),
        help("Set the environment variable: export {var_name}={example_value}")
    )]
    EnvVarMissing {
        var_name: String,
        example_value: String,
        used_for: String,
        alternatives: Vec<String>,
    },

    #[error("Port already in use")]
    #[diagnostic(
        code(pattern::main::port_in_use),
        help("Port {port} is already in use by {process_hint}")
    )]
    PortInUse {
        port: u16,
        service: String,
        process_hint: String,
        alternative_ports: Vec<u16>,
        #[source]
        cause: std::io::Error,
    },

    #[error("Feature not enabled")]
    #[diagnostic(
        code(pattern::main::feature_not_enabled),
        help("Rebuild with feature '{feature}' enabled: cargo build --features {feature}")
    )]
    FeatureNotEnabled {
        feature: String,
        requested_functionality: String,
        available_features: Vec<String>,
        compilation_command: String,
    },

    #[error("Graceful shutdown failed")]
    #[diagnostic(
        code(pattern::main::shutdown_failed),
        help("Some services did not shut down cleanly")
    )]
    GracefulShutdownFailed {
        failed_services: Vec<String>,
        timeout_seconds: u64,
        forcefully_terminated: Vec<String>,
    },

    #[error("Lock acquisition failed")]
    #[diagnostic(
        code(pattern::main::lock_acquisition_failed),
        help("Another instance of Pattern may already be running")
    )]
    LockAcquisitionFailed {
        lock_file: PathBuf,
        #[source]
        cause: std::io::Error,
        process_id: Option<u32>,
    },

    #[error("Signal handler error")]
    #[diagnostic(
        code(pattern::main::signal_handler_error),
        help("Failed to set up signal handler for {}", signal)
    )]
    SignalHandlerError {
        signal: String,
        #[source]
        cause: std::io::Error,
    },

    #[error("Plugin load failed")]
    #[diagnostic(
        code(pattern::main::plugin_load_failed),
        help("Failed to load plugin from {}", plugin_path.display())
    )]
    PluginLoadFailed {
        plugin_path: PathBuf,
        plugin_name: String,
        #[source]
        cause: Box<dyn std::error::Error + Send + Sync>,
        api_version_expected: String,
        api_version_found: Option<String>,
    },

    #[error("Logging initialization failed")]
    #[diagnostic(
        code(pattern::main::logging_init_failed),
        help("Failed to initialize logging subsystem")
    )]
    LoggingInitFailed {
        log_file: Option<PathBuf>,
        log_level: String,
        #[source]
        cause: Box<dyn std::error::Error + Send + Sync>,
    },

    #[error("API key invalid")]
    #[diagnostic(
        code(pattern::main::api_key_invalid),
        help("The API key for {service} is invalid or expired")
    )]
    ApiKeyInvalid {
        service: String,
        key_source: String, // "environment variable", "config file", etc.
        validation_error: String,
        documentation_url: String,
    },

    #[error("Resource limit exceeded")]
    #[diagnostic(
        code(pattern::main::resource_limit_exceeded),
        help("System resource limit exceeded: {resource}")
    )]
    ResourceLimitExceeded {
        resource: String,
        current_usage: String,
        limit: String,
        suggestions: Vec<String>,
    },

    // Bridge errors from sub-crates
    #[error(transparent)]
    #[diagnostic(transparent)]
    Core(#[from] pattern_core::CoreError),

    #[cfg(feature = "discord")]
    #[error(transparent)]
    #[diagnostic(transparent)]
    Discord(#[from] pattern_discord::DiscordError),

    #[cfg(feature = "mcp")]
    #[error(transparent)]
    #[diagnostic(transparent)]
    Mcp(#[from] pattern_mcp::McpError),

    #[error("IO error")]
    #[diagnostic(
        code(pattern::main::io_error),
        help("IO operation failed: {operation}")
    )]
    Io {
        operation: String,
        path: Option<PathBuf>,
        #[source]
        cause: std::io::Error,
    },

    #[error("Serialization error")]
    #[diagnostic(code(pattern::main::serialization_error))]
    Serialization {
        format: String,
        data_type: String,
        #[source]
        cause: serde_json::Error,
    },

    #[error("Unknown error")]
    #[diagnostic(
        code(pattern::main::unknown_error),
        help("An unexpected error occurred")
    )]
    Unknown {
        context: String,
        #[source]
        cause: Box<dyn std::error::Error + Send + Sync>,
    },
}

#[derive(Debug, Clone)]
pub struct ServiceStatus {
    pub name: String,
    pub status: Status,
    pub message: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Starting,
    Running,
    Failed,
    Stopped,
}

impl std::fmt::Display for Status {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Starting => write!(f, "starting"),
            Self::Running => write!(f, "running"),
            Self::Failed => write!(f, "failed"),
            Self::Stopped => write!(f, "stopped"),
        }
    }
}

pub type Result<T> = std::result::Result<T, PatternError>;

// Helper constructors for common errors
impl PatternError {
    pub fn config_not_found(expected: PathBuf) -> Self {
        let search_paths = vec![
            PathBuf::from("pattern.toml"),
            PathBuf::from("config/pattern.toml"),
            PathBuf::from("/etc/pattern/pattern.toml"),
        ];

        Self::ConfigFileNotFound {
            expected_path: expected,
            search_paths,
            current_dir: std::env::current_dir().unwrap_or_default(),
        }
    }

    pub fn config_parse(path: PathBuf, content: String, error: toml::de::Error) -> Self {
        // Try to extract line/column from toml error
        let span = if let Some(span) = error.span() {
            (span.start, span.end)
        } else {
            (0, content.len())
        };

        Self::ConfigParseError {
            config_content: content,
            span,
            cause: error,
            config_path: path,
        }
    }

    pub fn invalid_config(
        field: impl Into<String>,
        expected: impl Into<String>,
        actual: impl Into<String>,
    ) -> Self {
        let field = field.into();
        let expected = expected.into();
        let actual = actual.into();

        let suggestion = match field.as_str() {
            "database.url" => {
                "Ensure SurrealDB is running and the URL is correct (e.g., ws://localhost:8000)"
                    .to_string()
            }
            "discord.token" => {
                "Get a bot token from https://discord.com/developers/applications".to_string()
            }
            "models.default_provider" => {
                format!("Supported providers: anthropic, openai, gemini")
            }
            _ => format!("Expected {} but got {}", expected, actual),
        };

        Self::ConfigInvalid {
            field_path: field,
            expected,
            actual,
            suggestion,
            config_section: String::new(), // Would be populated with actual config section
            span: (0, 0),
        }
    }

    pub fn port_in_use(port: u16, service: impl Into<String>) -> Self {
        let alternatives = vec![port + 1, port + 2, port + 10, 8080, 8081, 3000];

        Self::PortInUse {
            port,
            service: service.into(),
            process_hint: "another process".to_string(),
            alternative_ports: alternatives,
            cause: std::io::Error::new(std::io::ErrorKind::AddrInUse, "Address already in use"),
        }
    }

    pub fn env_var_missing(var_name: impl Into<String>, used_for: impl Into<String>) -> Self {
        let var_name = var_name.into();
        let used_for = used_for.into();

        let (example_value, alternatives) = match var_name.as_str() {
            "DISCORD_TOKEN" => (
                "YOUR_BOT_TOKEN_HERE".to_string(),
                vec!["Set in pattern.toml instead".to_string()],
            ),
            "ANTHROPIC_API_KEY" => (
                "sk-ant-api03-...".to_string(),
                vec![
                    "Set in pattern.toml under [models.providers.anthropic]".to_string(),
                    "Use OPENAI_API_KEY instead".to_string(),
                ],
            ),
            _ => ("<value>".to_string(), vec![]),
        };

        Self::EnvVarMissing {
            var_name,
            example_value,
            used_for,
            alternatives,
        }
    }

    pub fn io_error(operation: impl Into<String>, cause: std::io::Error) -> Self {
        Self::Io {
            operation: operation.into(),
            path: None,
            cause,
        }
    }

    pub fn io_error_with_path(
        operation: impl Into<String>,
        path: PathBuf,
        cause: std::io::Error,
    ) -> Self {
        Self::Io {
            operation: operation.into(),
            path: Some(path),
            cause,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use miette::Report;

    #[test]
    fn test_config_not_found_shows_search_paths() {
        let error = PatternError::config_not_found(PathBuf::from("custom.toml"));
        let report = Report::new(error);
        let output = format!("{:?}", report);

        assert!(output.contains("pattern.toml"));
        assert!(output.contains("config/pattern.toml"));
        assert!(output.contains("/etc/pattern/pattern.toml"));
    }

    #[test]
    fn test_invalid_config_suggestions() {
        let error = PatternError::invalid_config("database.url", "valid URL", "not-a-url");

        if let PatternError::ConfigInvalid { suggestion, .. } = &error {
            assert!(suggestion.contains("ws://localhost:8000"));
        }
    }

    #[test]
    fn test_port_alternatives() {
        let error = PatternError::port_in_use(8080, "MCP server");

        if let PatternError::PortInUse {
            alternative_ports, ..
        } = &error
        {
            assert!(alternative_ports.contains(&8081));
            assert!(alternative_ports.contains(&3000));
        }
    }
}
