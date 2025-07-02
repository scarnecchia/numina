use clap::Parser;
use miette::Result;
use pattern::{agents::MultiAgentSystemBuilder, db::Database, server::PatternMcpServer};

use tracing::info;
use tracing_subscriber::EnvFilter;

#[derive(Parser, Debug)]
#[clap(author = "Orual", version, about)]
/// Pattern MCP server for Letta agent management
struct Args {
    /// Enable verbose logging
    #[arg(short = 'v', long)]
    verbose: bool,

    /// Database path (defaults to ./data/pattern.db)
    #[arg(long, default_value = "./data/pattern.db")]
    db_path: String,

    /// Letta server URL (if not using local client)
    #[arg(long)]
    letta_url: Option<String>,

    /// Letta API key (if using cloud deployment)
    #[arg(long)]
    letta_api_key: Option<String>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    // Initialize tracing
    let filter = if args.verbose {
        EnvFilter::new("debug")
    } else {
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"))
    };

    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .with_thread_ids(false)
        .with_file(false)
        .init();

    // Initialize database
    info!("Initializing database at: {}", args.db_path);
    let db = Database::new(&args.db_path).await?;
    db.migrate().await?;
    let db = std::sync::Arc::new(db);
    info!("Database initialized");

    // Initialize Letta client
    let letta_client = if let Some(api_key) = args.letta_api_key {
        info!("Connecting to Letta cloud");
        letta::LettaClient::cloud(api_key)?
    } else if let Some(letta_url) = args.letta_url {
        info!("Connecting to Letta server at: {}", letta_url);
        let client_config = letta::ClientConfig::new(&letta_url)?;
        letta::LettaClient::new(client_config)?
    } else {
        info!("Connecting to default Letta server at localhost:8000");
        letta::LettaClient::local()?
    };
    let letta_client = std::sync::Arc::new(letta_client);
    info!("Letta client initialized");

    // Initialize multi-agent system
    let multi_agent_system =
        std::sync::Arc::new(MultiAgentSystemBuilder::new(letta_client.clone(), db.clone()).build());
    info!("Multi-agent system initialized");

    // Create MCP server
    let server = PatternMcpServer::new(
        letta_client,
        db,
        multi_agent_system,
        #[cfg(feature = "discord")]
        None,
    );

    // Run server
    info!("Starting Pattern MCP server...");
    server.run_stdio().await
}
