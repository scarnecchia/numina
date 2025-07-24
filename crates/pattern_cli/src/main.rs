mod agent_ops;
mod commands;
mod output;

use clap::{Parser, Subcommand};
use miette::Result;
use owo_colors::OwoColorize;
use pattern_core::{
    config::{self},
    db::{DatabaseConfig, client},
};
use std::path::PathBuf;
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

    fmt()
        .with_env_filter(filter)
        .with_file(true)
        .with_line_number(true) // Show target module
        .with_thread_ids(false)
        .with_thread_names(false)
        .with_timer(tracing_subscriber::fmt::time::LocalTime::rfc_3339()) // Local time in RFC 3339 format
        .compact()
        .init();

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

    tracing::debug!("Using database config: {:?}", config.database);

    // Initialize database
    client::init_db(config.database.clone()).await?;

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
            let agent =
                agent_ops::load_or_create_agent(agent, model.clone(), !*no_tools, &config).await?;
            agent_ops::chat_with_agent(agent).await?;
        }
        Commands::Agent { cmd } => match cmd {
            AgentCommands::List => commands::agent::list().await?,
            AgentCommands::Create { name, agent_type } => {
                commands::agent::create(name, agent_type.as_deref(), &config).await?
            }
            AgentCommands::Status { name } => commands::agent::status(name).await?,
        },
        Commands::Db { cmd } => match cmd {
            DbCommands::Stats => commands::db::stats(&config).await?,
            DbCommands::Query { sql } => commands::db::query(sql).await?,
        },
        Commands::Debug { cmd } => match cmd {
            DebugCommands::SearchArchival {
                agent,
                query,
                limit,
            } => {
                commands::debug::search_archival_memory(agent, query, *limit).await?;
            }
            DebugCommands::ListArchival { agent } => {
                commands::debug::list_archival_memory(&agent).await?;
            }
            DebugCommands::ListCore { agent } => {
                commands::debug::list_core_memory(&agent).await?;
            }
            DebugCommands::ListAllMemory { agent } => {
                commands::debug::list_all_memory(&agent).await?;
            }
            DebugCommands::SearchConversations {
                agent,
                query,
                role,
                start_time,
                end_time,
                limit,
            } => {
                commands::debug::search_conversations(
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
            ConfigCommands::Show => commands::config::show(&config).await?,
            ConfigCommands::Save { path } => commands::config::save(&config, path).await?,
        },
    }

    Ok(())
}
