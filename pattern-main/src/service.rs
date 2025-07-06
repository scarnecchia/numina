use crate::{
    agent::{
        self,
        builder::MultiAgentSystemBuilder,
        constellation::{AgentConfig, MemoryBlockConfig, MultiAgentSystem},
        UserId,
    },
    config::{Config, PartnersConfig},
    db::Database,
    error::{PatternError, Result},
    mcp::McpTransport,
    sleeptime::SleeptimeMonitor,
};
use std::sync::Arc;
use tracing::{error, info};

// Include default agent configuration at compile time
const DEFAULT_AGENTS_CONFIG: &str = include_str!("default_agents.toml");

/// Core service orchestrator for Pattern
pub struct PatternService {
    #[allow(dead_code)] // Used by Discord/MCP features
    config: Config,
    #[allow(dead_code)] // Used by MCP feature
    db: Arc<Database>,
    #[allow(dead_code)] // Used by MCP feature
    letta_client: Arc<letta::LettaClient>,
    multi_agent_system: Arc<MultiAgentSystem>,
    #[cfg(feature = "discord")]
    discord_client: Option<Arc<tokio::sync::RwLock<serenity::Client>>>,
}

impl PatternService {
    /// Initialize the Pattern service
    pub async fn new(config: Config) -> Result<Self> {
        config.validate()?;

        // Initialize database
        let db = Database::new(&config.database.path).await?;
        db.migrate().await?;
        let db = Arc::new(db);

        info!("Database initialized");

        // Initialize Letta client
        let letta_client = Self::create_letta_client(&config).await?;
        let letta_client = Arc::new(letta_client);

        // Initialize multi-agent system
        let multi_agent_system =
            Self::create_multi_agent_system(letta_client.clone(), db.clone(), &config).await?;
        let multi_agent_system = Arc::new(multi_agent_system);

        info!(
            "Multi-agent system initialized with {} agents",
            multi_agent_system.agent_configs().len()
        );

        Ok(Self {
            config,
            db,
            letta_client,
            multi_agent_system,
            #[cfg(feature = "discord")]
            discord_client: None,
        })
    }

    /// Create Letta client based on configuration
    async fn create_letta_client(config: &Config) -> Result<letta::LettaClient> {
        info!("Connecting to Letta at {}", config.letta.base_url);

        // Create auth config based on whether API key is present
        let auth_config = if let Some(api_key) = &config.letta.api_key {
            info!("Using API key authentication");
            letta::auth::AuthConfig::bearer(api_key)
        } else {
            info!("Using no authentication");
            letta::auth::AuthConfig::none()
        };

        // Use configured base URL whether using API key or not
        // This allows using local server with API keys (e.g. for Gemini)
        let client = letta::LettaClient::builder()
            .base_url(&config.letta.base_url)
            .auth(auth_config)
            .timeout(std::time::Duration::from_secs(120)) // 2 minute timeout
            .build()
            .map_err(|e| crate::error::AgentError::LettaConnectionFailed {
                url: config.letta.base_url.clone(),
                source: e,
            })?;
        Ok(client)
    }

    /// Create multi-agent system with standard agents
    async fn create_multi_agent_system(
        letta_client: Arc<letta::LettaClient>,
        db: Arc<Database>,
        config: &Config,
    ) -> Result<MultiAgentSystem> {
        let mut builder = MultiAgentSystemBuilder::new(letta_client, db);

        // Initialize cache if configured
        let cache_dir = std::path::Path::new(&config.cache.directory);
        match crate::cache::PatternCache::new(cache_dir).await {
            Ok(cache) => {
                info!("Cache initialized at {:?}", cache_dir);
                builder = builder.with_cache(Arc::new(cache));
            }
            Err(e) => {
                error!(
                    "Failed to initialize cache: {:?}. Continuing without cache.",
                    e
                );
                // Continue without cache - it's optional
            }
        }

        // Check if we should load from external config file
        if let Some(config_path) = &config.agent_config_path {
            info!("Loading agent configuration from: {}", config_path);
            builder = builder.with_config_file(config_path).await?;
        } else {
            // Load defaults from compiled-in TOML
            builder = Self::load_default_agents(builder).await?;
        }

        // Apply model configuration
        let multi_agent_system = builder.with_model_config(config.models.clone()).build();
        Ok(multi_agent_system)
    }

