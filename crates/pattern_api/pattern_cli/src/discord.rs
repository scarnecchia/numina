//! Discord bot integration
//!
//! This module handles Discord bot setup and integration with agent groups,
//! including endpoint configuration and bot lifecycle management.

#[cfg(feature = "discord")]
#[cfg(feature = "discord")]
use crate::{
    agent_ops::load_model_embedding_providers,
    chat::{GroupSetup, run_group_chat_loop, setup_group},
    data_sources::get_bluesky_credentials,
    endpoints::CliEndpoint,
    output::Output,
};
#[cfg(feature = "discord")]
use miette::{IntoDiagnostic, Result};
#[cfg(feature = "discord")]
use owo_colors::OwoColorize;
#[cfg(feature = "discord")]
use pattern_core::{
    Agent,
    config::PatternConfig,
    data_source::{BlueskyFilter, DataSourceBuilder},
    db::client::DB,
    tool::builtin::DataSourceTool,
};
#[cfg(feature = "discord")]
use pattern_discord::{
    bot::{DiscordBot, DiscordBotConfig, DiscordEventHandler},
    endpoints::DiscordEndpoint,
};
#[cfg(feature = "discord")]
use std::os::unix::process::CommandExt;
#[cfg(feature = "discord")]
use std::sync::Arc;

/// Set up Discord endpoint for an agent if configured
#[cfg(feature = "discord")]
pub async fn setup_discord_endpoint(
    agent: &Arc<dyn Agent>,
    cfg: &pattern_core::config::PatternConfig,
    output: &Output,
) -> Result<()> {
    // Check if DISCORD_TOKEN is available
    let discord_token = match std::env::var("DISCORD_TOKEN") {
        Ok(token) => token,
        Err(_) => {
            // No Discord token configured
            return Ok(());
        }
    };

    output.status("Setting up Discord endpoint...");

    // Create Discord endpoint with config (handles channels and DM users automatically)
    let discord_endpoint =
        DiscordEndpoint::with_config(discord_token.clone(), cfg.discord.as_ref());

    // Log the configuration if present
    if let Some(dc) = &cfg.discord {
        if let Some(chs) = &dc.allowed_channels {
            if let Some(first) = chs.first() {
                output.info("Listen channel:", first);
            }
        }
        if let Some(admins) = &dc.admin_users {
            if let Some(first) = admins.first() {
                output.info("Default DM user:", first);
            }
        }
    }

    // Note: Bot reference is not set here, so channel validation will be skipped
    // This means the endpoint will accept any channel ID

    // Display APP_ID and PUBLIC_KEY if configured (for reference)
    if let Ok(app_id) = std::env::var("APP_ID") {
        output.info("Discord App ID:", &app_id);
    }
    if let Ok(_) = std::env::var("PUBLIC_KEY") {
        output.info("Public key:", "✓ Configured");
    }

    // Register the endpoint
    agent
        .register_endpoint("discord".to_string(), Arc::new(discord_endpoint))
        .await?;

    output.success("Discord endpoint configured");

    Ok(())
}

