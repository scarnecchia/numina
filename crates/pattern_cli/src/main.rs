use clap::{Parser, Subcommand};
use colored::Colorize;
use miette::{IntoDiagnostic, Result};
use pattern_core::{
    Agent, ModelProvider,
    agent::{AgentRecord, AgentState, AgentType, DatabaseAgent},
    config::{self, PatternConfig},
    db::{
        DatabaseConfig, DbEntity,
        client::{self, DB},
        ops,
    },
    id::AgentId,
    memory::{Memory, MemoryBlock},
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

    /// Configuration file path
    #[arg(long, short = 'c')]
    config: Option<PathBuf>,

    /// Database file path (overrides config)
    #[arg(long)]
    db_path: Option<PathBuf>,

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

        /// Disable tool usage
        #[arg(long)]
        no_tools: bool,
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
    /// Debug tools
    Debug {
        #[command(subcommand)]
        cmd: DebugCommands,
    },
    /// Configuration management
    Config {
        #[command(subcommand)]
        cmd: ConfigCommands,
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

#[derive(Subcommand)]
enum ConfigCommands {
    /// Show current configuration
    Show,
    /// Save current configuration to file
    Save {
        /// Path to save configuration
        #[arg(default_value = "pattern.toml")]
        path: PathBuf,
    },
}

#[derive(Subcommand)]
enum DebugCommands {
    /// Search archival memory as if you were an agent
    SearchArchival {
        /// Agent name to search as
        #[arg(long)]
        agent: String,
        /// Search query
        query: String,
        /// Maximum number of results
        #[arg(long, default_value = "10")]
        limit: usize,
    },
    /// List all archival memories for an agent
    ListArchival {
        /// Agent name
        #[arg(long)]
        agent: String,
    },
    /// List all core memory blocks for an agent
    ListCore {
        /// Agent name
        #[arg(long)]
        agent: String,
    },
    /// List all memory blocks for an agent (core + archival)
    ListAllMemory {
        /// Agent name
        #[arg(long)]
        agent: String,
    },
    /// Search conversation history
    SearchConversations {
        /// Agent name to search as
        #[arg(long)]
        agent: String,
        /// Optional search query for message content
        query: Option<String>,
        /// Filter by role (system, user, assistant, tool)
        #[arg(long)]
        role: Option<String>,
        /// Start time (ISO 8601, e.g., 2024-01-20T00:00:00Z)
        #[arg(long)]
        start_time: Option<String>,
        /// End time (ISO 8601, e.g., 2024-01-20T23:59:59Z)
        #[arg(long)]
        end_time: Option<String>,
        /// Maximum number of results
        #[arg(long, default_value = "20")]
        limit: usize,
    },
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
    // Load .env file if it exists
    let _ = dotenvy::dotenv();
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

    // Load configuration
    let mut config = if let Some(config_path) = &cli.config {
        info!("Loading config from: {:?}", config_path);
        config::load_config(config_path).await?
    } else {
        info!("Loading config from standard locations");
        config::load_config_from_standard_locations().await?
    };

    // Apply CLI overrides
    if let Some(db_path) = &cli.db_path {
        info!("Overriding database path with: {:?}", db_path);
        config.database = DatabaseConfig::Embedded {
            path: db_path.to_string_lossy().to_string(),
            strict_mode: false,
        };
    }

    info!("Using database config: {:?}", config.database);

    // Initialize database
    info!("Initializing database...");
    client::init_db(config.database.clone()).await?;
    info!("Database initialized successfully");

    match &cli.command {
        Commands::Chat {
            agent,
            model,
            no_tools,
        } => {
            println!("{}", "Starting chat mode...".bright_green());
            println!("Agent: {}", agent.bright_cyan());
            if let Some(model_name) = &model {
                println!("Model: {}", model_name.bright_yellow());
            }
            if !*no_tools {
                println!("Tools: {}", "enabled".bright_green());
            } else {
                println!("Tools: {}", "disabled".bright_red());
            }

            // Try to load existing agent or create new one
            let agent = load_or_create_agent(agent, model.clone(), !*no_tools, &config).await?;
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

                // Create agent record using user from config
                let user_id = config.user.id.clone();
                let now = chrono::Utc::now();

                // Use agent ID from config if available
                let agent_id = config.agent.id.clone().unwrap_or_else(AgentId::generate);

                // Use system prompt from config or generate default
                let base_instructions = if let Some(system_prompt) = &config.agent.system_prompt {
                    system_prompt.clone()
                } else {
                    // Use default system prompt
                    format!(
                        "You are {}, a {} agent in the Pattern ADHD support system.",
                        name,
                        parsed_type.as_str()
                    )
                };

                let agent = AgentRecord {
                    id: agent_id.clone(),
                    name: name.clone(),
                    agent_type: parsed_type.clone(),
                    state: AgentState::Ready,
                    base_instructions,
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

                        // Save the agent ID back to config if it was generated
                        if config.agent.id.is_none() {
                            println!("Saving agent ID to config for future sessions...");
                            let mut updated_config = config.clone();
                            updated_config.agent.id = Some(stored_agent.id.clone());
                            if let Err(e) =
                                config::save_config(&updated_config, &config::config_paths()[0])
                                    .await
                            {
                                println!(
                                    "{} Failed to save agent ID to config: {}",
                                    "Warning:".yellow(),
                                    e
                                );
                            }
                        }

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
                    match &config.database {
                        DatabaseConfig::Embedded { path, .. } => path.bright_white(),
                        //DatabaseConfig::Remote { url, .. } => url.bright_white(),
                        #[allow(unreachable_patterns)]
                        _ => {
                            "".bright_yellow()
                        }
                    }
                );

                // Get file size if possible for embedded databases
                #[allow(irrefutable_let_patterns)]
                if let DatabaseConfig::Embedded { path, .. } = &config.database {
                    if let Ok(metadata) = std::fs::metadata(path) {
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
            }
            DbCommands::Query { sql } => {
                println!("Running query: {}", sql.bright_cyan());
                println!();

                // Execute the query
                let response = DB.query(sql).await.into_diagnostic()?;

                println!("Results: {:?}", response);
            }
        },
        Commands::Debug { cmd } => match cmd {
            DebugCommands::SearchArchival {
                agent,
                query,
                limit,
            } => {
                search_archival_memory(agent, query, *limit).await?;
            }
            DebugCommands::ListArchival { agent } => {
                list_archival_memory(&agent).await?;
            }
            DebugCommands::ListCore { agent } => {
                list_core_memory(&agent).await?;
            }
            DebugCommands::ListAllMemory { agent } => {
                list_all_memory(&agent).await?;
            }
            DebugCommands::SearchConversations {
                agent,
                query,
                role,
                start_time,
                end_time,
                limit,
            } => {
                search_conversations(
                    &agent,
                    query.as_deref(),
                    role.as_deref(),
                    start_time.as_deref(),
                    end_time.as_deref(),
                    *limit,
                )
                .await?;
            }
        },
        Commands::Config { cmd } => match cmd {
            ConfigCommands::Show => {
                println!("{} Current Configuration", "⚙️".bright_blue());
                println!("{}", "═".repeat(50).dimmed());
                println!();

                // Display the current config in TOML format
                let toml_str = toml::to_string_pretty(&config).into_diagnostic()?;
                println!("{}", toml_str);
            }
            ConfigCommands::Save { path } => {
                println!(
                    "{} Saving configuration to: {}",
                    "💾".bright_blue(),
                    path.display()
                );

                // Save the current config
                config::save_config(&config, path).await?;

                println!("{} Configuration saved successfully!", "✓".bright_green());
                println!();
                println!("To use this configuration, run:");
                println!(
                    "  {} --config {}",
                    "pattern-cli".bright_green(),
                    path.display()
                );
            }
        },
    }

    Ok(())
}

/// Search conversation history for an agent
async fn search_conversations(
    agent_name: &str,
    query: Option<&str>,
    role: Option<&str>,
    start_time: Option<&str>,
    end_time: Option<&str>,
    limit: usize,
) -> Result<()> {
    use chrono::DateTime;
    use pattern_core::context::AgentHandle;
    use pattern_core::message::ChatRole;

    println!("{} Searching conversation history", "🔍".bright_blue());
    println!("{}", "─".repeat(50).dimmed());
    println!();

    // First, find the agent
    let query_sql = "SELECT * FROM agent WHERE name = $name LIMIT 1";
    let mut response = DB
        .query(query_sql)
        .bind(("name", agent_name.to_string()))
        .await
        .into_diagnostic()?;

    let agents: Vec<<AgentRecord as DbEntity>::DbModel> = response.take(0).into_diagnostic()?;
    let agents: Vec<_> = agents
        .into_iter()
        .map(|e| AgentRecord::from_db_model(e).unwrap())
        .collect();

    if let Some(agent_record) = agents.first() {
        println!(
            "Agent: {} (ID: {})",
            agent_record.name.bright_cyan(),
            agent_record.id.to_string().dimmed()
        );
        println!("Owner: {}", agent_record.owner_id.to_string().dimmed());

        // Display search parameters
        if let Some(q) = query {
            println!("Query: \"{}\"", q.bright_yellow());
        }
        if let Some(r) = role {
            println!("Role filter: {}", r.bright_yellow());
        }
        if let Some(st) = start_time {
            println!("Start time: {}", st.bright_yellow());
        }
        if let Some(et) = end_time {
            println!("End time: {}", et.bright_yellow());
        }
        println!("Limit: {}", limit.to_string().bright_white());
        println!();

        // Parse role if provided
        let role_filter = if let Some(role_str) = role {
            match role_str.to_lowercase().as_str() {
                "system" => Some(ChatRole::System),
                "user" => Some(ChatRole::User),
                "assistant" => Some(ChatRole::Assistant),
                "tool" => Some(ChatRole::Tool),
                _ => {
                    println!(
                        "{} Invalid role: {}. Using no role filter.",
                        "⚠️".yellow(),
                        role_str
                    );
                    println!("Valid roles: system, user, assistant, tool");
                    println!();
                    None
                }
            }
        } else {
            None
        };

        // Parse timestamps if provided
        let start_dt = if let Some(st) = start_time {
            match DateTime::parse_from_rfc3339(st) {
                Ok(dt) => Some(dt.to_utc()),
                Err(e) => {
                    println!("{} Invalid start time format: {}", "⚠️".yellow(), e);
                    println!("Expected ISO 8601 format: 2024-01-20T00:00:00Z");
                    println!();
                    None
                }
            }
        } else {
            None
        };

        let end_dt = if let Some(et) = end_time {
            match DateTime::parse_from_rfc3339(et) {
                Ok(dt) => Some(dt.to_utc()),
                Err(e) => {
                    println!("{} Invalid end time format: {}", "⚠️".yellow(), e);
                    println!("Expected ISO 8601 format: 2024-01-20T23:59:59Z");
                    println!();
                    None
                }
            }
        } else {
            None
        };

        // Create a minimal agent handle for searching
        let memory = Memory::with_owner(agent_record.owner_id.clone());
        let mut handle = AgentHandle::default();
        handle.name = agent_record.name.clone();
        handle.agent_id = agent_record.id.clone();
        handle.agent_type = agent_record.agent_type.clone();
        handle.memory = memory;
        handle.state = AgentState::Ready;
        let handle = handle.with_db(DB.clone());

        // Perform the search
        match handle
            .search_conversations(query, role_filter, start_dt, end_dt, limit)
            .await
        {
            Ok(messages) => {
                println!(
                    "Found {} messages:",
                    messages.len().to_string().bright_green()
                );
                println!();

                for (i, msg) in messages.iter().enumerate() {
                    println!(
                        "{} Message {} {}",
                        match msg.role {
                            ChatRole::System => "🔧",
                            ChatRole::User => "👤",
                            ChatRole::Assistant => "🤖",
                            ChatRole::Tool => "🔨",
                        },
                        (i + 1).to_string().bright_white(),
                        format!("({})", msg.id.0).dimmed()
                    );
                    println!("  Role: {}", format!("{:?}", msg.role).bright_yellow());
                    println!("  Time: {}", msg.created_at.format("%Y-%m-%d %H:%M:%S UTC"));

                    // Extract and display content
                    if let Some(text) = msg.text_content() {
                        let preview = if text.len() > 200 {
                            format!("{}...", &text[..200])
                        } else {
                            text.to_string()
                        };
                        println!("  Content:");
                        for line in preview.lines() {
                            println!("    {}", line.dimmed());
                        }
                    } else {
                        println!("  Content: [Non-text content]");
                    }

                    println!();
                }

                if messages.is_empty() {
                    println!(
                        "{} No messages found matching the search criteria",
                        "ℹ".yellow()
                    );
                    println!();
                    println!("Try:");
                    println!("  • Using broader search terms");
                    println!("  • Removing filters to see all messages");
                    println!("  • Checking if the agent has any messages in the database");
                }
            }
            Err(e) => {
                println!("{} Search failed: {}", "❌".bright_red(), e);
                println!();
                println!("This might mean:");
                println!("  • The database connection is not available");
                println!("  • There was an error in the query");
                println!("  • The message table or indexes are not set up");
            }
        }
    } else {
        println!("{} Agent '{}' not found", "❌".bright_red(), agent_name);
        println!();
        println!("Available agents:");
        let all_agents = ops::list_entities::<AgentRecord, _>(&DB).await?;
        for agent in all_agents {
            println!("  • {}", agent.name.bright_cyan());
        }
    }

    Ok(())
}

/// Load an existing agent from the database or create a new one
async fn load_or_create_agent(
    name: &str,
    model_name: Option<String>,
    enable_tools: bool,
    config: &PatternConfig,
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
        println!(
            "{} Loaded {} messages from history",
            "📨".bright_blue(),
            existing_agent.messages.len()
        );

        // Also manually load memory blocks using the ops function
        let memory_tuples = ops::get_agent_memories(&DB, agent_id)
            .await
            .map_err(|e| miette::miette!("Failed to load memory blocks: {}", e))?;

        println!(
            "{} Found {} memory blocks in database",
            "🧠".bright_blue(),
            memory_tuples.len()
        );
        for (block, _) in &memory_tuples {
            println!(
                "  • {} ({} chars)",
                block.label.bright_yellow(),
                block.value.len()
            );
        }

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
        create_agent_from_record(existing_agent.clone(), model_name, enable_tools, config).await
    } else {
        println!(
            "{} Creating new agent '{}'",
            "+".bright_yellow(),
            name.bright_cyan()
        );
        println!();

        // Create a new agent
        create_agent(name, model_name, enable_tools, config).await
    }
}

/// Create a runtime agent from a stored AgentRecord
async fn create_agent_from_record(
    record: AgentRecord,
    model_name: Option<String>,
    enable_tools: bool,
    _config: &PatternConfig,
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
        max_tokens: Some(1000000), // for gemini models
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
        DB.clone(),
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
    agent.start_stats_sync().await?;
    agent.start_memory_sync().await?;

    Ok(Box::new(agent))
}

/// Create an agent with the specified configuration
async fn create_agent(
    name: &str,
    model_name: Option<String>,
    enable_tools: bool,
    config: &PatternConfig,
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

    // Create memory with the configured user as owner
    let memory = Memory::with_owner(config.user.id.clone());

    // Create tool registry
    let tools = ToolRegistry::new();

    // Use IDs from config or generate new ones
    let agent_id = config.agent.id.clone().unwrap_or_else(AgentId::generate);
    let user_id = config.user.id.clone();

    // Create response options with the selected model
    let response_options = ResponseOptions {
        model_info: model_info.clone(),
        temperature: Some(0.7),
        max_tokens: Some(1000000), // for gemini models
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
        // Empty base instructions - will be set later from context
        String::new(),
        memory,
        DB.clone(),
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

    agent.start_stats_sync().await?;
    agent.start_memory_sync().await?;

    // Add persona as a core memory block if configured
    if let Some(persona) = &config.agent.persona {
        println!("Adding persona to agent's core memory...");
        let persona_block = MemoryBlock::owned(config.user.id.clone(), "persona", persona.clone())
            .with_description("Agent's persona and identity")
            .with_permission(pattern_core::memory::MemoryPermission::ReadOnly);

        if let Err(e) = agent.update_memory("persona", persona_block).await {
            println!(
                "{} Failed to add persona memory: {}",
                "Warning:".yellow(),
                e
            );
        }
    }

    // Add any pre-configured memory blocks
    for (label, block_config) in &config.agent.memory {
        println!(
            "Adding memory block '{}' to agent...",
            label.bright_yellow()
        );
        let memory_block = MemoryBlock::owned(
            config.user.id.clone(),
            label.clone(),
            block_config.content.clone(),
        )
        .with_memory_type(block_config.memory_type)
        .with_permission(block_config.permission);

        let memory_block = if let Some(desc) = &block_config.description {
            memory_block.with_description(desc.clone())
        } else {
            memory_block
        };

        if let Err(e) = agent.update_memory(label, memory_block).await {
            println!(
                "{} Failed to add memory block '{}': {}",
                "Warning:".yellow(),
                label,
                e
            );
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
                            match content {
                                MessageContent::Text(text) => {
                                    println!("{} {}", agent_name.bright_cyan().bold(), text);
                                }
                                MessageContent::ToolCalls(calls) => {
                                    for call in calls {
                                        println!(
                                            "{} Using tool: {}",
                                            "🔧".bright_blue(),
                                            call.fn_name.bright_yellow()
                                        );
                                        println!(
                                            "   Args: {}",
                                            serde_json::to_string_pretty(&call.fn_arguments)
                                                .unwrap_or_else(|_| call.fn_arguments.to_string())
                                        );
                                    }
                                }
                                MessageContent::ToolResponses(responses) => {
                                    for resp in responses {
                                        println!(
                                            "{} Tool result: {}",
                                            "✓".bright_green(),
                                            resp.content.dimmed()
                                        );
                                    }
                                }
                                MessageContent::Parts(_) => {
                                    // Multi-part content, just show text parts for now
                                    println!(
                                        "{} [Multi-part content]",
                                        agent_name.bright_cyan().bold()
                                    );
                                }
                            }
                        }

                        // Also show tool calls from the response object
                        if !response.tool_calls.is_empty() {
                            for call in &response.tool_calls {
                                println!(
                                    "{} Tool call: {}",
                                    "🔧".bright_blue(),
                                    call.fn_name.bright_yellow()
                                );
                                println!(
                                    "   Args: {}",
                                    serde_json::to_string_pretty(&call.fn_arguments)
                                        .unwrap_or_else(|_| call.fn_arguments.to_string())
                                );
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

/// Search archival memory as if we were the agent
async fn search_archival_memory(agent_name: &str, query: &str, limit: usize) -> Result<()> {
    use pattern_core::context::AgentHandle;

    println!("{} Searching archival memory", "🔍".bright_blue());
    println!("{}", "─".repeat(50).dimmed());
    println!();

    // First, find the agent
    let query_sql = "SELECT * FROM agent WHERE name = $name LIMIT 1";
    let mut response = DB
        .query(query_sql)
        .bind(("name", agent_name.to_string()))
        .await
        .into_diagnostic()?;

    let agents: Vec<<AgentRecord as DbEntity>::DbModel> = response.take(0).into_diagnostic()?;
    let agents: Vec<_> = agents
        .into_iter()
        .map(|e| AgentRecord::from_db_model(e).unwrap())
        .collect();

    if let Some(agent_record) = agents.first() {
        println!(
            "Agent: {} (ID: {})",
            agent_record.name.bright_cyan(),
            agent_record.id.to_string().dimmed()
        );
        println!("Owner: {}", agent_record.owner_id.to_string().dimmed());
        println!("Query: \"{}\"", query.bright_yellow());
        println!("Limit: {}", limit.to_string().bright_white());

        // Debug: Let's check what memories exist for this owner
        let debug_query = format!(
            "SELECT id, label, memory_type FROM mem WHERE owner_id = user:⟨{}⟩",
            agent_record
                .owner_id
                .to_string()
                .trim_start_matches("user_")
        );
        println!("Debug - checking memories for owner...");
        let debug_response = DB.query(&debug_query).await.into_diagnostic()?;
        println!("Debug response: {:?}", debug_response);
        println!();

        // Create a minimal agent handle for searching
        // IMPORTANT: Use the actual owner_id from the database so the search will match
        let memory = Memory::with_owner(agent_record.owner_id.clone());
        let mut handle = AgentHandle::default();
        handle.name = agent_record.name.clone();
        handle.agent_id = agent_record.id.clone();
        handle.agent_type = agent_record.agent_type.clone();
        handle.memory = memory;
        handle.state = AgentState::Ready;
        let handle = handle.with_db(DB.clone());

        // Perform the search
        match handle.search_archival_memories(query, limit).await {
            Ok(results) => {
                println!(
                    "Found {} results:",
                    results.len().to_string().bright_green()
                );
                println!();

                for (i, block) in results.iter().enumerate() {
                    println!(
                        "{} Result {} {}",
                        "📄".bright_blue(),
                        (i + 1).to_string().bright_white(),
                        format!("({})", block.id).dimmed()
                    );
                    println!("  Label: {}", block.label.bright_yellow());
                    println!("  Type: {:?}", block.memory_type);
                    println!(
                        "  Created: {}",
                        block.created_at.format("%Y-%m-%d %H:%M:%S UTC")
                    );
                    println!("  Content preview:");

                    // Show first 200 chars of content
                    let preview = if block.value.len() > 200 {
                        format!("{}...", &block.value[..200])
                    } else {
                        block.value.clone()
                    };

                    for line in preview.lines() {
                        println!("    {}", line.dimmed());
                    }
                    println!();
                }

                if results.is_empty() {
                    println!(
                        "{} No archival memories found matching '{}'",
                        "ℹ".yellow(),
                        query
                    );
                    println!();
                    println!("Try:");
                    println!("  • Using broader search terms");
                    println!(
                        "  • Checking if the agent has any archival memories with: pattern-cli debug list-archival --agent {}",
                        agent_name
                    );
                    println!("  • Verifying the full-text search index exists in the database");
                }
            }
            Err(e) => {
                println!("{} Search failed: {}", "❌".bright_red(), e);
                println!();
                println!("This might mean:");
                println!("  • The database connection is not available");
                println!("  • The full-text search index is not set up");
                println!("  • There was an error in the query");
            }
        }
    } else {
        println!("{} Agent '{}' not found", "❌".bright_red(), agent_name);
        println!();
        println!("Available agents:");
        let all_agents = ops::list_entities::<AgentRecord, _>(&DB).await?;
        for agent in all_agents {
            println!("  • {}", agent.name.bright_cyan());
        }
    }

    Ok(())
}

/// List all archival memories for an agent
async fn list_archival_memory(agent_name: &str) -> Result<()> {
    println!("{} Listing archival memories", "📚".bright_blue());
    println!("{}", "─".repeat(50).dimmed());
    println!();

    // First, find the agent
    let query_sql = "SELECT * FROM agent WHERE name = $name LIMIT 1";
    let mut response = DB
        .query(query_sql)
        .bind(("name", agent_name.to_string()))
        .await
        .into_diagnostic()?;

    println!("response: {:?}", response);

    let agents: Vec<<AgentRecord as DbEntity>::DbModel> = response.take(0).into_diagnostic()?;
    let agents: Vec<_> = agents
        .into_iter()
        .map(|e| AgentRecord::from_db_model(e).unwrap())
        .collect();

    if let Some(agent_record) = agents.first() {
        println!(
            "Agent: {} (ID: {})",
            agent_record.name.bright_cyan(),
            agent_record.id.to_string().dimmed()
        );
        println!();

        // Query for all archival memories this agent has access to
        // Note: SurrealDB requires fields in ORDER BY to be explicitly selected or use no prefix
        let mem_query = r#"
            SELECT *, ->agent_memories->mem AS memories FROM $agent_id FETCH memories
        "#;

        let mut mem_response = DB
            .query(mem_query)
            .bind(("agent_id", RecordId::from(&agent_record.id)))
            .await
            .into_diagnostic()?;

        println!("Debug - mem_response: {:?}", mem_response);

        let memories: Vec<Vec<<MemoryBlock as DbEntity>::DbModel>> =
            mem_response.take("memories").into_diagnostic()?;

        let memories: Vec<_> = memories
            .concat()
            .into_iter()
            .map(|m| MemoryBlock::from_db_model(m).expect("db model"))
            .collect();

        println!(
            "Found {} archival memories",
            memories.len().to_string().bright_green()
        );
        println!();

        for (i, block) in memories.iter().enumerate() {
            println!(
                "{} Memory {} {}",
                "🧠".bright_blue(),
                (i + 1).to_string().bright_white(),
                format!("({})", block.id).dimmed()
            );
            println!("  Label: {}", block.label.bright_yellow());
            println!("  Owner: {}", block.owner_id.to_string().dimmed());
            println!(
                "  Created: {}",
                block.created_at.format("%Y-%m-%d %H:%M:%S UTC")
            );
            println!(
                "  Size: {} chars",
                block.value.len().to_string().bright_white()
            );

            if let Some(desc) = &block.description {
                println!("  Description: {}", desc.dimmed());
            }

            // Show first 100 chars
            let preview = if block.value.len() > 100 {
                format!("{}...", &block.value[..100])
            } else {
                block.value.clone()
            };
            println!("  Preview: {}", preview.dimmed());
            println!();
        }

        if memories.is_empty() {
            println!("{} No archival memories found for this agent", "ℹ".yellow());
            println!();
            println!("Archival memories can be created:");
            println!("  • By the agent using the recall tool");
            println!("  • Through the API");
            println!("  • By importing from external sources");
        }
    } else {
        println!("{} Agent '{}' not found", "❌".bright_red(), agent_name);
    }

    Ok(())
}

/// List all core memory blocks for an agent
async fn list_core_memory(agent_name: &str) -> Result<()> {
    println!("{} Listing core memory blocks", "🧠".bright_blue());
    println!("{}", "─".repeat(50).dimmed());
    println!();

    // First, find the agent
    let query_sql = "SELECT * FROM agent WHERE name = $name LIMIT 1";
    let mut response = DB
        .query(query_sql)
        .bind(("name", agent_name.to_string()))
        .await
        .into_diagnostic()?;

    let agents: Vec<<AgentRecord as DbEntity>::DbModel> = response.take(0).into_diagnostic()?;
    let agents: Vec<_> = agents
        .into_iter()
        .map(|e| AgentRecord::from_db_model(e).unwrap())
        .collect();

    if let Some(agent_record) = agents.first() {
        println!(
            "Agent: {} (ID: {})",
            agent_record.name.bright_cyan(),
            agent_record.id.to_string().dimmed()
        );
        println!();

        // Query for all core memory blocks this agent has access to
        // Core memories are memory_type = 'core' or NULL (default)
        let mem_query = r#"
            SELECT * FROM mem
            WHERE id IN (
                SELECT out FROM agent_memories
                WHERE in = $agent_id
            )
            AND (memory_type = 'core' OR memory_type = NULL)
            ORDER BY created_at DESC
        "#;

        let mut mem_response = DB
            .query(mem_query)
            .bind(("agent_id", RecordId::from(&agent_record.id)))
            .await
            .into_diagnostic()?;

        let memories: Vec<MemoryBlock> = mem_response.take(0).into_diagnostic()?;

        println!(
            "Found {} core memory blocks",
            memories.len().to_string().bright_green()
        );
        println!();

        for (i, block) in memories.iter().enumerate() {
            println!(
                "{} Memory {} {}",
                "📝".bright_blue(),
                (i + 1).to_string().bright_white(),
                format!("({})", block.id).dimmed()
            );
            println!("  Label: {}", block.label.bright_yellow());
            println!("  Type: {:?}", block.memory_type);
            println!("  Permission: {:?}", block.permission);
            println!("  Owner: {}", block.owner_id.to_string().dimmed());
            println!(
                "  Created: {}",
                block.created_at.format("%Y-%m-%d %H:%M:%S UTC")
            );
            println!(
                "  Updated: {}",
                block.updated_at.format("%Y-%m-%d %H:%M:%S UTC")
            );
            println!(
                "  Size: {} chars",
                block.value.len().to_string().bright_white()
            );

            if let Some(desc) = &block.description {
                println!("  Description: {}", desc.dimmed());
            }

            // Show full content for core memories (they're usually smaller)
            println!("  Content:");
            for line in block.value.lines() {
                println!("    {}", line.dimmed());
            }
            println!();
        }

        if memories.is_empty() {
            println!(
                "{} No core memory blocks found for this agent",
                "ℹ".yellow()
            );
            println!();
            println!("Core memory blocks are usually created:");
            println!("  • Automatically when an agent is initialized");
            println!("  • By the agent using the context tool");
            println!("  • Through direct API calls");
        }
    } else {
        println!("{} Agent '{}' not found", "❌".bright_red(), agent_name);
    }

    Ok(())
}

/// List all memory blocks for an agent (both core and archival)
async fn list_all_memory(agent_name: &str) -> Result<()> {
    println!("{} Listing all memory blocks", "🗂️".bright_blue());
    println!("{}", "─".repeat(50).dimmed());
    println!();

    // First, find the agent
    let query_sql = "SELECT * FROM agent WHERE name = $name LIMIT 1";
    let mut response = DB
        .query(query_sql)
        .bind(("name", agent_name.to_string()))
        .await
        .into_diagnostic()?;

    let agents: Vec<<AgentRecord as DbEntity>::DbModel> = response.take(0).into_diagnostic()?;
    let agents: Vec<_> = agents
        .into_iter()
        .map(|e| AgentRecord::from_db_model(e).unwrap())
        .collect();

    if let Some(agent_record) = agents.first() {
        println!(
            "Agent: {} (ID: {})",
            agent_record.name.bright_cyan(),
            agent_record.id.to_string().dimmed()
        );
        println!("Owner: {}", agent_record.owner_id.to_string().dimmed());
        println!();

        // Query for all memory blocks this agent has access to
        let mem_query = r#"
            SELECT * FROM mem
            WHERE id IN (
                SELECT out FROM agent_memories
                WHERE in = $agent_id
            )
            ORDER BY memory_type, created_at DESC
        "#;

        let mut mem_response = DB
            .query(mem_query)
            .bind(("agent_id", RecordId::from(&agent_record.id)))
            .await
            .into_diagnostic()?;

        let memories: Vec<MemoryBlock> = mem_response.take(0).into_diagnostic()?;

        // Group by memory type
        let mut core_memories = Vec::new();
        let mut archival_memories = Vec::new();
        let mut other_memories = Vec::new();

        for memory in memories {
            match memory.memory_type {
                pattern_core::memory::MemoryType::Core => core_memories.push(memory),
                pattern_core::memory::MemoryType::Archival => archival_memories.push(memory),
                _ => other_memories.push(memory),
            }
        }

        let total = core_memories.len() + archival_memories.len() + other_memories.len();
        println!(
            "Found {} total memory blocks",
            total.to_string().bright_green()
        );
        println!();

        // Display core memories
        if !core_memories.is_empty() {
            println!(
                "{} Core Memory Blocks ({})",
                "📝".bright_blue(),
                core_memories.len()
            );
            println!("{}", "─".repeat(30).dimmed());
            for (i, block) in core_memories.iter().enumerate() {
                println!(
                    "  {} {} - {}",
                    (i + 1).to_string().bright_white(),
                    block.label.bright_yellow(),
                    format!("{} chars", block.value.len()).dimmed()
                );
                if let Some(desc) = &block.description {
                    println!("     {}", desc.dimmed());
                }
            }
            println!();
        }

        // Display archival memories
        if !archival_memories.is_empty() {
            println!(
                "{} Archival Memory Blocks ({})",
                "📚".bright_blue(),
                archival_memories.len()
            );
            println!("{}", "─".repeat(30).dimmed());
            for (i, block) in archival_memories.iter().enumerate() {
                println!(
                    "  {} {} - {}",
                    (i + 1).to_string().bright_white(),
                    block.label.bright_yellow(),
                    format!("{} chars", block.value.len()).dimmed()
                );
                // Show preview for archival memories
                let preview = if block.value.len() > 50 {
                    format!("{}...", &block.value[..50])
                } else {
                    block.value.clone()
                };
                println!("     {}", preview.dimmed());
            }
            println!();
        }

        // Display other memories
        if !other_memories.is_empty() {
            println!(
                "{} Other Memory Blocks ({})",
                "📋".bright_blue(),
                other_memories.len()
            );
            println!("{}", "─".repeat(30).dimmed());
            for (i, block) in other_memories.iter().enumerate() {
                println!(
                    "  {} {} ({:?}) - {}",
                    (i + 1).to_string().bright_white(),
                    block.label.bright_yellow(),
                    block.memory_type,
                    format!("{} chars", block.value.len()).dimmed()
                );
            }
            println!();
        }

        if total == 0 {
            println!("{} No memory blocks found for this agent", "ℹ".yellow());
            println!();
            println!("Memory blocks are created:");
            println!("  • Automatically when an agent is used in chat");
            println!("  • By the agent using memory management tools");
            println!("  • Through direct API calls");
        }
    } else {
        println!("{} Agent '{}' not found", "❌".bright_red(), agent_name);
    }

    Ok(())
}
