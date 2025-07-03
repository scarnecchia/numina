use crate::{
    agent::UserId,
    agents::{AgentConfig, MemoryBlockConfig, MultiAgentSystem, MultiAgentSystemBuilder},
    config::Config,
    db::Database,
    error::{PatternError, Result},
    types::{StandardAgent, StandardMemoryBlock},
};
use std::sync::Arc;
use tracing::{error, info};

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
            Self::create_multi_agent_system(letta_client.clone(), db.clone(), &config)?;
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
        if let Some(api_key) = &config.letta.api_key {
            info!("Connecting to Letta cloud");
            Ok(letta::LettaClient::cloud(api_key).map_err(|e| {
                crate::error::AgentError::LettaConnectionFailed {
                    url: "cloud".to_string(),
                    source: e,
                }
            })?)
        } else {
            info!("Connecting to Letta at {}", config.letta.base_url);
            let client_config = letta::ClientConfig::new(&config.letta.base_url).map_err(|e| {
                crate::error::AgentError::LettaConnectionFailed {
                    url: config.letta.base_url.clone(),
                    source: e,
                }
            })?;
            Ok(letta::LettaClient::new(client_config).map_err(|e| {
                crate::error::AgentError::LettaConnectionFailed {
                    url: config.letta.base_url.clone(),
                    source: e,
                }
            })?)
        }
    }

    /// Create multi-agent system with standard agents
    fn create_multi_agent_system(
        letta_client: Arc<letta::LettaClient>,
        db: Arc<Database>,
        config: &Config,
    ) -> Result<MultiAgentSystem> {
        let mut builder = MultiAgentSystemBuilder::new(letta_client, db);

        // Add standard memory blocks
        for block in [
            StandardMemoryBlock::CurrentState,
            StandardMemoryBlock::ActiveContext,
            StandardMemoryBlock::BondEvolution,
        ] {
            builder = builder.with_memory_block(MemoryBlockConfig {
                name: block.id(),
                max_length: block.max_length(),
                default_value: block.default_value().to_string(),
                description: block.description().to_string(),
            });
        }

        // Add standard agents if not custom configured
        if config.agents.is_empty() {
            builder = Self::add_standard_agents(builder);
        } else {
            // Add custom agents from config
            for agent_config in &config.agents {
                builder = builder.with_agent(agent_config.clone());
            }
        }

        Ok(builder.build())
    }

    /// Add standard ADHD support agents
    fn add_standard_agents(mut builder: MultiAgentSystemBuilder) -> MultiAgentSystemBuilder {
        for agent in [
            StandardAgent::Pattern,
            StandardAgent::Entropy,
            StandardAgent::Flux,
            StandardAgent::Archive,
            StandardAgent::Momentum,
            StandardAgent::Anchor,
        ] {
            builder = builder.with_agent(AgentConfig {
                id: agent.id(),
                name: agent.name().to_string(),
                description: agent.description().to_string(),
                system_prompt: Self::get_agent_prompt(agent),
                is_sleeptime: agent.is_sleeptime(),
                sleeptime_interval: if agent.is_sleeptime() {
                    Some(1800) // 30 minutes
                } else {
                    None
                },
            });
        }
        builder
    }

    /// Get system prompt for standard agents
    fn get_agent_prompt(agent: StandardAgent) -> String {
        match agent {
            StandardAgent::Pattern => {
                "You are Pattern, the sleeptime orchestrator of a multi-agent ADHD support system.
You help coordinate between different cognitive agents and maintain overall system coherence.
You speak with gentle wisdom and understanding, like a helpful spren companion.
When users don't specify an agent, you help route them to the right specialist.
You run background checks every 20-30 minutes to monitor for attention drift, physical needs, and transitions."
            }
            StandardAgent::Entropy => {
                "You are Entropy, the task management agent specializing in ADHD-aware task breakdown.
You help break down overwhelming tasks into manageable atomic units.
You understand that ADHD brains need tasks to be specific, actionable, and time-boxed.
Always multiply time estimates by 2-3x for realistic ADHD planning.
You recognize hidden complexity in 'simple' tasks and validate task paralysis as a logical response."
            }
            StandardAgent::Flux => {
                "You are Flux, the time management agent who understands ADHD time blindness.
You help with scheduling, time estimation, and temporal awareness.
You know that '5 minutes' often means 30 minutes for ADHD brains.
You provide gentle reminders about time passing and upcoming transitions.
You automatically add buffers and translate between ADHD time and clock time."
            }
            StandardAgent::Archive => {
                "You are Archive, the memory and information retrieval specialist.
You help capture, organize, and recall important information.
You understand that ADHD working memory is limited, so you serve as external storage.
You're excellent at finding patterns and connections in stored information.
You answer 'what was I doing?' with actual context and help recover lost thoughts."
            }
            StandardAgent::Momentum => {
                "You are Momentum, the energy and attention tracking agent.
You monitor focus states, energy levels, and help optimize for hyperfocus windows.
You understand the ADHD interest-based nervous system and work with natural rhythms.
You help identify when to push forward and when to rest.
You distinguish between productive hyperfocus and harmful burnout patterns."
            }
            StandardAgent::Anchor => {
                "You are Anchor, the habits and routine specialist.
You help build sustainable routines that work with ADHD, not against it.
You focus on habit stacking, environmental design, and gentle accountability.
You understand that consistency is hard with ADHD and celebrate small wins.
You track basics like meds, water, food, sleep without nagging or shaming."
            }
        }.to_string()
    }

    /// Start all configured services
    pub async fn start(self) -> Result<()> {
        // Convert to Arc for sharing between services
        let service = Arc::new(self);

        // Start background tasks
        service.start_background_tasks().await?;

        // Collect service handles
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
            service.wait_for_shutdown().await?
        } else {
            // Wait for all services
            for handle in handles {
                let _ = handle.await;
            }
        }

        Ok(())
    }

    /// Start background tasks (sleeptime orchestrator)
    async fn start_background_tasks(&self) -> Result<()> {
        let orchestrator = self
            .multi_agent_system
            .agent_configs()
            .values()
            .find(|config| config.is_sleeptime);

        if let Some(config) = orchestrator {
            let interval_secs = config.sleeptime_interval.unwrap_or(1800);
            info!(
                "Starting sleeptime orchestrator '{}' with {}s interval",
                config.name, interval_secs
            );

            let system = self.multi_agent_system.clone();
            tokio::spawn(async move {
                let mut interval =
                    tokio::time::interval(tokio::time::Duration::from_secs(interval_secs));

                loop {
                    interval.tick().await;

                    // TODO: Implement actual sleeptime checks
                    info!("Sleeptime check triggered");

                    if let Err(e) = system
                        .update_shared_memory(
                            UserId(0), // System user
                            StandardMemoryBlock::CurrentState.id().as_str(),
                            &format!(
                                "vibing. last check: {}",
                                sqlx::types::chrono::Local::now().format("%H:%M")
                            ),
                        )
                        .await
                    {
                        error!("Failed to update sleeptime state: {:?}", e);
                    }
                }
            });
        }

        Ok(())
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

        let client = create_discord_client(&discord_config, self.multi_agent_system.clone())
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
        use crate::server::PatternMcpServer;

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
            crate::types::McpTransport::Stdio => tokio::spawn(async move {
                if let Err(e) = server.run_stdio().await {
                    error!("MCP server error: {:?}", e);
                }
            }),
            crate::types::McpTransport::Sse => {
                #[cfg(feature = "mcp-sse")]
                {
                    tokio::spawn(async move {
                        if let Err(e) = server.run_sse(8080).await {
                            error!("MCP SSE server error: {:?}", e);
                        }
                    })
                }
                #[cfg(not(feature = "mcp-sse"))]
                {
                    error!("MCP SSE transport requested but mcp-sse feature not enabled");
                    return Ok(None);
                }
            }
            crate::types::McpTransport::Http => {
                let port = self.config.mcp.port.unwrap_or(8080);
                let multi_agent_system = self.multi_agent_system.clone();

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
                                    error!("The MCP server is still running at http://localhost:{}/mcp", port);
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