/// Run a Discord bot for group chat with optional concurrent CLI interface
#[cfg(feature = "discord")]
pub async fn run_discord_bot_with_group(
    group_name: &str,
    model: Option<String>,
    no_tools: bool,
    config: &PatternConfig,
    enable_cli: bool,
) -> Result<()> {
    tracing::info!(
        "run_discord_bot_with_group called with group: {}, discord: true",
        group_name
    );
    use pattern_discord::serenity::{Client, all::GatewayIntents};
    use rustyline_async::Readline;

    // Create readline and output for concurrent CLI/Discord
    let (rl, writer) = if enable_cli {
        let (rl, writer) = Readline::new(format!("{} ", ">".bright_blue())).into_diagnostic()?;
        // Update the global tracing writer to use the SharedWriter
        crate::tracing_writer::set_shared_writer(writer.clone());
        (Some(rl), Some(writer))
    } else {
        (None, None)
    };

    // Create output with SharedWriter if CLI is enabled
    let output = if let Some(writer) = writer.clone() {
        Output::new().with_writer(writer)
    } else {
        Output::new()
    };

    // Check if Discord token is available
    let discord_token = match std::env::var("DISCORD_TOKEN") {
        Ok(token) => token,
        Err(_) => {
            output.error("DISCORD_TOKEN environment variable not set");
            return Err(miette::miette!("Discord token required to run bot"));
        }
    };

    // Use shared setup function
    let group_setup = setup_group(group_name, model, no_tools, config, &output).await?;

    let GroupSetup {
        group,
        agents_with_membership,
        supervisor_agent,
        agent_tools,
        pattern_manager,
        constellation_tracker: _,
        heartbeat_sender: _,
        heartbeat_receiver,
    } = group_setup;

    output.success("Starting Discord bot with group chat...");
    output.info("Group:", &group.name.bright_cyan().to_string());
    output.info("Pattern:", &format!("{:?}", group.coordination_pattern));

    // Set up data sources if we have a Supervisor agent (similar to jetstream)
    if let Some(ref pattern_agent) = supervisor_agent {
        tracing::info!("Discord group mode: Checking for Bluesky config");
        if config.bluesky.is_some() {
            tracing::info!("Discord group mode: Bluesky config found, setting up data sources");
            output.info("Bluesky:", "Setting up data source routing to group...");

            // Set up data sources with group as target
            let group_target = pattern_core::tool::builtin::MessageTarget {
                target_type: pattern_core::tool::builtin::TargetType::Group,
                target_id: Some(group.id.to_record_id()),
            };

            // Get the embedding provider
            let embedding_provider = if let Ok((_, embedding_provider, _)) =
                load_model_embedding_providers(None, config, None, true).await
            {
                embedding_provider
            } else {
                None
            };

            // Register data sources NOW (not in a spawn) since group endpoint is ready
            if let Some(embedding_provider) = embedding_provider {
                // Set up data sources synchronously (not in a spawn)
                let filter = config
                    .bluesky
                    .as_ref()
                    .and_then(|b| b.default_filter.as_ref())
                    .unwrap_or(&BlueskyFilter {
                        exclude_keywords: vec!["patternstop".to_string()],
                        ..Default::default()
                    })
                    .clone();

                tracing::info!("Discord: Building data sources with DataSourceBuilder");
                let data_sources = DataSourceBuilder::new()
                    .with_bluesky_source("bluesky_jetstream".to_string(), filter, true)
                    .build_with_target(
                        pattern_agent.id(),
                        pattern_agent.name(),
                        DB.clone(),
                        Some(embedding_provider),
                        Some(pattern_agent.handle().await),
                        get_bluesky_credentials(&config).await,
                        group_target,
                    )
                    .await
                    .map_err(|e| miette::miette!("Failed to build data sources: {:?}", e))?;

                // NOTE: Monitoring is already started during agent loading via register_data_sources
                // Calling start_monitoring again here can cause write lock contention and deadlocks
                // Just verify it's running instead
                tracing::info!(
                    "Discord: Data sources built, monitoring should already be active from agent loading"
                );
                output.success("Data sources configured for group");

                // Register endpoints on the data source's router
                let data_sources_router = data_sources.router();

                // Register the CLI endpoint as default user endpoint
                data_sources_router
                    .register_endpoint(
                        "user".to_string(),
                        Arc::new(CliEndpoint::new(output.clone())),
                    )
                    .await;

                // Register the group endpoint so it can route to the group
                let group_endpoint = Arc::new(crate::endpoints::GroupCliEndpoint {
                    group: group.clone(),
                    agents: agents_with_membership.clone(),
                    manager: pattern_manager.clone(),
                    output: output.clone(),
                });
                data_sources_router
                    .register_endpoint("group".to_string(), group_endpoint)
                    .await;

                // Register DataSourceTool on all agent tool registries
                let data_source_tool = DataSourceTool::new(Arc::new(data_sources));
                for tools in &agent_tools {
                    tools.register(data_source_tool.clone());
                }
            }
        }
    }

    // Register Discord endpoint on all agents for Discord responses
    #[cfg(feature = "discord")]
    {
        output.status("Registering Discord endpoint on agents...");

        // Create Discord endpoint with config (handles channels and DM users automatically)
        let discord_endpoint = pattern_discord::endpoints::DiscordEndpoint::with_config(
            discord_token.clone(),
            config.discord.as_ref(),
        );

        // Log the configuration if present
        if let Some(dc) = &config.discord {
            if let Some(chs) = &dc.allowed_channels {
                if let Some(first) = chs.first() {
                    output.info("Discord channel:", first);
                }
            }
            if let Some(admins) = &dc.admin_users {
                if let Some(first) = admins.first() {
                    output.info("Discord DM user:", first);
                }
            }
        }

        // Don't create Arc yet, we'll update it with bot reference
        let mut discord_endpoint_base = discord_endpoint;

        // Create Discord bot config from PatternConfig (with env var fallback)
        let bot_cfg = if let Some(dc) = &config.discord {
            DiscordBotConfig::from_config(
                std::env::var("DISCORD_TOKEN").expect("DISCORD_TOKEN must be set"),
                dc,
            )
        } else {
            DiscordBotConfig::new(
                std::env::var("DISCORD_TOKEN").expect("DISCORD_TOKEN must be set"),
            )
        };

        // Create the Discord bot in CLI mode (wrapped in Arc for sharing)
        // Build forward sinks so CLI can mirror Discord stream (and optional file sink)
        let sinks =
            crate::forwarding::build_discord_group_sinks(&output, &agents_with_membership).await;

        let (restart_tx, mut restart_rx) = tokio::sync::mpsc::channel(1);

        let bot = Arc::new(DiscordBot::new_cli_mode(
            bot_cfg,
            agents_with_membership.clone(),
            group.clone(),
            pattern_manager.clone(),
            Some(sinks),
            restart_tx.clone(),
        ));

        // Connect the bot to the Discord endpoint for timing context
        discord_endpoint_base = discord_endpoint_base.with_bot(bot.clone());
        let discord_endpoint = Arc::new(discord_endpoint_base);

        // Register on all agents in the group
        for awm in &agents_with_membership {
            awm.agent
                .register_endpoint(
                    "cli".to_string(),
                    Arc::new(CliEndpoint::new(output.clone())),
                )
                .await?;
            awm.agent
                .set_default_user_endpoint(discord_endpoint.clone())
                .await?;
        }
        output.success("✓ Discord endpoint registered on all agents");

        // Build Discord client
        let intents = GatewayIntents::GUILD_MESSAGES
            | GatewayIntents::DIRECT_MESSAGES
            | GatewayIntents::MESSAGE_CONTENT
            | GatewayIntents::GUILDS
            | GatewayIntents::GUILD_MESSAGE_REACTIONS
            | GatewayIntents::DIRECT_MESSAGE_REACTIONS;

        // Extract bot from Arc - we need to consume it for event_handler
        // But we already gave it to the endpoint, so we need to clone the Arc's contents
        let bot_for_handler = DiscordEventHandler::new(bot.clone());
        let mut client_builder =
            Client::builder(&discord_token, intents).event_handler(bot_for_handler);

        // Set application ID if provided
        if let Ok(app_id) = std::env::var("APP_ID") {
            if let Ok(app_id_u64) = app_id.parse::<u64>() {
                client_builder = client_builder.application_id(app_id_u64.into());
                output.info("App ID:", &format!("Set to {}", app_id));
            } else {
                output.warning(&format!("Invalid APP_ID format: {}", app_id));
            }
        }

        // If CLI is enabled, spawn Discord bot in background and run CLI in foreground
        if enable_cli {
            output.status("Starting Discord bot in background...");
            output.status("CLI interface available. Type 'quit' or 'exit' to stop both.");

            // Spawn Discord bot in background
            let discord_handle = tokio::spawn(async move {
                let mut client = client_builder.await.unwrap();
                if let Err(why) = client.start().await {
                    tracing::error!("Discord bot error: {:?}", why);
                }
            });

            tokio::spawn(async move {
                restart_rx.recv().await;
                tracing::info!("restart signal received");
                let _ = crossterm::terminal::disable_raw_mode();

                let exe = std::env::current_exe().unwrap();
                let args: Vec<String> = std::env::args().collect();

                let _ = std::process::Command::new(exe).args(&args[1..]).exec();

                std::process::exit(0);
            });

            // Run CLI chat loop in foreground
            if let Some(rl) = rl {
                run_group_chat_loop(
                    group.clone(),
                    agents_with_membership,
                    pattern_manager,
                    heartbeat_receiver,
                    output.clone(),
                    rl,
                )
                .await?;
            }

            // When CLI exits, also stop Discord bot
            discord_handle.abort();

            // Force exit the entire process to ensure all spawned tasks are killed
            // This is a bit heavy-handed but ensures clean shutdown
            std::process::exit(0);
        } else {
            let mut client = client_builder.await.into_diagnostic()?;
            // Run Discord bot in foreground (blocking)
            output.status("Discord bot starting... Press Ctrl+C to stop.");

            // Use generic heartbeat processor for Discord-only mode
            let agents_for_heartbeat: Vec<Arc<dyn Agent>> = agents_with_membership
                .iter()
                .map(|awm| awm.agent.clone())
                .collect();

            let output_clone = output.clone();
            let heartbeat_handle =
                tokio::spawn(pattern_core::context::heartbeat::process_heartbeats(
                    heartbeat_receiver,
                    agents_for_heartbeat,
                    move |event, _agent_id, agent_name| {
                        let output = output_clone.clone();
                        async move {
                            output
                                .status(&format!("💓 Heartbeat continuation from {}:", agent_name));
                            crate::chat::print_response_event(event, &output);
                        }
                    },
                ));

            // Run Discord bot
            if let Err(why) = client.start().await {
                output.error(&format!("Discord bot error: {:?}", why));
                heartbeat_handle.abort(); // Clean up heartbeat task
                return Err(miette::miette!("Failed to run Discord bot"));
            }

            // Clean up heartbeat task when Discord bot exits
            heartbeat_handle.abort();
        }
    }

    Ok(())
}

#[cfg(not(feature = "discord"))]
/// Placeholder when Discord feature is not enabled
pub fn discord_not_available() {
    let output = crate::output::Output::new();
    output.error("Discord support not compiled in. Enable the 'discord' feature.");
}
