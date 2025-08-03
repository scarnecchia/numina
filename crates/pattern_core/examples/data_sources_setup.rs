//! Example configurations for setting up data sources with Pattern agents

use pattern_core::{
    agent::{Agent, DatabaseAgent},
    data_source::{
        BlueskyFilter, DataSourceBuilder, create_coordinator_with_agent_info,
        create_full_data_pipeline, create_knowledge_base, monitor_bluesky_mentions,
        monitor_directory,
    },
    embeddings::EmbeddingProvider,
    id::{AgentId, UserId},
    memory::Memory,
    model::ModelProvider,
    tool::ToolRegistry,
};
use std::sync::Arc;
use surrealdb::{Surreal, engine::local::SurrealKv};

// Use miette Result for examples
use miette::{IntoDiagnostic, Result};

/// Example 1: Simple file monitoring
///
/// Monitor a directory for changes and notify the agent when files are added/modified
async fn example_file_monitoring() -> Result<()> {
    // Setup database and agent
    let db = Surreal::new::<SurrealKv>("data/pattern.db")
        .await
        .into_diagnostic()?;
    db.use_ns("pattern")
        .use_db("main")
        .await
        .into_diagnostic()?;

    let agent_id = AgentId::generate();
    let agent_name = "FileMonitor".to_string();

    // Create a simple file monitoring data source
    let coordinator = monitor_directory(agent_id, agent_name, db, "/path/to/watch").await?;

    println!(
        "File monitoring coordinator created with {} sources",
        coordinator.list_sources().await.len()
    );

    Ok(())
}

/// Example 2: Knowledge base with semantic search
///
/// Create an indexed knowledge base from a directory of documents
async fn example_knowledge_base<E: EmbeddingProvider + 'static>(
    embedding_provider: Arc<E>,
) -> Result<()> {
    let db = Surreal::new::<SurrealKv>("data/pattern.db")
        .await
        .into_diagnostic()?;
    db.use_ns("pattern")
        .use_db("main")
        .await
        .into_diagnostic()?;

    let agent_id = AgentId::generate();
    let agent_name = "KnowledgeKeeper".to_string();

    // Create an indexed knowledge base
    let coordinator = create_knowledge_base(
        agent_id,
        agent_name,
        db,
        "/path/to/knowledge/docs",
        embedding_provider as Arc<dyn EmbeddingProvider>,
    )
    .await?;

    // Search within the knowledge base
    let results = coordinator
        .search_source(
            "file:/path/to/knowledge/docs",
            "ADHD management strategies",
            10,
        )
        .await?;

    println!("Found {} relevant documents", results.len());

    Ok(())
}

/// Example 3: Bluesky social media monitoring
///
/// Monitor Bluesky for mentions and relevant conversations
/// Note: In real usage, create a DatabaseAgent first and extract its handle
async fn example_bluesky_monitoring() -> Result<()> {
    let db = Surreal::new::<SurrealKv>("data/pattern.db")
        .await
        .into_diagnostic()?;
    db.use_ns("pattern")
        .use_db("main")
        .await
        .into_diagnostic()?;

    let agent_id = AgentId::generate();
    let agent_name = "SocialMonitor".to_string();

    // Monitor mentions of a specific handle
    // Note: In production, pass an agent_handle to enable enhanced notifications
    let _coordinator = monitor_bluesky_mentions(
        agent_id,
        agent_name,
        db,
        "pattern.bsky.social",
        None, // No agent handle in this simple example
    )
    .await?;

    println!("Bluesky monitoring active for @pattern.bsky.social");

    Ok(())
}

/// Example 4: Custom multi-source pipeline
///
/// Combine multiple data sources for a comprehensive agent setup
async fn example_custom_pipeline<E: EmbeddingProvider + 'static>(
    embedding_provider: Arc<E>,
) -> Result<()> {
    let db = Surreal::new::<SurrealKv>("data/pattern.db")
        .await
        .into_diagnostic()?;
    db.use_ns("pattern")
        .use_db("main")
        .await
        .into_diagnostic()?;

    let agent_id = AgentId::generate();
    let agent_name = "MultiSourceAgent".to_string();

    // Create a custom filter for Bluesky
    let bluesky_filter = BlueskyFilter {
        nsids: vec!["app.bsky.feed.post".to_string()],
        mentions: vec!["pattern.bsky.social".to_string()],
        keywords: vec!["ADHD".to_string(), "executive function".to_string()],
        languages: vec!["en".to_string()],
        ..Default::default()
    };

    // Build a comprehensive data pipeline
    let coordinator = DataSourceBuilder::new()
        // Static knowledge base
        .with_file_source("/data/knowledge", false, true)
        // Live document monitoring
        .with_file_source("/data/inbox", true, false)
        // Custom template for notifications
        .with_templated_file_source(
            "/data/reports",
            "/templates/report_notification.j2",
            true,
            false,
        )
        // Bluesky monitoring
        .with_bluesky_source(
            "adhd_conversations".to_string(),
            bluesky_filter.clone(),
            true,
        )
        // Custom Bluesky endpoint (e.g., for testing)
        .with_custom_bluesky_source(
            "test_firehose".to_string(),
            "wss://localhost:8080/firehose".to_string(),
            bluesky_filter,
            true,
        )
        .build(
            agent_id,
            agent_name,
            db,
            Some(embedding_provider as Arc<dyn EmbeddingProvider>),
            None, // No agent handle in this example
        )
        .await?;

    // List all active sources
    let sources = coordinator.list_sources().await;
    for (id, source_type) in sources {
        println!("Active source: {} ({})", id, source_type);
    }

    Ok(())
}

