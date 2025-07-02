use clap::Parser;
use miette::Result;
use pattern::mcp_server::PatternServer;
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

    // Create server
    let service = PatternServer::new();

    // TODO: Initialize database connection
    // TODO: Initialize Letta client

    // Run server with stdio transport
    info!("Starting Pattern MCP server...");
    let transport = (stdin(), stdout());

    let server = service
        .serve(transport)
        .await
        .map_err(|e| miette::miette!("Failed to start server: {}", e))?;

    let quit_reason = server
        .waiting()
        .await
        .map_err(|e| miette::miette!("Server error: {}", e))?;

    info!("Server stopped: {:?}", quit_reason);
    Ok(())
}
