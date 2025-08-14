//! Discord bot integration
//!
//! This module handles Discord bot setup and integration with agent groups,
//! including endpoint configuration and bot lifecycle management.

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
    coordination::groups::GroupManager,
};
#[cfg(feature = "discord")]
use pattern_discord::{bot::DiscordBot, endpoints::DiscordEndpoint};
#[cfg(feature = "discord")]
use std::sync::Arc;
#[cfg(feature = "discord")]
use tokio::sync::RwLock;

#[cfg(feature = "discord")]
use crate::{
    agent_ops::load_model_embedding_providers,
    chat::{run_group_chat_loop, setup_group, GroupSetup},
    data_sources::get_bluesky_credentials,
    endpoints::CliEndpoint,
    output::Output,
};

/// Set up Discord endpoint for an agent if configured
#[cfg(feature = "discord")]
pub async fn setup_discord_endpoint(agent: &Arc<dyn Agent>, output: &Output) -> Result<()> {
    // Check if DISCORD_TOKEN is available
    let discord_token = match std::env::var("DISCORD_TOKEN") {
        Ok(token) => token,
        Err(_) => {
            // No Discord token configured
            return Ok(());
        }
    };

    output.status("Setting up Discord endpoint...");

    // Create Discord endpoint
    let mut discord_endpoint = DiscordEndpoint::new(discord_token);

    // Check for DISCORD_CHANNEL_ID (specific channel to listen to)
    if let Ok(channel_id) = std::env::var("DISCORD_CHANNEL_ID") {
        if let Ok(channel_id) = channel_id.parse::<u64>() {
            discord_endpoint = discord_endpoint.with_default_channel(channel_id);
            output.info("Listen channel:", &format!("{}", channel_id));
        } else {
            output.warning("Invalid DISCORD_CHANNEL_ID value");
        }
    }

    // Check if there's a default channel ID (fallback for messages without target)
    if let Ok(channel_id) = std::env::var("DISCORD_DEFAULT_CHANNEL") {
        if let Ok(channel_id) = channel_id.parse::<u64>() {
            // Only set if DISCORD_CHANNEL_ID wasn't already set
            if std::env::var("DISCORD_CHANNEL_ID").is_err() {
                discord_endpoint = discord_endpoint.with_default_channel(channel_id);
                output.info("Default channel:", &format!("{}", channel_id));
            }
        } else {
            output.warning("Invalid DISCORD_DEFAULT_CHANNEL value");
        }
    }

    // Check if there's a default DM user ID for CLI mode
    if let Ok(user_id) = std::env::var("DISCORD_DEFAULT_DM_USER") {
        if let Ok(user_id) = user_id.parse::<u64>() {
            discord_endpoint = discord_endpoint.with_default_dm_user(user_id);
            output.info("Default DM user:", &format!("{}", user_id));
        } else {
            output.warning("Invalid DISCORD_DEFAULT_DM_USER value");
        }
    }

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
pub async fn run_discord_bot_with_group<M: GroupManager + Clone + 'static>(
    group_name: &str,
    pattern_manager: M,
    model: Option<String>,
    no_tools: bool,
    config: &PatternConfig,
    enable_cli: bool,
) -> Result<()> {
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
    let group_setup = setup_group(
        group_name,
        &pattern_manager,
        model,
        no_tools,
        config,
        &output,
    )
    .await?;

    let GroupSetup {
        group,
        agents_with_membership,
        pattern_agent,
        agent_tools,
        constellation_tracker: _,
        heartbeat_sender: _,
        mut heartbeat_receiver,
    } = group_setup;

    output.success("Starting Discord bot with group chat...");
    output.info("Group:", &group.name.bright_cyan().to_string());
    output.info("Pattern:", &format!("{:?}", group.coordination_pattern));

    // Set up data sources if we have Pattern agent (similar to jetstream)
    if let Some(ref pattern_agent) = pattern_agent {
        if config.bluesky.is_some() {
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

                let mut data_sources = DataSourceBuilder::new()
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
                    .map_err(|e| miette::miette!("Failed to build data sources: {}", e))?;

                data_sources
                    .start_monitoring("bluesky_jetstream")
                    .await
                    .map_err(|e| miette::miette!("Failed to start monitoring: {}", e))?;

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
                    manager: Arc::new(pattern_manager.clone()),
                    output: output.clone(),
                });
                data_sources_router
                    .register_endpoint("group".to_string(), group_endpoint)
                    .await;

                // Register DataSourceTool on all agent tool registries
                let data_source_tool = DataSourceTool::new(Arc::new(RwLock::new(data_sources)));
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

        // Create Discord endpoint with token
        let mut discord_endpoint =
            pattern_discord::endpoints::DiscordEndpoint::new(discord_token.clone());

        // Configure default channel if set
        if let Ok(channel_id) = std::env::var("DISCORD_CHANNEL_ID") {
            if let Ok(channel_id) = channel_id.parse::<u64>() {
                discord_endpoint = discord_endpoint.with_default_channel(channel_id);
                output.info("Discord channel:", &format!("{}", channel_id));
            }
        }

        // Configure default DM user if set
        if let Ok(user_id) = std::env::var("DISCORD_DEFAULT_DM_USER") {
            if let Ok(user_id) = user_id.parse::<u64>() {
                discord_endpoint = discord_endpoint.with_default_dm_user(user_id);
                output.info("Discord DM user:", &format!("{}", user_id));
            }
        }

        // Don't create Arc yet, we'll update it with bot reference
        let mut discord_endpoint_base = discord_endpoint;

        // Create the Discord bot in CLI mode (wrapped in Arc for sharing)
        let bot = Arc::new(DiscordBot::new_cli_mode(
            agents_with_membership.clone(),
            group.clone(),
            Arc::new(pattern_manager.clone()),
        ));

        // Connect the bot to the Discord endpoint for timing context
        discord_endpoint_base = discord_endpoint_base.with_bot(bot.clone());
        let discord_endpoint = Arc::new(discord_endpoint_base);

        // Register on all agents in the group
        for awm in &agents_with_membership {
            awm.agent
                .register_endpoint("discord".to_string(), discord_endpoint.clone())
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
        let bot_for_handler = Arc::try_unwrap(bot.clone()).unwrap_or_else(|arc| (*arc).clone());
        let mut client_builder = Client::builder(&discord_token, intents).event_handler(bot_for_handler);

        // Set application ID if provided
        if let Ok(app_id) = std::env::var("APP_ID") {
            if let Ok(app_id_u64) = app_id.parse::<u64>() {
                client_builder = client_builder.application_id(app_id_u64.into());
                output.info("App ID:", &format!("Set to {}", app_id));
            } else {
                output.warning(&format!("Invalid APP_ID format: {}", app_id));
            }
        }

        let mut client = client_builder.await.into_diagnostic()?;

        // If CLI is enabled, spawn Discord bot in background and run CLI in foreground
        if enable_cli {
            output.status("Starting Discord bot in background...");
            output.status("CLI interface available. Type 'quit' or 'exit' to stop both.");

            // Spawn Discord bot in background
            let discord_handle = tokio::spawn(async move {
                if let Err(why) = client.start().await {
                    tracing::error!("Discord bot error: {:?}", why);
                }
            });

            // Run CLI chat loop in foreground
            if let Some(rl) = rl {
                run_group_chat_loop(
                    group.clone(),
                    agents_with_membership,
                    pattern_manager.clone(),
                    heartbeat_receiver,
                    output.clone(),
                    rl,
                )
                .await?;
            }

            // When CLI exits, also stop Discord bot
            discord_handle.abort();
        } else {
            // Run Discord bot in foreground (blocking)
            output.status("Discord bot starting... Press Ctrl+C to stop.");

            // Spawn a simple heartbeat consumer for Discord-only mode
            // The heartbeats are already processed internally by agents,
            // we just need to consume them to prevent the channel from blocking
            let heartbeat_handle = tokio::spawn(async move {
                while let Some(heartbeat) = heartbeat_receiver.recv().await {
                    tracing::debug!(
                        "💓 Consumed heartbeat in Discord-only mode from agent {}: tool {} (call_id: {})",
                        heartbeat.agent_id,
                        heartbeat.tool_name,
                        heartbeat.tool_call_id
                    );
                    // Heartbeats are processed internally by the agents during their tool execution
                    // We just consume them here to keep the channel flowing
                }
                tracing::debug!("Heartbeat receiver closed in Discord-only mode");
            });

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