/// Example 5: Full agent with data sources
///
/// Create a complete agent with integrated data sources
async fn example_agent_with_data_sources<C, M, E>(
    db: Surreal<C>,
    model_provider: Arc<tokio::sync::RwLock<M>>,
    embedding_provider: Arc<E>,
) -> Result<()>
where
    C: surrealdb::Connection + Clone + std::fmt::Debug + 'static,
    M: ModelProvider + 'static,
    E: EmbeddingProvider + 'static,
{
    // Create the agent
    let agent = DatabaseAgent::new(
        AgentId::generate(),
        UserId::generate(),
        pattern_core::agent::AgentType::Generic,
        "DataAgent".to_string(),
        "You are an agent that monitors and responds to data from multiple sources.".to_string(),
        Memory::with_owner(&UserId::generate()),
        db.clone(),
        model_provider,
        ToolRegistry::new(),
        Some(embedding_provider.clone()),
        tokio::sync::mpsc::channel(100).0, // Heartbeat sender
        vec![],                            // Tool rules
    );

    // Set up data pipeline without agent handle
    let coordinator = create_full_data_pipeline(
        agent.id().clone(),
        agent.name().to_string(),
        db,
        "/data/knowledge".into(),
        "/data/watch".into(),
        BlueskyFilter {
            mentions: vec![agent.name().to_string()],
            ..Default::default()
        },
        embedding_provider as Arc<dyn EmbeddingProvider>,
        None, // No agent handle for this example
    )
    .await?;

    // Example: Manually read from a source
    let items = coordinator
        .read_source("file:/data/watch", 10, None)
        .await?;

    println!("Read {} items from watched directory", items.len());

    // Example: Pause/resume sources
    coordinator.pause_source("bluesky_firehose").await?;
    println!("Paused Bluesky monitoring");

    tokio::time::sleep(tokio::time::Duration::from_secs(60)).await;

    coordinator.resume_source("bluesky_firehose").await?;
    println!("Resumed Bluesky monitoring");

    Ok(())
}

/// Example 6: Dynamic source management
///
/// Add and remove data sources dynamically during runtime
async fn example_dynamic_sources() -> Result<()> {
    let db = Surreal::new::<SurrealKv>("data/pattern.db")
        .await
        .into_diagnostic()?;
    db.use_ns("pattern")
        .use_db("main")
        .await
        .into_diagnostic()?;

    let agent_id = AgentId::generate();
    let agent_name = "DynamicAgent".to_string();

    // Start with empty coordinator
    let mut coordinator = create_coordinator_with_agent_info(agent_id, agent_name, db, None);

    // Add sources dynamically based on conditions
    if std::env::var("MONITOR_DOCS").is_ok() {
        pattern_core::data_source::add_file_source(
            &mut coordinator,
            "/data/documents",
            true,
            false,
            None,
        )
        .await?;
        println!("Added document monitoring");
    }

    if std::env::var("BLUESKY_HANDLE").is_ok() {
        pattern_core::data_source::add_bluesky_source(
            &mut coordinator,
            "social_monitor".to_string(),
            None,
            BlueskyFilter::default(),
            None,
        )
        .await?;
        println!("Added Bluesky monitoring");
    }

    // Check buffer statistics
    if let Ok(stats) = coordinator.get_buffer_stats("social_monitor").await {
        println!(
            "Buffer stats: {}",
            serde_json::to_string_pretty(&stats).into_diagnostic()?
        );
    }

    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize tracing (comment out for examples)
    // tracing_subscriber::fmt::init();

    println!("Pattern Data Sources Examples");
    println!("=============================\n");

    // Run examples (commented out to avoid actual execution)
    // example_file_monitoring().await?;
    // example_knowledge_base(embedding_provider).await?;
    // example_bluesky_monitoring(agent_handle).await?;
    // example_custom_pipeline(embedding_provider, agent_handle).await?;
    // example_agent_with_data_sources(db, model, embeddings).await?;
    // example_dynamic_sources().await?;

    println!("\nExamples demonstrate various data source configurations.");
    println!("Uncomment specific examples to run them.");

    Ok(())
}
