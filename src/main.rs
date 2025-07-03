use miette::Result;
use pattern::{
    config::{self, Config},
    service::PatternService,
};
use tracing::info;

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging
    init_logging();

    info!("Starting Pattern - Multi-Agent ADHD Support System");

    // Load configuration
    config::load_dotenv();
    let config = Config::load()?;

    info!("Configuration loaded");

    // Initialize and start the service
    let service = PatternService::new(config).await?;
    service.start().await?;

    Ok(())
}

fn init_logging() {
    use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

    // Create logs directory if it doesn't exist
    std::fs::create_dir_all("logs").ok();

    // Create file appender
    let file_appender = tracing_appender::rolling::daily("logs", "pattern.log");
    let (non_blocking, _guard) = tracing_appender::non_blocking(file_appender);

    // Leak the guard to keep it alive for the entire program
    Box::leak(Box::new(_guard));

    // Set up subscribers
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "pattern=debug,letta=debug,discord=info".into()),
        )
        .with(
            // Console output - pretty and human-readable
            tracing_subscriber::fmt::layer()
                .with_target(false) // Don't show module paths for cleaner output
                .with_thread_ids(false)
                .with_line_number(false) // Less clutter
                .with_level(true)
                .with_timer(tracing_subscriber::fmt::time::time())
                .pretty(), // Pretty format with colors and indentation
        )
        .with(
            // File output
            tracing_subscriber::fmt::layer()
                .with_writer(non_blocking)
                .with_target(true)
                .with_thread_ids(true)
                .with_line_number(true)
                .with_ansi(false),
        )
        .init();
}
