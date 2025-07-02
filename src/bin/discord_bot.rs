use miette::{IntoDiagnostic, Result};
use pattern::{
    agents::MultiAgentSystemBuilder,
    config::{self, Config},
    db::Database,
    discord::{run_discord_bot, DiscordConfig},
};
use std::sync::Arc;

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging
    println!("Starting Pattern Discord Bot");

    // Load environment variables
    config::load_dotenv();

    // Load configuration
    let config = Config::load()?;
    config.validate()?;

    println!("Loaded configuration");

    // Initialize database
    let db = Database::new(&config.database.path).await?;
    db.migrate().await?;

    println!("Database initialized");

    // Initialize Letta client
    let letta_client = if let Some(api_key) = &config.letta.api_key {
        println!("Connecting to Letta cloud");
        letta::LettaClient::cloud(api_key)
    } else {
        println!("Connecting to Letta at {}", config.letta.base_url);
        // Create a ClientConfig from the base URL
        let client_config = letta::ClientConfig::new(&config.letta.base_url).into_diagnostic()?;
        letta::LettaClient::new(client_config)
    }
    .into_diagnostic()?;

    // Initialize multi-agent system
    let multi_agent_system =
        MultiAgentSystemBuilder::new(Arc::new(letta_client), Arc::new(db)).build();

    println!("Multi-agent system initialized");

    // Convert config to discord module format
    let discord_config = DiscordConfig {
        token: config.discord.token,
        channel_id: config.discord.channel_id,
        respond_to_dms: config.discord.respond_to_dms,
        respond_to_mentions: config.discord.respond_to_mentions,
        max_message_length: 2000,
    };

    // Run Discord bot
    if let Err(why) = run_discord_bot(discord_config, Arc::new(multi_agent_system)).await {
        eprintln!("Discord bot error: {:?}", why);
    }

    Ok(())
}
