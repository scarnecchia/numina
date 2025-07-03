use crate::{
    agents::AgentConfig,
    error::{ConfigError, Result},
    types::McpTransport,
};
use serde::{Deserialize, Serialize};
use std::{env, path::Path};

/// Main configuration for Pattern
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// Database configuration
    pub database: DatabaseConfig,
    /// Discord bot configuration
    pub discord: DiscordConfig,
    /// Letta configuration
    pub letta: LettaConfig,
    /// MCP server configuration
    pub mcp: McpConfig,
    /// Agent configurations (optional, uses defaults if empty)
    #[serde(default)]
    pub agents: Vec<AgentConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DatabaseConfig {
    /// Path to SQLite database file
    pub path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscordConfig {
    /// Discord bot token
    pub token: String,
    /// Discord application ID
    pub application_id: Option<u64>,
    /// Optional channel ID to limit bot responses
    pub channel_id: Option<u64>,
    /// Whether to respond to DMs
    pub respond_to_dms: bool,
    /// Whether to respond to mentions
    pub respond_to_mentions: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LettaConfig {
    /// Base URL for Letta server
    pub base_url: String,
    /// Optional API key for Letta cloud
    pub api_key: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpConfig {
    /// Whether MCP server is enabled
    pub enabled: bool,
    /// MCP transport type
    pub transport: McpTransport,
    /// Port for HTTP/SSE transports
    pub port: Option<u16>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            database: DatabaseConfig {
                path: "pattern.db".to_string(),
            },
            discord: DiscordConfig {
                token: String::new(),
                application_id: None,
                channel_id: None,
                respond_to_dms: true,
                respond_to_mentions: true,
            },
            letta: LettaConfig {
                base_url: "http://localhost:8000".to_string(),
                api_key: None,
            },
            mcp: McpConfig {
                enabled: false,
                transport: McpTransport::Stdio,
                port: None,
            },
            agents: Vec::new(),
        }
    }
}

impl Config {
    /// Validate the configuration
    pub fn validate(&self) -> Result<()> {
        if self.database.path.is_empty() {
            return Err(ConfigError::Invalid {
                field: "database.path".to_string(),
                reason: "Database path cannot be empty".to_string(),
            }
            .into());
        }

        if self.letta.base_url.is_empty() && self.letta.api_key.is_none() {
            return Err(ConfigError::Invalid {
                field: "letta".to_string(),
                reason: "Either base_url or api_key must be provided".to_string(),
            }
            .into());
        }

        Ok(())
    }

    /// Load configuration from environment variables and config file
    pub fn load() -> Result<Self> {
        // Try to load from file first
        let config_path = env::var("PATTERN_CONFIG").unwrap_or_else(|_| "pattern.toml".to_string());

        if Path::new(&config_path).exists() {
            let contents =
                std::fs::read_to_string(&config_path).map_err(|_e| ConfigError::NotFound {
                    path: config_path.clone(),
                })?;
            let mut config: Config =
                toml::from_str(&contents).map_err(|e| ConfigError::ParseFailed { source: e })?;

            // Override with environment variables
            config = Self::override_from_env(config);
            Ok(config)
        } else {
            // Load from environment variables only
            Ok(Self::from_env())
        }
    }

    /// Create config from environment variables
    pub fn from_env() -> Self {
        let mut config = Self::default();

        // Database
        if let Ok(path) = env::var("DATABASE_PATH") {
            config.database.path = path;
        }

        // Discord
        if let Ok(token) = env::var("DISCORD_TOKEN") {
            config.discord.token = token;
        }
        if let Ok(app_id) = env::var("APP_ID") {
            if let Ok(id) = app_id.parse() {
                config.discord.application_id = Some(id);
            }
        }
        if let Ok(channel_id) = env::var("DISCORD_CHANNEL_ID") {
            if let Ok(id) = channel_id.parse() {
                config.discord.channel_id = Some(id);
            }
        }

        // Letta
        if let Ok(url) = env::var("LETTA_BASE_URL") {
            config.letta.base_url = url;
        }
        if let Ok(api_key) = env::var("LETTA_API_KEY") {
            config.letta.api_key = Some(api_key);
        }

        // MCP
        if let Ok(enabled) = env::var("MCP_ENABLED") {
            config.mcp.enabled = enabled.to_lowercase() == "true";
        }
        if let Ok(transport) = env::var("MCP_TRANSPORT") {
            if let Ok(t) = transport.parse() {
                config.mcp.transport = t;
            }
        }
        if let Ok(port) = env::var("MCP_PORT") {
            if let Ok(p) = port.parse() {
                config.mcp.port = Some(p);
            }
        }

        config
    }

    /// Override config values with environment variables
    fn override_from_env(mut self) -> Self {
        // Database
        if let Ok(path) = env::var("DATABASE_PATH") {
            self.database.path = path;
        }

        // Discord
        if let Ok(token) = env::var("DISCORD_TOKEN") {
            self.discord.token = token;
        }
        if let Ok(app_id) = env::var("APP_ID") {
            if let Ok(id) = app_id.parse() {
                self.discord.application_id = Some(id);
            }
        }
        if let Ok(channel_id) = env::var("DISCORD_CHANNEL_ID") {
            if let Ok(id) = channel_id.parse() {
                self.discord.channel_id = Some(id);
            }
        }

        // Letta
        if let Ok(url) = env::var("LETTA_BASE_URL") {
            self.letta.base_url = url;
        }
        if let Ok(api_key) = env::var("LETTA_API_KEY") {
            self.letta.api_key = Some(api_key);
        }

        // MCP
        if let Ok(enabled) = env::var("MCP_ENABLED") {
            self.mcp.enabled = enabled.to_lowercase() == "true";
        }
        if let Ok(transport) = env::var("MCP_TRANSPORT") {
            if let Ok(t) = transport.parse() {
                self.mcp.transport = t;
            }
        }
        if let Ok(port) = env::var("MCP_PORT") {
            if let Ok(p) = port.parse() {
                self.mcp.port = Some(p);
            }
        }

        self
    }
}

/// Helper to load dotenv file if it exists
pub fn load_dotenv() {
    if let Ok(path) = env::var("DOTENV_PATH") {
        dotenv::from_path(&path).ok();
    } else {
        dotenv::dotenv().ok();
    }
}