    /// Load default agents from compiled-in TOML configuration
    async fn load_default_agents(
        mut builder: MultiAgentSystemBuilder,
    ) -> Result<MultiAgentSystemBuilder> {
        // Parse the TOML directly
        let config_data: toml::Value = toml::from_str(DEFAULT_AGENTS_CONFIG)
            .map_err(|e| crate::error::ConfigError::ParseFailed { source: e })?;

        // Extract system prompt template if present
        if let Some(template) = config_data.get("system_prompt_template") {
            if let Some(content) = template.get("content").and_then(|v| v.as_str()) {
                builder = builder.with_system_prompt_template(content);
            }
        }

        // Extract pattern-specific prompt if present
        if let Some(pattern) = config_data.get("pattern_specific_prompt") {
            if let Some(content) = pattern.get("content").and_then(|v| v.as_str()) {
                builder = builder.with_pattern_specific_prompt(content);
            }
        }

        // Extract agents section
        if let Some(agents) = config_data.get("agents").and_then(|v| v.as_table()) {
            for (_key, agent_value) in agents {
                if let Ok(agent_toml) = agent_value
                    .clone()
                    .try_into::<agent::constellation::AgentConfigToml>()
                {
                    let agent_id = agent::AgentId::new(&agent_toml.id)?;

                    // Build agent config with persona
                    let config = AgentConfig {
                        id: agent_id,
                        name: agent_toml.name,
                        description: agent_toml.description,
                        system_prompt: agent_toml.persona, // Store persona in system_prompt for now
                        is_sleeptime: agent_toml.is_sleeptime,
                        sleeptime_interval: agent_toml.sleeptime_interval,
                    };

                    builder = builder.with_agent(config);
                }
            }
        }

        // Extract memory blocks section
        if let Some(memory) = config_data.get("memory").and_then(|v| v.as_table()) {
            for (_key, memory_value) in memory {
                if let Ok(memory_toml) = memory_value
                    .clone()
                    .try_into::<agent::constellation::MemoryBlockConfigToml>()
                {
                    let memory_id = crate::agent::MemoryBlockId::new(&memory_toml.name)?;

                    let config = MemoryBlockConfig {
                        name: memory_id,
                        max_length: memory_toml.max_length,
                        default_value: memory_toml.default_value,
                        description: memory_toml.description,
                    };

                    builder = builder.with_memory_block(config);
                }
            }
        }

        info!("Loaded default agents from compiled configuration");
        Ok(builder)
    }

    /// Start all configured services
    pub async fn start(self) -> Result<()> {
        // Start background monitors first
        let monitors = self.create_sleeptime_monitors().await?;

        // Convert to Arc for sharing between services
        let service = Arc::new(self);

        // Start background monitoring tasks
        for monitor in monitors {
            monitor.start().await?;
        }

        // Collect service handles
        #[allow(unused_mut)]
        let mut handles: Vec<tokio::task::JoinHandle<()>> = Vec::new();

        // Start Discord if configured
        #[cfg(feature = "discord")]
        {
            if let Some(handle) = service.clone().start_discord().await? {
                handles.push(handle);
            }
        }

        // Start MCP if configured
        #[cfg(feature = "mcp")]
        {
            if let Some(handle) = service.clone().start_mcp().await? {
                handles.push(handle);
            }
        }

        // Wait for services or just run background tasks
        if handles.is_empty() {
            info!("No services enabled. Running background tasks only.");

            // If no MCP server is running, we still need to initialize partners
            #[allow(unused_variables)]
            let mcp_enabled = false;
            #[cfg(feature = "mcp")]
            let mcp_enabled = service.config.mcp.enabled;

            if !mcp_enabled {
                if let Some(partners_config) = &service.config.partners {
                    info!("Initializing partners without MCP server...");
                    if let Err(e) =
                        Self::initialize_partners(&service.multi_agent_system, partners_config)
                            .await
                    {
                        error!("Failed to initialize partners: {:?}", e);
                    }
                }
            }

            service.wait_for_shutdown().await?
        } else {
            // Wait for all services
            for handle in handles {
                let _ = handle.await;
            }
        }

        Ok(())
    }

