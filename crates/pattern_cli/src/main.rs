use clap::{Parser, Subcommand};
use colored::Colorize;
use miette::{IntoDiagnostic, Result};
use pattern_core::{
    ModelProvider,
    agent::{AgentRecord, AgentState, AgentType, DatabaseAgent},
    db::{
        DatabaseConfig,
        client::{self, DB},
        ops,
    },
    id::{AgentId, UserId},
    memory::Memory,
    model::GenAiClient,
    tool::ToolRegistry,
};
use std::path::PathBuf;
use std::sync::Arc;
use surrealdb::RecordId;
use tokio::sync::RwLock;
use tracing::info;

#[derive(Parser)]
#[command(name = "pattern-cli")]
#[command(about = "Pattern ADHD Support System CLI")]
#[command(version)]
struct Cli {
    #[command(subcommand)]
    command: Commands,

    /// Database file path
    #[arg(long, default_value = "pattern-cli.db")]
    db_path: PathBuf,

    /// Enable debug logging
    #[arg(long)]
    debug: bool,
}

#[derive(Subcommand)]
enum Commands {
    /// Interactive chat with agents
    Chat {
        /// Agent name to chat with
        #[arg(long, default_value = "Pattern")]
        agent: String,

        /// Model to use (e.g. gpt-4o, claude-3-haiku)
        #[arg(long)]
        model: Option<String>,

        /// Enable tool usage
        #[arg(long)]
        tools: bool,
    },
    /// Agent management
    Agent {
        #[command(subcommand)]
        cmd: AgentCommands,
    },
    /// Database inspection
    Db {
        #[command(subcommand)]
        cmd: DbCommands,
    },
}

#[derive(Subcommand)]
enum AgentCommands {
    /// List all agents
    List,
    /// Create a new agent
    Create {
        name: String,
        #[arg(long)]
        agent_type: Option<String>,
    },
    /// Show agent status
    Status { name: String },
}

#[derive(Subcommand)]
enum DbCommands {
    /// Show database stats
    Stats,
    /// Run a query
    Query { sql: String },
}

/// Format agent state for display
fn format_agent_state(state: &AgentState) -> String {
    match state {
        AgentState::Ready => "Ready".bright_green().to_string(),
        AgentState::Processing => "Processing".bright_yellow().to_string(),
        AgentState::Cooldown { until } => format!("Cooldown until {}", until.format("%H:%M:%S"))
            .yellow()
            .to_string(),
        AgentState::Suspended => "Suspended".bright_red().to_string(),
        AgentState::Error => "Error".red().bold().to_string(),
    }
}

