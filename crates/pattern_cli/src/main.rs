mod agent_ops;
mod commands;
mod endpoints;
mod output;
mod tracing_writer;

use clap::{Parser, Subcommand};
use miette::Result;
use owo_colors::OwoColorize;
use pattern_core::{
    config::{self},
    coordination::selectors::DefaultSelectorRegistry,
    db::{
        DatabaseConfig,
        client::{self, DB},
        ops,
    },
};
use std::{path::PathBuf, sync::Arc};
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
        #[arg(long, default_value = "Pattern", conflicts_with = "group")]
        agent: String,

        /// Group name to chat with
        #[arg(long, conflicts_with = "agent")]
        group: Option<String>,

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
    /// Agent group management
    Group {
        #[command(subcommand)]
        cmd: GroupCommands,
    },
    /// OAuth authentication
    #[cfg(feature = "oauth")]
    Auth {
        #[command(subcommand)]
        cmd: AuthCommands,
    },
    /// ATProto/Bluesky authentication
    Atproto {
        #[command(subcommand)]
        cmd: AtprotoCommands,
    },
    /// Bluesky firehose testing
    Firehose {
        #[command(subcommand)]
        cmd: FirehoseCommands,
    },
    /// Export agents, groups, or constellations to CAR files
    Export {
        #[command(subcommand)]
        cmd: ExportCommands,
    },
    /// Import from CAR files
    Import {
        /// Path to CAR file to import
        file: PathBuf,

        /// Rename imported entity to this name
        #[arg(long)]
        rename_to: Option<String>,

        /// Preserve original IDs when importing
        #[arg(long)]
        preserve_ids: bool,
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
    /// Export agent configuration (persona and memory only)
    Export {
        /// Agent name to export
        name: String,
        /// Output file path (defaults to <agent_name>.toml)
        #[arg(short = 'o', long)]
        output: Option<PathBuf>,
    },
    /// Add a workflow rule to an agent
    AddRule {
        /// Agent name
        agent: String,
        /// Rule type (start-constraint, max-calls, exit-loop, continue-loop, cooldown, requires-preceding). If not provided, interactive mode is used.
        rule_type: Option<String>,
        /// Tool name the rule applies to. If not provided, interactive mode is used.
        tool: Option<String>,
        /// Optional rule parameters (e.g., max count for max-calls, duration for cooldown)
        #[arg(short = 'p', long)]
        params: Option<String>,
        /// Optional conditions (comma-separated tool names)
        #[arg(short = 'c', long)]
        conditions: Option<String>,
        /// Rule priority (1-10, higher = more important)
        #[arg(long, default_value = "5")]
        priority: u8,
    },
    /// List workflow rules for an agent
    ListRules {
        /// Agent name
        agent: String,
    },
    /// Remove a workflow rule from an agent
    RemoveRule {
        /// Agent name
        agent: String,
        /// Tool name to remove rules for
        tool: String,
        /// Optional rule type to remove (removes all if not specified)
        rule_type: Option<String>,
    },
}

