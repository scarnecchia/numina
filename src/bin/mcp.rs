use clap::Parser;
use miette::Result;
use pattern::{mcp_server::PatternServer, PatternService};
use rmcp::ServiceExt;
use tokio::io::{stdin, stdout};
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

    // Create Pattern service
    info!(
        "Initializing Pattern service with database: {}",
        args.db_path
    );
    let mut service = PatternService::new(&args.db_path).await?;

    // Initialize Letta client if URL or API key is provided
    if let Some(api_key) = args.letta_api_key {
        info!("Connecting to Letta cloud");
        let letta_client = letta::LettaClient::cloud(api_key)?;
        service = service.with_letta_client(letta_client);
        info!("Letta cloud client initialized");
    } else if let Some(letta_url) = args.letta_url {
        info!("Connecting to Letta server at: {}", letta_url);
        let letta_client = letta::LettaClient::builder().base_url(&letta_url).build()?;
        service = service.with_letta_client(letta_client);
        info!("Letta local client initialized");
    } else {
        info!("No Letta URL or API key provided, agent features will be disabled");
    }

    // Create MCP server
    let server = PatternServer::new(service);

    // Run server with stdio transport
    info!("Starting Pattern MCP server...");
    let transport = (stdin(), stdout());

    let mcp_server = server
        .serve(transport)
        .await
        .map_err(|e| miette::miette!("Failed to start server: {}", e))?;

    let quit_reason = mcp_server
        .waiting()
        .await
        .map_err(|e| miette::miette!("Server error: {}", e))?;

    info!("Server stopped: {:?}", quit_reason);
    Ok(())
}