/// Format a timestamp as relative time
fn format_relative_time(time: chrono::DateTime<chrono::Utc>) -> String {
    let now = chrono::Utc::now();
    let duration = now.signed_duration_since(time);

    if duration.num_seconds() < 60 {
        format!("{} seconds ago", duration.num_seconds())
            .dimmed()
            .to_string()
    } else if duration.num_minutes() < 60 {
        format!("{} minutes ago", duration.num_minutes())
            .dimmed()
            .to_string()
    } else if duration.num_hours() < 24 {
        format!("{} hours ago", duration.num_hours())
            .dimmed()
            .to_string()
    } else if duration.num_days() < 30 {
        format!("{} days ago", duration.num_days())
            .dimmed()
            .to_string()
    } else {
        time.format("%Y-%m-%d").to_string().dimmed().to_string()
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    dotenvy::dotenv().into_diagnostic()?;
    miette::set_hook(Box::new(|_| {
        Box::new(
            miette::MietteHandlerOpts::new()
                .terminal_links(true)
                //.rgb_colors(miette::RgbColors::)
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
    let cli = Cli::parse();

    // Initialize tracing
    use tracing_subscriber::{EnvFilter, fmt};

    let filter = if cli.debug {
        // Only show debug output from pattern crates
        EnvFilter::new(
            "pattern_core=debug,pattern_cli=debug,pattern_nd=debug,pattern_mcp=debug,pattern_discord=debug,pattern_main=debug",
        )
    } else {
        // Show info level for pattern crates, warn for everything else
        EnvFilter::new(
            "pattern_core=info,pattern_cli=info,pattern_nd=info,pattern_mcp=info,pattern_discord=info,pattern_main=info,warn",
        )
    };

    fmt().with_env_filter(filter).init();

    info!("Starting pattern-cli with database: {:?}", cli.db_path);

    // Initialize database
    let db_config = DatabaseConfig::Embedded {
        path: cli.db_path.to_string_lossy().to_string(),
        strict_mode: false,
    };

    info!("Initializing database...");
    client::init_db(db_config).await?;
    info!("Database initialized successfully");

    match cli.command {
        Commands::Chat {
            agent,
            model,
            tools,
        } => {
            println!("{}", "Starting chat mode...".bright_green());
            println!("Agent: {}", agent.bright_cyan());
            if let Some(model_name) = &model {
                println!("Model: {}", model_name.bright_yellow());
            }
            if tools {
                println!("Tools: {}", "enabled".bright_green());
            }

            // Try to load existing agent or create new one
            let agent = load_or_create_agent(&agent, model, tools).await?;
            chat_with_agent(agent).await?;
        }
        Commands::Agent { cmd } => match cmd {
            AgentCommands::List => {
                let agents = ops::list_entities::<AgentRecord, _>(&DB).await?;

                if agents.is_empty() {
                    println!("{}", "No agents found".dimmed());
                    println!(
                        "Create an agent with: {} agent create <name>",
                        "pattern-cli".bright_green()
                    );
                } else {
                    println!(
                        "{}",
                        format!("Found {} agent(s):", agents.len()).bright_green()
                    );
                    println!();

                    for agent in agents {
                        println!("{} {}", "•".bright_blue(), agent.name.bright_cyan());
                        println!("  {} {}", "ID:".dimmed(), agent.id.to_string().dimmed());
                        println!(
                            "  {} {}",
                            "Type:".dimmed(),
                            format!("{:?}", agent.agent_type).bright_yellow()
                        );
                        println!(
                            "  {} {}",
                            "State:".dimmed(),
                            format_agent_state(&agent.state)
                        );
                        println!(
                            "  {} {} messages, {} tool calls",
                            "Stats:".dimmed(),
                            agent.total_messages.to_string().bright_white(),
                            agent.total_tool_calls.to_string().bright_white()
                        );
                        println!(
                            "  {} {}",
                            "Last active:".dimmed(),
                            format_relative_time(agent.last_active)
                        );
                        println!();
                    }
                }
            }
            AgentCommands::Create { name, agent_type } => {
                println!(
                    "{} {}",
                    "Creating agent:".bright_green(),
                    name.bright_cyan()
                );

                // Parse agent type
                let parsed_type = if let Some(type_str) = agent_type {
                    match type_str.parse::<AgentType>() {
                        Ok(t) => t,
                        Err(_) => {
                            println!(
                                "{} Unknown agent type '{}', using 'generic'",
                                "Warning:".yellow(),
                                type_str
                            );
                            AgentType::Generic
                        }
                    }
                } else {
                    AgentType::Generic
                };

                // Create agent record
                let user_id = UserId::generate(); // TODO: Get from config or auth
                let now = chrono::Utc::now();

                let agent = AgentRecord {
                    id: AgentId::generate(),
                    name: name.clone(),
                    agent_type: parsed_type.clone(),
                    state: AgentState::Ready,
                    base_instructions: format!(
                        "You are {}, a {} agent in the Pattern ADHD support system.",
                        name,
                        parsed_type.as_str()
                    ),
                    owner_id: user_id,
                    created_at: now,
                    updated_at: now,
                    last_active: now,
                    ..Default::default()
                };

                // Save to database using store_with_relations since AgentRecord has relations
                match agent.store_with_relations(&DB).await {
                    Ok(stored_agent) => {
                        println!();
                        println!("{} Created agent successfully!", "✓".bright_green());
                        println!();
                        println!("{} {}", "Name:".dimmed(), stored_agent.name.bright_cyan());
                        println!(
                            "{} {}",
                            "ID:".dimmed(),
                            stored_agent.id.to_string().dimmed()
                        );
                        println!(
                            "{} {}",
                            "Type:".dimmed(),
                            format!("{:?}", stored_agent.agent_type).bright_yellow()
                        );
                        println!();
                        println!(
                            "Start chatting with: {} chat --agent {}",
                            "pattern-cli".bright_green(),
                            name
                        );
                    }
                    Err(e) => {
                        println!("{} Failed to create agent: {}", "Error:".bright_red(), e);
                    }
                }
            }
            AgentCommands::Status { name } => {
                // Query for the agent by name
                let query = "SELECT * FROM agent WHERE name = $name LIMIT 1";
                let mut response = DB
                    .query(query)
                    .bind(("name", name.to_string()))
                    .await
                    .into_diagnostic()?;

                let agents: Vec<AgentRecord> = response.take(0).into_diagnostic()?;

                if let Some(agent) = agents.first() {
                    println!("{} Agent Status", "📊".bright_blue());
                    println!("{}", "─".repeat(40).dimmed());
                    println!();

                    // Basic info
                    println!(
                        "{} {}",
                        "Name:".bright_cyan(),
                        agent.name.bright_white().bold()
                    );
                    println!("{} {}", "ID:".bright_cyan(), agent.id.to_string().dimmed());
                    println!(
                        "{} {}",
                        "Type:".bright_cyan(),
                        format!("{:?}", agent.agent_type).bright_yellow()
                    );
                    println!(
                        "{} {}",
                        "State:".bright_cyan(),
                        format_agent_state(&agent.state)
                    );
                    println!();

                    // Instructions
                    println!("{}", "Instructions:".bright_cyan());
                    println!("{}", agent.base_instructions.dimmed());
                    println!();

                    // Statistics
                    println!("{}", "Statistics:".bright_cyan());
                    println!(
                        "  {} {}",
                        "Messages:".dimmed(),
                        agent.total_messages.to_string().bright_white()
                    );
                    println!(
                        "  {} {}",
                        "Tool calls:".dimmed(),
                        agent.total_tool_calls.to_string().bright_white()
                    );
                    println!(
                        "  {} {}",
                        "Context rebuilds:".dimmed(),
                        agent.context_rebuilds.to_string().bright_white()
                    );
                    println!(
                        "  {} {}",
                        "Compression events:".dimmed(),
                        agent.compression_events.to_string().bright_white()
                    );
                    println!();

                    // Memory blocks
                    println!(
                        "{} {} memory blocks",
                        "Memory:".bright_cyan(),
                        agent.memories.len().to_string().bright_white()
                    );
                    if !agent.memories.is_empty() {
                        for (memory, _relation) in &agent.memories {
                            println!(
                                "  • {} ({})",
                                memory.label.bright_yellow(),
                                format!("{} chars", memory.value.len()).dimmed()
                            );
                        }
                    }
                    println!();

                    // Timestamps
                    println!("{}", "Timestamps:".bright_cyan());
                    println!(
                        "  {} {}",
                        "Created:".dimmed(),
                        agent.created_at.format("%Y-%m-%d %H:%M:%S UTC")
                    );
                    println!(
                        "  {} {}",
                        "Updated:".dimmed(),
                        agent.updated_at.format("%Y-%m-%d %H:%M:%S UTC")
                    );
                    println!(
                        "  {} {}",
                        "Last active:".dimmed(),
                        format_relative_time(agent.last_active)
                    );

                    // Model preference
                    if let Some(model_id) = &agent.model_id {
                        println!();
                        println!(
                            "{} {}",
                            "Preferred model:".bright_cyan(),
                            model_id.bright_yellow()
                        );
                    }
                } else {
                    println!("{} No agent found with name '{}'", "❌".bright_red(), name);
                    println!();
                    println!("Available agents:");

                    // List all agents
                    let all_agents = ops::list_entities::<AgentRecord, _>(&DB).await?;
                    if all_agents.is_empty() {
                        println!("  {} No agents created yet", "•".dimmed());
                    } else {
                        for agent in all_agents {
                            println!("  {} {}", "•".bright_blue(), agent.name.bright_cyan());
                        }
                    }
                }
            }
        },
        Commands::Db { cmd } => match cmd {
            DbCommands::Stats => {
                println!("{} Database Statistics", "📊".bright_blue());
                println!("{}", "═".repeat(50).dimmed());
                println!();

                // Count entities - for now, just print responses
                let agent_response = DB
                    .query("SELECT count() FROM agent")
                    .await
                    .into_diagnostic()?;

                println!("Agent count response: {:?}", agent_response);

                let agent_count = 0; // TODO: Parse properly
                let message_count = 0; // TODO: Parse properly
                let memory_count = 0; // TODO: Parse properly
                let tool_call_count = 0; // TODO: Parse properly

                // Entity counts
                println!("{}", "Entity Counts:".bright_cyan());
                println!(
                    "  {} {} agents",
                    "👥".bright_blue(),
                    agent_count.to_string().bright_white()
                );
                println!(
                    "  {} {} messages",
                    "💬".bright_blue(),
                    message_count.to_string().bright_white()
                );
                println!(
                    "  {} {} memory blocks",
                    "🧠".bright_blue(),
                    memory_count.to_string().bright_white()
                );
                println!(
                    "  {} {} tool calls",
                    "🔧".bright_blue(),
                    tool_call_count.to_string().bright_white()
                );
                println!();

                // Most active agents
                let active_agents_query = r#"
                    SELECT name, total_messages, total_tool_calls, last_active
                    FROM agent
                    ORDER BY total_messages DESC
                    LIMIT 5
                "#;

                let mut response = DB.query(active_agents_query).await.into_diagnostic()?;

                let active_agents: Vec<serde_json::Value> = response.take(0).into_diagnostic()?;

                if !active_agents.is_empty() {
                    println!("{}", "Most Active Agents:".bright_cyan());
                    for agent in active_agents {
                        if let (Some(name), Some(messages), Some(tools)) = (
                            agent.get("name").and_then(|v| v.as_str()),
                            agent.get("total_messages").and_then(|v| v.as_u64()),
                            agent.get("total_tool_calls").and_then(|v| v.as_u64()),
                        ) {
                            println!(
                                "  {} {} - {} messages, {} tool calls",
                                "•".bright_blue(),
                                name.bright_yellow(),
                                messages.to_string().bright_white(),
                                tools.to_string().bright_white()
                            );
                        }
                    }
                    println!();
                }

                // Database info
                println!("{}", "Database Info:".bright_cyan());
                println!(
                    "  {} {}",
                    "Type:".dimmed(),
                    "SurrealDB (embedded)".bright_white()
                );
                println!(
                    "  {} {}",
                    "File:".dimmed(),
                    cli.db_path.display().to_string().bright_white()
                );

                // Get file size if possible
                if let Ok(metadata) = std::fs::metadata(&cli.db_path) {
                    let size = metadata.len();
                    let size_str = if size < 1024 {
                        format!("{} bytes", size)
                    } else if size < 1024 * 1024 {
                        format!("{:.2} KB", size as f64 / 1024.0)
                    } else {
                        format!("{:.2} MB", size as f64 / (1024.0 * 1024.0))
                    };
                    println!("  {} {}", "Size:".dimmed(), size_str.bright_white());
                }
            }
            DbCommands::Query { sql } => {
                println!("Running query: {}", sql);
                // TODO: Execute SQL query
            }
        },
    }

    Ok(())
}

/// Load an existing agent from the database or create a new one
async fn load_or_create_agent(
    name: &str,
    model_name: Option<String>,
    enable_tools: bool,
) -> Result<Box<dyn pattern_core::Agent>> {
    // First, try to find an existing agent with this name
    let query = "SELECT id FROM agent WHERE name = $name LIMIT 1";
    let mut response = DB
        .query(query)
        .bind(("name", name.to_string()))
        .await
        .into_diagnostic()?;

    let agent_ids: Vec<RecordId> = response.take("id").into_diagnostic()?;

    if let Some(id_value) = agent_ids.first() {
        let agent_id = AgentId::from_record(id_value.clone());

        // Load the full agent record
        let mut existing_agent = match AgentRecord::load_with_relations(&DB, agent_id).await {
            Ok(Some(agent)) => {
                tracing::trace!("Full AgentRecord: {:#?}", agent);
                agent
            }
            Ok(None) => return Err(miette::miette!("Agent not found after query")),
            Err(e) => return Err(miette::miette!("Failed to load agent: {}", e)),
        };

        // Manually load message history since the macro doesn't handle edge entities properly yet
        existing_agent.messages = existing_agent
            .load_message_history(&DB, false)
            .await
            .map_err(|e| miette::miette!("Failed to load message history: {}", e))?;

        tracing::debug!(
            "After loading message history: {} messages",
            existing_agent.messages.len()
        );

        // Also manually load memory blocks using the ops function
        let memory_tuples = ops::get_agent_memories(&DB, agent_id)
            .await
            .map_err(|e| miette::miette!("Failed to load memory blocks: {}", e))?;

        // Convert to the format expected by AgentRecord
        existing_agent.memories = memory_tuples
            .into_iter()
            .map(|(memory_block, access_level)| {
                let relation = pattern_core::agent::AgentMemoryRelation {
                    id: None,
                    in_id: agent_id,
                    out_id: memory_block.id,
                    access_level,
                    created_at: chrono::Utc::now(),
                };
                (memory_block, relation)
            })
            .collect();

        tracing::debug!(
            "After loading memory blocks: {} memories",
            existing_agent.memories.len()
        );
        println!(
            "{} Found existing agent '{}'",
            "✓".bright_green(),
            name.bright_cyan()
        );
        println!(
            "  {} {}",
            "ID:".dimmed(),
            existing_agent.id.to_string().dimmed()
        );
        println!(
            "  {} {}",
            "Type:".dimmed(),
            format!("{:?}", existing_agent.agent_type).bright_yellow()
        );
        println!(
            "  {} {} messages in history",
            "History:".dimmed(),
            existing_agent.total_messages.to_string().bright_white()
        );
        println!();

        // Create runtime agent from the stored record
        create_agent_from_record(existing_agent.clone(), model_name, enable_tools).await
    } else {
        println!(
            "{} Creating new agent '{}'",
            "+".bright_yellow(),
            name.bright_cyan()
        );
        println!();

        // Create a new agent
        create_agent(name, model_name, enable_tools).await
    }
}

/// Create a runtime agent from a stored AgentRecord
async fn create_agent_from_record(
    record: AgentRecord,
    model_name: Option<String>,
    enable_tools: bool,
) -> Result<Box<dyn pattern_core::Agent>> {
    use pattern_core::{embeddings::cloud::OpenAIEmbedder, model::ResponseOptions};

    // Create model provider
    let model_provider = Arc::new(RwLock::new(GenAiClient::new().await?));

    // Get available models and select the one to use
    let model_info = {
        let provider = model_provider.read().await;
        let models = provider.list_models().await?;

        // If a specific model was requested, try to find it
        let selected_model = if let Some(requested_model) = &model_name {
            models
                .iter()
                .find(|m| m.id.contains(requested_model) || m.name.contains(requested_model))
                .cloned()
        } else if let Some(stored_model) = &record.model_id {
            // Try to use the agent's stored model preference
            models.iter().find(|m| &m.id == stored_model).cloned()
        } else {
            // Default to Gemini models with free tier
            models
                .iter()
                .find(|m| m.provider == "Gemini" && m.id.contains("gemini-2.5-flash"))
                .cloned()
                .or_else(|| {
                    models
                        .iter()
                        .find(|m| m.provider == "Gemini" && m.id.contains("gemini-2.5-pro"))
                        .cloned()
                })
                .or_else(|| models.into_iter().next())
        };

        selected_model.ok_or_else(|| {
            miette::miette!("No models available. Please set API keys in your .env file")
        })?
    };

    info!("Selected model: {} ({})", model_info.name, model_info.id);

    // Create embedding provider if API key is available
    let embedding_provider = if let Ok(api_key) = std::env::var("OPENAI_API_KEY") {
        Some(Arc::new(OpenAIEmbedder::new(
            "text-embedding-3-small".to_string(),
            api_key,
            None,
        )))
    } else {
        None
    };

    // Create tool registry
    let tools = ToolRegistry::new();

    // Create response options with the selected model
    let response_options = ResponseOptions {
        model_info: model_info.clone(),
        temperature: Some(0.7),
        max_tokens: Some(120000),
        capture_content: Some(true),
        capture_tool_calls: Some(enable_tools),
        top_p: None,
        stop_sequences: vec![],
        capture_usage: Some(true),
        capture_reasoning_content: None,
        capture_raw_body: None,
        response_format: None,
        normalize_reasoning_content: None,
        reasoning_effort: None,
    };

    // Create agent from the record
    let agent = DatabaseAgent::from_record(
        record,
        Arc::new(DB.clone()),
        model_provider,
        tools,
        embedding_provider,
    )
    .await?;

    // Set the chat options with our selected model
    {
        let mut options = agent.chat_options.write().await;
        *options = Some(response_options);
    }

    Ok(Box::new(agent))
}

/// Create an agent with the specified configuration
async fn create_agent(
    name: &str,
    model_name: Option<String>,
    enable_tools: bool,
) -> Result<Box<dyn pattern_core::Agent>> {
    use pattern_core::{embeddings::cloud::OpenAIEmbedder, model::ResponseOptions};

    // Create model provider
    let model_provider = Arc::new(RwLock::new(GenAiClient::new().await?));

    // Get available models and select the one to use
    let model_info = {
        let provider = model_provider.read().await;
        let models = provider.list_models().await?;

        // Debug: print available models
        for model in &models {
            info!(
                "Available model: {} (id: {}, provider: {})",
                model.name, model.id, model.provider
            );
        }

        // If a specific model was requested, try to find it
        let selected_model = if let Some(requested_model) = &model_name {
            models
                .iter()
                .find(|m| m.id.contains(requested_model) || m.name.contains(requested_model))
                .cloned()
        } else {
            // Default to Gemini models with free tier, prioritizing Flash for better rate limits
            models
                .iter()
                .find(|m| m.provider == "Gemini" && m.id.contains("gemini-2.5-pro"))
                .cloned()
                .or_else(|| {
                    models
                        .iter()
                        .find(|m| m.provider == "Gemini" && m.id.contains("gemini-2.5-flash"))
                        .cloned()
                })
                .or_else(|| models.into_iter().next())
        };

        let model_info = selected_model.ok_or_else(|| {
            miette::miette!(
                "No models available. Please set one of the following environment variables:\n\
                - OPENAI_API_KEY\n\
                - ANTHROPIC_API_KEY\n\
                - GEMINI_API_KEY\n\
                - GROQ_API_KEY\n\
                - COHERE_API_KEY\n\n\
                You can add these to a .env file in your project root."
            )
        })?;

        info!("Selected model: {} ({})", model_info.name, model_info.id);
        model_info
    };

    // Create embedding provider if API key is available
    let embedding_provider = if let Ok(api_key) = std::env::var("OPENAI_API_KEY") {
        Some(Arc::new(OpenAIEmbedder::new(
            "text-embedding-3-small".to_string(),
            api_key,
            None,
        )))
    } else {
        None
    };

    // Create memory
    let memory = Memory::new();

    // Create tool registry
    let tools = ToolRegistry::new();

    // Generate IDs
    let agent_id = AgentId::generate();
    let user_id = UserId::generate();

    // Create response options with the selected model
    let response_options = ResponseOptions {
        model_info: model_info.clone(),
        temperature: Some(0.7),
        max_tokens: Some(1000),
        capture_content: Some(true),
        capture_tool_calls: Some(enable_tools),
        top_p: None,
        stop_sequences: vec![],
        capture_usage: Some(true),
        capture_reasoning_content: None,
        capture_raw_body: None,
        response_format: None,
        normalize_reasoning_content: None,
        reasoning_effort: None,
    };

    // Create agent
    let agent = DatabaseAgent::new(
        agent_id,
        user_id,
        AgentType::Generic,
        name.to_string(),
        String::new(),
        memory,
        Arc::new(DB.clone()),
        model_provider,
        tools,
        embedding_provider,
    );

    // Set the chat options with our selected model
    {
        let mut options = agent.chat_options.write().await;
        *options = Some(response_options);
    }

    // Store the agent in the database
    match agent.store().await {
        Ok(_) => {
            println!(
                "{} Saved new agent '{}' to database",
                "✓".dimmed(),
                name.bright_cyan()
            );
            println!();
        }
        Err(e) => {
            println!(
                "{} Warning: Failed to save agent to database: {}",
                "⚠".yellow(),
                e
            );
            println!("Agent will work for this session but won't persist");
            println!();
        }
    }

    Ok(Box::new(agent))
}

/// Chat with an agent
async fn chat_with_agent(agent: Box<dyn pattern_core::Agent>) -> Result<()> {
    use pattern_core::message::{Message, MessageContent};
    use rustyline::DefaultEditor;

    println!("{}", "Type 'quit' or 'exit' to leave the chat".dimmed());
    println!(
        "{}",
        "Use Ctrl+D for multiline input, Enter to send".dimmed()
    );
    println!();

    let mut rl = DefaultEditor::new().into_diagnostic()?;
    let prompt = format!("{} ", ">".bright_blue());

    loop {
        let readline = rl.readline(&prompt);
        match readline {
            Ok(line) => {
                if line.trim().is_empty() {
                    continue;
                }

                if line.trim() == "quit" || line.trim() == "exit" {
                    println!("{}", "Goodbye!".bright_green());
                    break;
                }

                // Add to history
                let _ = rl.add_history_entry(line.as_str());

                // Create a message using the actual Message structure
                let message = Message {
                    content: MessageContent::Text(line.clone()),
                    word_count: line.split_whitespace().count() as u32,
                    ..Default::default()
                };

                // Process message
                println!("{}", "Thinking...".dimmed());
                match agent.process_message(message).await {
                    Ok(response) => {
                        // Print response
                        let agent_name = agent.name();
                        for content in &response.content {
                            if let MessageContent::Text(text) = content {
                                println!("{} {}", agent_name.bright_cyan().bold(), text);
                            }
                        }
                        println!();
                    }
                    Err(e) => {
                        println!("{} {}", "Error:".bright_red(), e);
                    }
                }
            }
            Err(rustyline::error::ReadlineError::Interrupted) => {
                println!("{}", "CTRL-C".dimmed());
                continue;
            }
            Err(rustyline::error::ReadlineError::Eof) => {
                println!("{}", "CTRL-D".dimmed());
                break;
            }
            Err(err) => {
                println!("{} {:?}", "Error:".bright_red(), err);
                break;
            }
        }
    }

    Ok(())
}
