//! Pattern - Multi-agent cognitive support system
//!
//! This is a temporary main.rs while we migrate from the monolithic structure.

use clap::Parser;
use miette::{IntoDiagnostic, Result};
use tracing::info;

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Path to configuration file
    #[arg(short, long, default_value = "pattern.toml")]
    config: String,

    /// Enable debug logging
    #[arg(short, long)]
    debug: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    // Parse command line arguments
    let args = Args::parse();

    // Initialize logging
    let filter = if args.debug { "debug" } else { "info" };
    tracing_subscriber::fmt().with_env_filter(filter).init();

    info!("Starting Pattern...");
    info!("Config file: {}", args.config);

    // TODO: Load configuration
    // TODO: Initialize SurrealDB
    // TODO: Create constellation
    // TODO: Start services based on features

    #[cfg(feature = "discord")]
    {
        info!("Discord support enabled");
        // TODO: Initialize Discord bot
    }

    #[cfg(feature = "mcp")]
    {
        info!("MCP server enabled");
        // TODO: Initialize MCP server
    }

    #[cfg(feature = "nd")]
    {
        info!("Neurodivergent features enabled");
        // TODO: Initialize ADHD-specific components
    }

    // Keep running until shutdown signal
    tokio::signal::ctrl_c().await.into_diagnostic()?;
    info!("Shutting down Pattern...");

    Ok(())
}