    /// Create sleeptime monitors for all users
    async fn create_sleeptime_monitors(&self) -> Result<Vec<Arc<SleeptimeMonitor>>> {
        let mut monitors = Vec::new();

        // Only create monitors if background tasks are enabled
        if !self.config.background.enabled {
            return Ok(monitors);
        }

        // Get all users to monitor
        let users = self.db.list_all_users().await?;

        // Create monitor for each user
        for user in users {
            if let Some(discord_id) = user.discord_id {
                let user_id = UserId(user.id);

                // Create sleeptime config from service configuration
                let sleeptime_config = crate::sleeptime::SleeptimeConfig {
                    tier1_interval_secs: self.config.background.sleeptime_interval_secs,
                    max_hyperfocus_minutes: 90,
                    max_sedentary_minutes: 120,
                    max_water_gap_minutes: 120,
                    min_energy_level: 4,
                    use_expensive_models: true,
                };

                // Create monitor
                let monitor = Arc::new(SleeptimeMonitor::new(
                    sleeptime_config,
                    self.db.clone(),
                    self.multi_agent_system.clone(),
                    user_id,
                ));

                monitors.push(monitor);

                info!(
                    "Created sleeptime monitor for user {} (Discord: {})",
                    user.name, discord_id
                );
            }
        }

        Ok(monitors)
    }

    /// Start Discord bot if configured
    #[cfg(feature = "discord")]
    async fn start_discord(self: Arc<Self>) -> Result<Option<tokio::task::JoinHandle<()>>> {
        use crate::discord::{create_discord_client, DiscordConfig};

        if self.config.discord.token.is_empty() {
            info!("Discord token not configured, skipping Discord bot");
            return Ok(None);
        }

        info!("Creating Discord client");

        let discord_config = DiscordConfig {
            token: self.config.discord.token.clone(),
            application_id: self.config.discord.application_id,
            channel_id: self.config.discord.channel_id,
            respond_to_dms: self.config.discord.respond_to_dms,
            respond_to_mentions: self.config.discord.respond_to_mentions,
            max_message_length: 2000,
        };

        let client = create_discord_client(
            &discord_config,
            self.multi_agent_system.clone(),
            self.db.clone(),
        )
        .await
        .map_err(|e| {
            PatternError::Config(crate::error::ConfigError::Invalid {
                field: "discord".to_string(),
                reason: format!("Failed to create Discord client: {}", e),
            })
        })?;
        let client_arc = Arc::new(tokio::sync::RwLock::new(client));

        info!("Starting Discord bot");
        let handle = tokio::spawn(async move {
            let mut client = client_arc.write().await;
            if let Err(e) = client.start().await {
                error!("Discord bot error: {:?}", e);
            }
        });

        Ok(Some(handle))
    }

