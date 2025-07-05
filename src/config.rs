use crate::{
    agent::constellation::AgentConfig,
    error::{ConfigError, Result},
    mcp::McpTransport,
};
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, env, path::Path};

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
    /// Path to external agent configuration file
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_config_path: Option<String>,
    /// Model capability mappings
    #[serde(default)]
    pub models: ModelConfig,
    /// Partner configurations
    #[serde(skip_serializing_if = "Option::is_none")]
    pub partners: Option<PartnersConfig>,
    /// Cache configuration
    #[serde(default)]
    pub cache: CacheConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DatabaseConfig {
    /// Path to SQLite database file
    pub path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheConfig {
    /// Path to cache directory
    pub directory: String,
}

impl Default for CacheConfig {
    fn default() -> Self {
        Self {
            directory: ".pattern_cache".to_string(),
        }
    }
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

/// Model capability configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelConfig {
    /// Default model mappings for each capability level
    #[serde(default)]
    pub default: CapabilityModels,
    /// Per-agent model overrides (agent_id -> capability mappings)
    #[serde(default)]
    pub agents: HashMap<String, CapabilityModels>,
}

/// Partners configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PartnersConfig {
    /// List of partner users
    pub users: Vec<PartnerUser>,
}

/// Individual partner configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PartnerUser {
    /// Discord user ID as string
    pub discord_id: String,
    /// Partner's display name
    pub name: String,
    /// Whether to auto-initialize at boot
    #[serde(default)]
    pub auto_initialize: bool,
}

/// Model mappings for each capability level
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapabilityModels {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub routine: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub interactive: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub investigative: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub critical: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
}

impl Default for ModelConfig {
    fn default() -> Self {
        Self {
            default: CapabilityModels::default(),
            agents: HashMap::new(),
        }
    }
}

impl Default for CapabilityModels {
    fn default() -> Self {
        Self {
            routine: Some("groq/llama-3.1-8b-instant".to_string()),
            interactive: Some("groq/llama-3.3-70b-versatile".to_string()),
            investigative: Some("openai/gpt-4o".to_string()),
            critical: Some("anthropic/claude-3-opus-20240229".to_string()),
            temperature: Some(0.7),
        }
    }
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
                enabled: false, // Disabled by default for simplicity
                transport: McpTransport::Stdio,
                port: None,
            },
            agents: Vec::new(),
            agent_config_path: None,
            models: ModelConfig::default(),
            partners: None,
            cache: CacheConfig::default(),
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

        // Agent config path
        if let Ok(path) = env::var("AGENT_CONFIG_PATH") {
            config.agent_config_path = Some(path);
        }

        // Model configs are loaded from file only, not env vars
        config.models = ModelConfig::default();

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

        // Agent config path
        if let Ok(path) = env::var("AGENT_CONFIG_PATH") {
            self.agent_config_path = Some(path);
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
