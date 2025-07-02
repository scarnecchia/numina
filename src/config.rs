use miette::{IntoDiagnostic, Result};
use serde::{Deserialize, Serialize};
use std::env;

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
    pub transport: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            database: DatabaseConfig {
                path: "pattern.db".to_string(),
            },
            discord: DiscordConfig {
                token: String::new(),
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
                transport: "stdio".to_string(),
            },
        }
    }
}

impl Config {
    /// Load configuration from environment variables and config file
    pub fn load() -> Result<Self> {
        let mut config = Self::default();

        // Override with environment variables
        if let Ok(db_path) = env::var("PATTERN_DB_PATH") {
            config.database.path = db_path;
        }

        if let Ok(token) = env::var("DISCORD_TOKEN") {
            config.discord.token = token;
        }

        if let Ok(channel_id) = env::var("DISCORD_CHANNEL_ID") {
            config.discord.channel_id = channel_id.parse().ok();
        }

        if let Ok(letta_url) = env::var("LETTA_BASE_URL") {
            config.letta.base_url = letta_url;
        }

        if let Ok(api_key) = env::var("LETTA_API_KEY") {
            config.letta.api_key = Some(api_key);
        }

        // Try to load from config file
        if let Ok(config_path) = env::var("PATTERN_CONFIG") {
            config = Self::load_from_file(&config_path)?;
        } else if let Ok(contents) = std::fs::read_to_string("pattern.toml") {
            if let Ok(file_config) = toml::from_str::<Config>(&contents) {
                // Merge with environment variables (env vars take precedence)
                config = Self::merge(file_config, config);
            }
        }

        Ok(config)
    }

    /// Load configuration from a TOML file
    pub fn load_from_file(path: &str) -> Result<Self> {
        let contents = std::fs::read_to_string(path).into_diagnostic()?;
        toml::from_str(&contents).into_diagnostic()
    }

    /// Merge two configs, with the second taking precedence
    fn merge(base: Self, override_config: Self) -> Self {
        Self {
            database: DatabaseConfig {
                path: if override_config.database.path.is_empty() {
                    base.database.path
                } else {
                    override_config.database.path
                },
            },
            discord: DiscordConfig {
                token: if override_config.discord.token.is_empty() {
                    base.discord.token
                } else {
                    override_config.discord.token
                },
                channel_id: override_config
                    .discord
                    .channel_id
                    .or(base.discord.channel_id),
                respond_to_dms: override_config.discord.respond_to_dms,
                respond_to_mentions: override_config.discord.respond_to_mentions,
            },
            letta: LettaConfig {
                base_url: if override_config.letta.base_url == "http://localhost:8000" {
                    base.letta.base_url
                } else {
                    override_config.letta.base_url
                },
                api_key: override_config.letta.api_key.or(base.letta.api_key),
            },
            mcp: McpConfig {
                enabled: override_config.mcp.enabled,
                transport: override_config.mcp.transport,
            },
        }
    }

    /// Validate the configuration
    pub fn validate(&self) -> Result<()> {
        if self.discord.token.is_empty() {
            return Err(miette::miette!("Discord token is required"));
        }

        Ok(())
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
