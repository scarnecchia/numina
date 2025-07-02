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

    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "pattern=info,letta=info".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();
}