#[cfg(feature = "oauth")]
#[derive(Subcommand)]
enum AuthCommands {
    /// Authenticate with Anthropic OAuth
    Login {
        /// Provider to authenticate with
        #[arg(default_value = "anthropic")]
        provider: String,
    },
    /// Show current auth status
    Status,
    /// Logout (remove stored tokens)
    Logout {
        /// Provider to logout from
        #[arg(default_value = "anthropic")]
        provider: String,
    },
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
enum GroupCommands {
    /// List all groups
    List,
    /// Create a new group
    Create {
        /// Group name
        name: String,
        /// Group description
        #[arg(short = 'd', long)]
        description: String,
        /// Coordination pattern (round_robin, supervisor, dynamic, pipeline)
        #[arg(short = 'p', long, default_value = "round_robin")]
        pattern: String,
    },
    /// Add an agent to a group
    AddMember {
        /// Group name
        group: String,
        /// Agent name
        agent: String,
        /// Member role (regular, supervisor, specialist)
        #[arg(long, default_value = "regular")]
        role: String,
        /// Capabilities (comma-separated)
        #[arg(long)]
        capabilities: Option<String>,
    },
    /// Show group status and members
    Status {
        /// Group name
        name: String,
    },
    /// Export group configuration (members and pattern only)
    Export {
        /// Group name to export
        name: String,
        /// Output file path (defaults to <group_name>_group.toml)
        #[arg(short = 'o', long)]
        output: Option<PathBuf>,
    },
}

#[derive(Subcommand)]
enum ExportCommands {
    /// Export an agent to a CAR file
    Agent {
        /// Agent name to export
        name: String,
        /// Output file path (defaults to <name>.car)
        #[arg(short = 'o', long)]
        output: Option<PathBuf>,
    },
    /// Export a group with all member agents to a CAR file
    Group {
        /// Group name to export
        name: String,
        /// Output file path (defaults to <name>.car)
        #[arg(short = 'o', long)]
        output: Option<PathBuf>,
    },
    /// Export entire constellation to a CAR file
    Constellation {
        /// Output file path (defaults to constellation.car)
        #[arg(short = 'o', long)]
        output: Option<PathBuf>,
    },
}

#[derive(Subcommand)]
enum FirehoseCommands {
    /// Listen to the Jetstream firehose with filters
    Listen {
        /// How many events to receive before stopping (0 for unlimited)
        #[arg(long, default_value = "10")]
        limit: usize,

        /// NSIDs to filter (e.g., app.bsky.feed.post)
        #[arg(long)]
        nsid: Vec<String>,

        /// DIDs to filter
        #[arg(long)]
        did: Vec<String>,

        /// Handles to filter mentions
        #[arg(long)]
        mention: Vec<String>,

        /// Keywords to filter
        #[arg(long)]
        keyword: Vec<String>,

        /// Languages to filter (e.g., en, ja)
        #[arg(long)]
        lang: Vec<String>,

        /// Custom Jetstream endpoint URL
        #[arg(long)]
        endpoint: Option<String>,

        /// Output format (pretty, json, raw)
        #[arg(long, default_value = "pretty")]
        format: String,
    },
    /// Test connection to Jetstream
    Test {
        /// Custom Jetstream endpoint URL
        #[arg(long)]
        endpoint: Option<String>,
    },
}

#[derive(Subcommand)]
enum AtprotoCommands {
    /// Login with app password
    Login {
        /// Your handle (e.g., alice.bsky.social) or DID
        identifier: String,
        /// App password (will prompt if not provided)
        #[arg(short = 'p', long)]
        app_password: Option<String>,
    },
    /// Login with OAuth (coming soon)
    OAuth {
        /// Your handle (e.g., alice.bsky.social) or DID
        identifier: String,
    },
    /// Show authentication status
    Status,
    /// Unlink an ATProto identity
    Unlink {
        /// Handle or DID to unlink
        identifier: String,
    },
    /// Test ATProto connections
    Test,
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
        agent: String,
    },
    /// List all core memory blocks for an agent
    ListCore {
        /// Agent name
        agent: String,
    },
    /// List all memory blocks for an agent (core + archival)
    ListAllMemory {
        /// Agent name
        agent: String,
    },
    /// Search conversation history
    SearchConversations {
        /// Agent name to search conversations for
        agent: String,
        /// Search query (optional)
        query: Option<String>,
        /// Filter by role (user, assistant, system, tool)
        #[arg(long)]
        role: Option<String>,
        /// Start time filter (ISO 8601 format)
        #[arg(long)]
        start_time: Option<String>,
        /// End time filter (ISO 8601 format)
        #[arg(long)]
        end_time: Option<String>,
        /// Maximum number of results
        #[arg(long, default_value = "20")]
        limit: usize,
    },
    /// Show the current context that would be passed to the LLM
    ShowContext {
        /// Agent name
        agent: String,
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

    // Initialize our custom tracing writer
    let tracing_writer = tracing_writer::init_tracing_writer();

    // Initialize tracing
    use tracing_subscriber::{EnvFilter, fmt};

    if cli.debug {
        // Only show debug output from pattern crates
        fmt()
            .with_env_filter(EnvFilter::new(
                "pattern_core=debug,pattern_cli=debug,pattern_nd=debug,pattern_mcp=debug,pattern_discord=debug,pattern_main=debug",
            ))
            .with_file(true)
            .with_line_number(true) // Show target module
            .with_thread_ids(false)
            .with_thread_names(false)
            .with_timer(tracing_subscriber::fmt::time::LocalTime::rfc_3339()) // Local time in RFC 3339 format
            .with_writer(tracing_writer.clone())
            .pretty()
            .init();
    } else {
        // Show info level for pattern crates, warn for everything else
        fmt()
            .with_env_filter( EnvFilter::new(
                "pattern_core=info,pattern_cli=info,pattern_nd=info,pattern_mcp=info,pattern_discord=info,pattern_main=info,warn",
            ))
            .with_target(false)
            .with_thread_ids(false)
            .with_thread_names(false)
            .with_writer(tracing_writer.clone())
            .compact()
            .init();
    };

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

    tracing::info!("Using database config: {:?}", config.database);

    // Initialize database
    client::init_db(config.database.clone()).await?;

    // Initialize groups from configuration (skip for auth/atproto/config commands to avoid API key issues)
    let skip_group_init = matches!(
        &cli.command,
        Commands::Auth { .. } | Commands::Config { .. } | Commands::Atproto { .. }
    );

    if !config.groups.is_empty() && !skip_group_init {
        // Create a heartbeat channel for group initialization
        let (heartbeat_sender, _receiver) = pattern_core::context::heartbeat::heartbeat_channel();
        commands::group::initialize_from_config(&config, heartbeat_sender).await?;
    }

    match &cli.command {
        Commands::Chat {
            agent,
            group,
            model,
            no_tools,
        } => {
            let output = crate::output::Output::new();

            // Create heartbeat channel for agent(s)
            let (heartbeat_sender, heartbeat_receiver) =
                pattern_core::context::heartbeat::heartbeat_channel();

            if let Some(group_name) = group {
                // Chat with a group
                output.success("Starting group chat mode...");
                output.info("Group:", &group_name.bright_cyan().to_string());

                // Load the group from database
                let group = ops::get_group_by_name(&DB, &config.user.id, group_name).await?;
                let group = match group {
                    Some(g) => g,
                    None => {
                        output.error(&format!("Group '{}' not found", group_name));
                        return Ok(());
                    }
                };

                // Group was already loaded with relations in get_group_by_name

                // Load all agents in the group
                tracing::info!("Group has {} members to load", group.members.len());
                let mut agents = Vec::new();
                for (mut agent_record, _membership) in group.members.clone() {
                    // Load memories and messages for the agent
                    agent_ops::load_agent_memories_and_messages(&mut agent_record).await?;

                    // Create runtime agent from record
                    let agent = agent_ops::create_agent_from_record(
                        agent_record,
                        model.clone(),
                        !*no_tools,
                        &config,
                        heartbeat_sender.clone(),
                    )
                    .await?;
                    agents.push(agent);
                }

                if agents.is_empty() {
                    output.error("No agents in group");
                    output.info(
                        "Hint:",
                        "Add agents with: pattern-cli group add-member <group> <agent>",
                    );
                    return Ok(());
                }

                // Create the appropriate pattern manager
                use pattern_core::coordination::types::CoordinationPattern;
                use pattern_core::coordination::{
                    DynamicManager, PipelineManager, RoundRobinManager, SleeptimeManager,
                    SupervisorManager, VotingManager,
                };

                match &group.coordination_pattern {
                    CoordinationPattern::RoundRobin { .. } => {
                        let manager = RoundRobinManager;
                        agent_ops::chat_with_group(group, agents, manager, heartbeat_receiver)
                            .await?;
                    }
                    CoordinationPattern::Dynamic { .. } => {
                        let manager = DynamicManager::new(Arc::new(DefaultSelectorRegistry::new()));
                        agent_ops::chat_with_group(group, agents, manager, heartbeat_receiver)
                            .await?;
                    }
                    CoordinationPattern::Pipeline { .. } => {
                        let manager = PipelineManager;
                        agent_ops::chat_with_group(group, agents, manager, heartbeat_receiver)
                            .await?;
                    }
                    CoordinationPattern::Supervisor { .. } => {
                        let manager = SupervisorManager;
                        agent_ops::chat_with_group(group, agents, manager, heartbeat_receiver)
                            .await?;
                    }
                    CoordinationPattern::Voting { .. } => {
                        let manager = VotingManager;
                        agent_ops::chat_with_group(group, agents, manager, heartbeat_receiver)
                            .await?;
                    }
                    CoordinationPattern::Sleeptime { .. } => {
                        let manager = SleeptimeManager;
                        agent_ops::chat_with_group(group, agents, manager, heartbeat_receiver)
                            .await?;
                    }
                }
            } else {
                // Chat with a single agent
                output.success("Starting chat mode...");
                output.info("Agent:", &agent.bright_cyan().to_string());
                if let Some(model_name) = &model {
                    output.info("Model:", &model_name.bright_yellow().to_string());
                }
                if !*no_tools {
                    output.info("Tools:", &"enabled".bright_green().to_string());
                } else {
                    output.info("Tools:", &"disabled".bright_red().to_string());
                }

                // Try to load existing agent or create new one
                let agent = agent_ops::load_or_create_agent(
                    agent,
                    model.clone(),
                    !*no_tools,
                    &config,
                    heartbeat_sender,
                )
                .await?;
                agent_ops::chat_with_agent(agent, heartbeat_receiver).await?;
            }
        }
        Commands::Agent { cmd } => match cmd {
            AgentCommands::List => commands::agent::list().await?,
            AgentCommands::Create { name, agent_type } => {
                commands::agent::create(name, agent_type.as_deref(), &config).await?
            }
            AgentCommands::Status { name } => commands::agent::status(name).await?,
            AgentCommands::Export { name, output } => {
                commands::agent::export(name, output.as_deref()).await?
            }
            AgentCommands::AddRule {
                agent,
                rule_type,
                tool,
                params,
                conditions,
                priority,
            } => {
                let rule_type_str = rule_type.as_deref().unwrap_or("");
                let tool_str = tool.as_deref().unwrap_or("");
                commands::agent::add_rule(
                    agent,
                    rule_type_str,
                    tool_str,
                    params.as_deref(),
                    conditions.as_deref(),
                    *priority,
                )
                .await?
            }
            AgentCommands::ListRules { agent } => commands::agent::list_rules(agent).await?,
            AgentCommands::RemoveRule {
                agent,
                tool,
                rule_type,
            } => commands::agent::remove_rule(agent, tool, rule_type.as_deref()).await?,
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
            DebugCommands::ShowContext { agent } => {
                commands::debug::show_context(&agent).await?;
            }
        },
        Commands::Config { cmd } => match cmd {
            ConfigCommands::Show => commands::config::show(&config).await?,
            ConfigCommands::Save { path } => commands::config::save(&config, path).await?,
        },
        Commands::Group { cmd } => match cmd {
            GroupCommands::List => commands::group::list(&config).await?,
            GroupCommands::Create {
                name,
                description,
                pattern,
            } => commands::group::create(name, description, pattern, &config).await?,
            GroupCommands::AddMember {
                group,
                agent,
                role,
                capabilities,
            } => {
                commands::group::add_member(group, agent, role, capabilities.as_deref(), &config)
                    .await?
            }
            GroupCommands::Status { name } => commands::group::status(name, &config).await?,
            GroupCommands::Export { name, output } => {
                commands::group::export(name, output.as_deref(), &config).await?
            }
        },
        #[cfg(feature = "oauth")]
        Commands::Auth { cmd } => match cmd {
            AuthCommands::Login { provider } => commands::auth::login(provider, &config).await?,
            AuthCommands::Status => commands::auth::status(&config).await?,
            AuthCommands::Logout { provider } => commands::auth::logout(provider, &config).await?,
        },
        Commands::Atproto { cmd } => match cmd {
            AtprotoCommands::Login {
                identifier,
                app_password,
            } => {
                commands::atproto::app_password_login(identifier, app_password.clone(), &config)
                    .await?
            }
            AtprotoCommands::OAuth { identifier } => {
                commands::atproto::oauth_login(identifier, &config).await?
            }
            AtprotoCommands::Status => commands::atproto::status(&config).await?,
            AtprotoCommands::Unlink { identifier } => {
                commands::atproto::unlink(identifier, &config).await?
            }
            AtprotoCommands::Test => commands::atproto::test(&config).await?,
        },
        Commands::Firehose { cmd } => match cmd {
            FirehoseCommands::Listen {
                limit,
                nsid,
                did,
                mention,
                keyword,
                lang,
                endpoint,
                format,
            } => {
                commands::firehose::listen(
                    *limit,
                    nsid.clone(),
                    did.clone(),
                    mention.clone(),
                    keyword.clone(),
                    lang.clone(),
                    endpoint.clone(),
                    format.clone(),
                    &config,
                )
                .await?
            }
            FirehoseCommands::Test { endpoint } => {
                commands::firehose::test_connection(endpoint.clone(), &config).await?
            }
        },
        Commands::Export { cmd } => match cmd {
            ExportCommands::Agent { name, output } => {
                commands::export::export_agent(name, output.clone(), &config).await?
            }
            ExportCommands::Group { name, output } => {
                commands::export::export_group(name, output.clone(), &config).await?
            }
            ExportCommands::Constellation { output } => {
                commands::export::export_constellation(output.clone(), &config).await?
            }
        },
        Commands::Import {
            file,
            rename_to,
            preserve_ids,
        } => {
            commands::export::import(file.clone(), rename_to.clone(), *preserve_ids, &config)
                .await?
        }
    }

    Ok(())
}