    /// Start MCP server if configured
    #[cfg(feature = "mcp")]
    async fn start_mcp(self: Arc<Self>) -> Result<Option<tokio::task::JoinHandle<()>>> {
        use crate::mcp::server::PatternMcpServer;

        if !self.config.mcp.enabled {
            info!("MCP server disabled in configuration");
            return Ok(None);
        }

        info!("Starting MCP server on {}", self.config.mcp.transport);

        let server = PatternMcpServer::new(
            self.letta_client.clone(),
            self.db.clone(),
            self.multi_agent_system.clone(),
            #[cfg(feature = "discord")]
            self.discord_client.clone(),
        );

        let handle = match self.config.mcp.transport {
            McpTransport::Stdio => {
                // For stdio, we need to initialize partners before starting the server
                // since stdio doesn't have a configuration step with Letta
                let partners_config = self.config.partners.clone();
                let multi_agent_system = self.multi_agent_system.clone();

                tokio::spawn(async move {
                    // Initialize partners first for stdio
                    if let Some(partners_config) = &partners_config {
                        info!("Initializing partners for stdio MCP transport...");
                        if let Err(e) = PatternService::initialize_partners(
                            &multi_agent_system,
                            partners_config,
                        )
                        .await
                        {
                            error!("Failed to initialize partners: {:?}", e);
                        }
                    }

                    // Then run the stdio server
                    if let Err(e) = server.run_stdio().await {
                        error!("MCP server error: {:?}", e);
                    }
                })
            }
            McpTransport::Sse => {
                #[cfg(feature = "mcp-sse")]
                {
                    let port = self.config.mcp.port.unwrap_or(8080);
                    let multi_agent_system = self.multi_agent_system.clone();
                    let partners_config = self.config.partners.clone();

                    tokio::spawn(async move {
                        // Start the SSE server first
                        let server_handle = tokio::spawn(async move {
                            if let Err(e) = server.run_sse(port).await {
                                error!("MCP SSE server error: {:?}", e);
                            }
                        });

                        // Give the server time to start up
                        info!("Waiting for MCP SSE server to be ready...");
                        tokio::time::sleep(tokio::time::Duration::from_secs(3)).await;

                        // Configure Letta to use the SSE server
                        if let Err(e) = multi_agent_system.configure_mcp_server_sse(port).await {
                            error!("Failed to configure SSE MCP server in Letta: {:?}", e);
                        } else {
                            info!("Successfully configured SSE MCP server in Letta");

                            // Initialize partners if configured
                            if let Some(partners_config) = &partners_config {
                                if let Err(e) = PatternService::initialize_partners(
                                    &multi_agent_system,
                                    partners_config,
                                )
                                .await
                                {
                                    error!("Failed to initialize partners: {:?}", e);
                                }
                            }
                        }

                        // Wait for server to finish
                        let _ = server_handle.await;
                    })
                }
                #[cfg(not(feature = "mcp-sse"))]
                {
                    error!("MCP SSE transport requested but mcp-sse feature not enabled");
                    return Ok(None);
                }
            }
            McpTransport::Http => {
                let port = self.config.mcp.port.unwrap_or(8080);
                let multi_agent_system = self.multi_agent_system.clone();
                let partners_config = self.config.partners.clone();

                tokio::spawn(async move {
                    // Start the HTTP server first
                    let server_handle = tokio::spawn(async move {
                        if let Err(e) = server.run_http(port).await {
                            error!("MCP HTTP server error: {:?}", e);
                        }
                    });

                    // Give the server time to start up fully
                    info!("Waiting for MCP HTTP server to be ready...");
                    tokio::time::sleep(tokio::time::Duration::from_secs(3)).await;

                    // Then configure Letta to use it with retries
                    for attempt in 1..=3 {
                        info!(
                            "Attempting to configure MCP server in Letta (attempt {}/3)",
                            attempt
                        );
                        info!("Note: Letta can be slow to respond. This may take a moment...");

                        match multi_agent_system.configure_mcp_server(port).await {
                            Ok(_) => {
                                info!("Successfully configured MCP server in Letta");

                                // Initialize partners if configured
                                if let Some(partners_config) = &partners_config {
                                    if let Err(e) = PatternService::initialize_partners(
                                        &multi_agent_system,
                                        partners_config,
                                    )
                                    .await
                                    {
                                        error!("Failed to initialize partners: {:?}", e);
                                    }
                                }

                                break;
                            }
                            Err(e) => {
                                error!(
                                    "Failed to configure MCP server in Letta (attempt {}): {:?}",
                                    attempt, e
                                );
                                if attempt < 3 {
                                    // Wait longer between retries, especially for slow Letta responses
                                    let wait_time = 3 + attempt * 2; // 5s, 7s
                                    info!("Waiting {} seconds before retry...", wait_time);
                                    tokio::time::sleep(tokio::time::Duration::from_secs(wait_time))
                                        .await;
                                } else {
                                    // Don't fail the whole service if MCP registration fails
                                    // The server is still running and can be configured manually
                                    error!("Failed to configure MCP server after 3 attempts.");
                                    error!("This might be due to Letta being slow or having a backlog.");
                                    error!(
                                        "The MCP server is still running at http://localhost:{}",
                                        port
                                    );
                                    error!("You can manually configure it in Letta's ADE when it's ready.");
                                }
                            }
                        }
                    }

                    // Wait for the server to finish
                    let _ = server_handle.await;
                })
            }
        };

        Ok(Some(handle))
    }

    /// Initialize partner constellations at boot time
    async fn initialize_partners(
        multi_agent_system: &Arc<MultiAgentSystem>,
        partners_config: &PartnersConfig,
    ) -> Result<()> {
        info!("Initializing {} partners", partners_config.users.len());

        for partner in &partners_config.users {
            if partner.auto_initialize {
                info!(
                    "Auto-initializing partner: {} (Discord ID: {})",
                    partner.name, partner.discord_id
                );

                match multi_agent_system
                    .initialize_partner(&partner.discord_id, &partner.name)
                    .await
                {
                    Ok(user_id) => {
                        info!(
                            "Successfully initialized partner {} with user_id {}",
                            partner.name, user_id.0
                        );
                    }
                    Err(e) => {
                        error!(
                            "Failed to initialize partner {} ({}): {}",
                            partner.name, partner.discord_id, e
                        );
                        // Continue with other partners even if one fails
                    }
                }
            } else {
                info!("Skipping partner {} (auto_initialize=false)", partner.name);
            }
        }

        Ok(())
    }

    /// Wait for shutdown signal
    async fn wait_for_shutdown(&self) -> Result<()> {
        tokio::signal::ctrl_c().await.map_err(|e| {
            PatternError::Config(crate::error::ConfigError::Invalid {
                field: "signal".to_string(),
                reason: format!("Failed to install signal handler: {}", e),
            })
        })?;
        info!("Shutting down...");
        Ok(())
    }
}
