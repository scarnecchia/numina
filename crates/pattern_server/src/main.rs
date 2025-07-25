//! Pattern API Server
//!
//! Unified backend providing HTTP/WebSocket APIs, MCP integration, and Discord bot

use miette::IntoDiagnostic;
use pattern_server::{ServerConfig, start_server};
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> miette::Result<()> {
    // Load .env file if it exists
    //let _ = dotenvy::dotenv();
    miette::set_hook(Box::new(|_| {
        Box::new(
            miette::MietteHandlerOpts::new()
                .terminal_links(true)
                .rgb_colors(miette::RgbColors::Preferred)
                .with_cause_chain()
                .with_syntax_highlighting(miette::highlighters::SyntectHighlighter::default())
                .color(true)
                .context_lines(5)
                .tab_width(2)
                .break_words(true)
                .build(),
        )
    }))?;
    miette::set_panic_hook();
    // Initialize tracing
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::new(
            "pattern_api=debug,pattern_server=debug,pattern_core=debug,pattern_cli=debug,pattern_nd=debug,pattern_mcp=debug,pattern_discord=debug,pattern_main=debug",
        ))
        .with_file(true)
        .with_line_number(true) // Show target module
        .with_thread_ids(false)
        .with_thread_names(false)
        .with_timer(tracing_subscriber::fmt::time::LocalTime::rfc_3339()) // Local time in RFC 3339 format
        .pretty()
        .init();

    // Load config (for now use defaults)
    let config = ServerConfig::default();

    // Start server
    start_server(config).await.into_diagnostic()?;

    Ok(())
}
