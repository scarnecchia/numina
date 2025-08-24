//! Chat interaction loops for agents and groups
//!
//! This module contains the main chat loop functionality for both single agents
//! and agent groups, including message handling and response processing.

use miette::{IntoDiagnostic, Result};
use owo_colors::OwoColorize;
use pattern_core::{
    Agent, ToolRegistry,
    agent::ResponseEvent,
    config::PatternConfig,
    context::heartbeat::{self, HeartbeatReceiver, HeartbeatSender},
    coordination::groups::{AgentGroup, AgentWithMembership, GroupManager, GroupResponseEvent},
    data_source::{BlueskyFilter, DataSourceBuilder},
    db::{client::DB, ops},
    message::{Message, MessageContent},
    tool::builtin::DataSourceTool,
};
use std::sync::Arc;

use crate::{
    agent_ops::{
        create_agent_from_record_with_tracker, load_agent_memories_and_messages,
        load_model_embedding_providers,
    },
    data_sources::get_bluesky_credentials,
    endpoints::CliEndpoint,
    output::Output,
    slash_commands::handle_slash_command,
};

/// Chat with an agent
pub async fn chat_with_agent(
    agent: Arc<dyn Agent>,
    heartbeat_receiver: heartbeat::HeartbeatReceiver,
) -> Result<()> {
    use rustyline_async::{Readline, ReadlineEvent};
    let output = Output::new();

    output.status("Type 'quit' or 'exit' to leave the chat");
    output.status("Use Ctrl+D for multiline input, Enter to send");

    let (mut rl, writer) = Readline::new(format!("{} ", ">".bright_blue())).into_diagnostic()?;

    // Update the global tracing writer to use the SharedWriter
    crate::tracing_writer::set_shared_writer(writer.clone());

    // Create output with SharedWriter for proper concurrent output
    let output = output.with_writer(writer.clone());

    // Set up default user endpoint
    let use_discord_default = std::env::var("DISCORD_DEFAULT_USER_ENDPOINT")
        .map(|v| v.to_lowercase() == "true" || v == "1")
        .unwrap_or(false);

    if use_discord_default && std::env::var("DISCORD_TOKEN").is_ok() {
        // Discord is configured and user wants it as default
        // The Discord endpoint should already be registered
        output.info("Default endpoint:", "Discord (DMs/channels)");
    } else {
        // Register CLI endpoint as default
        let cli_endpoint = Arc::new(CliEndpoint::new(output.clone()));
        agent.set_default_user_endpoint(cli_endpoint).await?;
        output.info("Default endpoint:", "CLI");
    }

    // Use generic heartbeat processor
    let output_clone = output.clone();
    tokio::spawn(pattern_core::context::heartbeat::process_heartbeats(
        heartbeat_receiver,
        vec![agent.clone()],
        move |event, _agent_id, agent_name| {
            let output = output_clone.clone();
            async move {
                output.status(&format!("💓 Heartbeat continuation from {}:", agent_name));
                print_response_event(event, &output);
            }
        },
    ));

    loop {
        // Handle user input
        let event = rl.readline().await;
        match event {
            Ok(ReadlineEvent::Line(line)) => {
                if line.trim().is_empty() {
                    continue;
                }

                // Check for slash commands
                if line.trim().starts_with('/') {
                    match handle_slash_command(
                        &line,
                        crate::slash_commands::CommandContext::SingleAgent(&agent),
                        &output,
                    )
                    .await
                    {
                        Ok(should_exit) => {
                            if should_exit {
                                output.status("Goodbye!");
                                break;
                            }
                            continue;
                        }
                        Err(e) => {
                            output.error(&format!("Command error: {}", e));
                            continue;
                        }
                    }
                }

                if line.trim() == "quit" || line.trim() == "exit" {
                    output.status("Goodbye!");
                    break;
                }

                // Add to history
                rl.add_history_entry(line.clone());

                // Create a message using the actual Message structure
                let message = Message {
                    content: MessageContent::Text(line.clone()),
                    word_count: line.split_whitespace().count() as u32,
                    ..Default::default()
                };

                let r_agent = agent.clone();
                let output = output.clone();
                tokio::spawn(async move {
                    // Process message with streaming
                    output.status("Thinking...");

                    use tokio_stream::StreamExt;

                    match r_agent.clone().process_message_stream(message).await {
                        Ok(mut stream) => {
                            while let Some(event) = stream.next().await {
                                print_response_event(event, &output);
                            }
                        }
                        Err(e) => {
                            output.error(&format!("Error: {}", e));
                        }
                    }
                });
            }
            Ok(ReadlineEvent::Interrupted) => {
                output.status("CTRL-C");
                continue;
            }
            Ok(ReadlineEvent::Eof) => {
                output.status("CTRL-D");
                break;
            }
            Err(err) => {
                output.error(&format!("Error: {:?}", err));
                break;
            }
        }
    }

    Ok(())
}

pub fn print_response_event(event: ResponseEvent, output: &Output) {
    match event {
        ResponseEvent::ToolCallStarted {
            call_id: _,
            fn_name,
            args,
        } => {
            // For send_message, hide the content arg since it's displayed below
            let args_display = if fn_name == "send_message" {
                let mut display_args = args.clone();
                if let Some(args_obj) = display_args.as_object_mut() {
                    if args_obj.contains_key("content") {
                        args_obj.insert("content".to_string(), serde_json::json!("[shown below]"));
                    }
                }
                serde_json::to_string(&display_args).unwrap_or_else(|_| display_args.to_string())
            } else {
                serde_json::to_string_pretty(&args).unwrap_or_else(|_| args.to_string())
            };

            output.tool_call(&fn_name, &args_display);
        }
        ResponseEvent::ToolCallCompleted { call_id, result } => match result {
            Ok(content) => {
                output.tool_result(&content);
            }
            Err(error) => {
                output.error(&format!("Tool error ({}): {}", call_id, error));
            }
        },
        ResponseEvent::TextChunk { text, .. } => {
            // Display agent's response text
            output.agent_message("Agent", &text);
        }
        ResponseEvent::ReasoningChunk { text, is_final: _ } => {
            output.status(&format!("💭 Reasoning: {}", text));
        }
        ResponseEvent::ToolCalls { .. } => {
            // Skip - we handle individual ToolCallStarted events instead
        }
        ResponseEvent::ToolResponses { .. } => {
            // Skip - we handle individual ToolCallCompleted events instead
        }
        ResponseEvent::Complete {
            message_id,
            metadata,
        } => {
            // Could display metadata if desired
            tracing::debug!("Message {} complete: {:?}", message_id, metadata);
        }
        ResponseEvent::Error {
            message,
            recoverable,
        } => {
            if recoverable {
                output.warning(&format!("Recoverable error: {}", message));
            } else {
                output.error(&format!("Error: {}", message));
            }
        }
    }
}

/// Chat with a group of agents
pub async fn chat_with_group(
    group_name: &str,
    model: Option<String>,
    no_tools: bool,
    config: &PatternConfig,
) -> Result<()> {
    use rustyline_async::Readline;

    let (rl, writer) = Readline::new(format!("{} ", ">".bright_blue())).into_diagnostic()?;

    // Update the global tracing writer to use the SharedWriter
    crate::tracing_writer::set_shared_writer(writer.clone());

    // Create output with SharedWriter for proper concurrent output
    let output = Output::new().with_writer(writer.clone());

    // Use shared setup function
    let group_setup = setup_group(group_name, model, no_tools, config, &output).await?;

    let is_context_sync = group_setup.group.name == "Context Sync";

    // Handle Context Sync special case for sleeptime groups
    if is_context_sync {
        if let pattern_core::coordination::types::CoordinationPattern::Sleeptime { .. } =
            &group_setup.group.coordination_pattern
        {
            output.success("Starting Context Sync background monitoring...");

            // Start the background monitoring task
            let monitoring_handle = crate::background_tasks::start_context_sync_monitoring(
                group_setup.group.clone(),
                group_setup.agents_with_membership.clone(),
                group_setup.pattern_manager.clone(),
                output.clone(),
            )
            .await?;

            output.info(
                "Background task started",
                "Context sync will run periodically in the background",
            );

            // Don't enter interactive chat for Context Sync, just let it run
            output.status("Context Sync group is now running in background mode");
            output.status("Press Ctrl+C to stop monitoring");

            // Wait for the monitoring task to complete (or be cancelled)
            monitoring_handle.await.into_diagnostic()?;
            return Ok(());
        }
    }

    // Run normal chat loop for all other cases
    run_group_chat_loop(
        group_setup.group,
        group_setup.agents_with_membership,
        group_setup.pattern_manager,
        group_setup.heartbeat_receiver,
        output,
        rl,
    )
    .await
}

/// Shared setup for group agents including loading, memory setup, and tool registration
pub struct GroupSetup {
    pub group: AgentGroup,
    pub agents_with_membership: Vec<AgentWithMembership<Arc<dyn Agent>>>,
    pub pattern_agent: Option<Arc<dyn Agent>>,
    pub agent_tools: Vec<ToolRegistry>, // Each agent's tool registry
    pub pattern_manager: Arc<dyn GroupManager + Send + Sync>,
    #[allow(dead_code)] // Used during agent creation, kept for potential future use
    pub constellation_tracker:
        Arc<pattern_core::constellation_memory::ConstellationActivityTracker>,
    #[allow(dead_code)] // Used during agent creation, kept for potential future use
    pub heartbeat_sender: HeartbeatSender,
    pub heartbeat_receiver: HeartbeatReceiver,
}

/// Set up a group with all necessary components
pub async fn setup_group(
    group_name: &str,
    model: Option<String>,
    no_tools: bool,
    config: &PatternConfig,
    output: &Output,
) -> Result<GroupSetup> {
    // Load the group from database or create from config
    let group = ops::get_group_by_name(&DB, &config.user.id, group_name).await?;
    let group = match group {
        Some(g) => g,
        None => {
            // Check if group is defined in config
            if let Some(group_config) = config.groups.iter().find(|g| g.name == group_name) {
                output.status(&format!("Creating group '{}' from config...", group_name));

                // Convert pattern from config to coordination pattern
                let coordination_pattern = crate::commands::group::convert_pattern_config(
                    &group_config.pattern,
                    &config.user.id,
                    &group_config.members,
                )
                .await?;

                // Create the group
                let new_group = pattern_core::coordination::groups::AgentGroup {
                    id: group_config
                        .id
                        .clone()
                        .unwrap_or_else(pattern_core::id::GroupId::generate),
                    name: group_config.name.clone(),
                    description: group_config.description.clone(),
                    coordination_pattern,
                    created_at: chrono::Utc::now(),
                    updated_at: chrono::Utc::now(),
                    is_active: true,
                    state: pattern_core::coordination::types::GroupState::RoundRobin {
                        current_index: 0,
                        last_rotation: chrono::Utc::now(),
                    },
                    members: vec![],
                };

                // Create group in database
                let created = ops::create_group_for_user(&DB, &config.user.id, &new_group).await?;
                output.success(&format!("Created group: {}", created.name));

                // Add members from config
                for member_config in &group_config.members {
                    output.status(&format!("Adding member: {}", member_config.name));

                    // Load or create agent from member config
                    let agent = crate::agent_ops::load_or_create_agent_from_member(
                        member_config,
                        &config.user.id,
                        model.clone(),
                        !no_tools,
                        heartbeat::heartbeat_channel().0, // temporary sender for member creation
                        Some(config),
                    )
                    .await?;

                    // Convert role
                    let role = crate::commands::group::convert_role_config(&member_config.role);

                    // Create membership
                    let membership = pattern_core::coordination::groups::GroupMembership {
                        id: pattern_core::id::RelationId::nil(),
                        in_id: agent.id().clone(),
                        out_id: created.id.clone(),
                        joined_at: chrono::Utc::now(),
                        role,
                        is_active: true,
                        capabilities: member_config.capabilities.clone(),
                    };

                    // Add to group
                    ops::add_agent_to_group(&DB, &membership).await?;
                    output.success(&format!(
                        "Added member: {} ({:?})",
                        member_config.name, membership.role
                    ));
                }

                // Reload the group with members
                ops::get_group_by_name(&DB, &config.user.id, group_name)
                    .await?
                    .ok_or_else(|| miette::miette!("Failed to reload created group"))?
            } else {
                output.error(&format!(
                    "Group '{}' not found in database or config",
                    group_name
                ));
                return Err(miette::miette!("Group not found"));
            }
        }
    };

    // Create heartbeat channel for agents
    let (heartbeat_sender, heartbeat_receiver) = heartbeat::heartbeat_channel();

    // Create a shared constellation activity tracker for the group
    let group_id_str = group.id.to_string();
    let tracker_memory_id = pattern_core::MemoryId(group_id_str);
    let constellation_tracker = Arc::new(
        pattern_core::constellation_memory::ConstellationActivityTracker::with_memory_id(
            tracker_memory_id,
            100,
        ),
    );

    // Load all agents in the group
    tracing::info!("Group has {} members to load", group.members.len());
    let mut agents = Vec::new();
    let mut agent_tools = Vec::new(); // Track each agent's tool registry
    let mut pattern_agent_index = None;

    // Find the group config to get member config_paths
    let group_config = config.groups.iter().find(|g| g.name == group_name).cloned();

    if group_config.is_some() {
        output.info("✓", &format!("Found group config for '{}'", group_name));
    } else {
        output.warning(&format!(
            "No group config found for '{}' in config file",
            group_name
        ));
    }

    // First pass: create all agents WITHOUT data sources
    for (i, (mut agent_record, _membership)) in group.members.clone().into_iter().enumerate() {
        output.section(&format!(
            "Loading agent {}/{}: {}",
            i + 1,
            group.members.len(),
            agent_record.name.bright_cyan()
        ));

        // Load memories and messages for the agent
        load_agent_memories_and_messages(&mut agent_record, output).await?;

        // Check if this is the Pattern agent
        if agent_record.name == "Pattern" {
            pattern_agent_index = Some(i);
        }

        // Find the member config for this agent
        let member_config = group_config.as_ref().and_then(|gc| {
            output.status(&format!(
                "Searching {} group members for agent_id {}",
                gc.members.len(),
                agent_record.id
            ));
            gc.members
                .iter()
                .find(|m| m.agent_id.as_ref() == Some(&agent_record.id))
                .cloned()
        });

        if let Some(ref member) = member_config {
            output.info(
                "✓",
                &format!(
                    "Found member config for {} (has config_path: {})",
                    member.name,
                    member.config_path.is_some()
                ),
            );
        } else {
            output.warning(&format!(
                "No member config found for agent_id {}",
                agent_record.id
            ));
        }

        // Load agent config if member has a config_path
        let agent_config = if let Some(member) = &member_config {
            if let Some(config_path) = &member.config_path {
                output.status(&format!(
                    "Loading config for {} from {}",
                    agent_record.name.bright_cyan(),
                    config_path.display()
                ));
                match pattern_core::config::AgentConfig::load_from_file(config_path).await {
                    Ok(cfg) => {
                        output.success(&format!(
                            "Loaded config (has persona: {})",
                            cfg.persona.is_some()
                        ));
                        Some(cfg)
                    }
                    Err(e) => {
                        output.warning(&format!("Failed to load config: {}", e));
                        None
                    }
                }
            } else {
                output.info("ℹ", "Member has no config_path");
                None
            }
        } else {
            output.info("ℹ", "No member config, using defaults");
            None
        };

        // Build full config with loaded agent config or defaults
        let full_config = if let Some(agent_cfg) = agent_config {
            output.info(
                "📋",
                &format!(
                    "Using loaded config (persona: {} chars)",
                    agent_cfg.persona.as_ref().map(|p| p.len()).unwrap_or(0)
                ),
            );
            let model = agent_cfg.model.clone().unwrap_or(config.model.clone());
            PatternConfig {
                user: config.user.clone(),
                agent: agent_cfg,
                model,
                database: config.database.clone(),
                groups: config.groups.clone(),
                bluesky: config.bluesky.clone(),
            }
        } else {
            output.info("📋", "Using default config (no persona)");
            config.clone()
        };

        // Create agent with its own tool registry
        let tools = ToolRegistry::new();
        let agent = if agent_record.name == "Anchor" && !no_tools {
            let tools = ToolRegistry::new();
            // Create Anchor agent with its own tools
            let agent = create_agent_from_record_with_tracker(
                agent_record.clone(),
                model.clone(),
                !no_tools,
                &full_config,
                heartbeat_sender.clone(),
                Some(constellation_tracker.clone()),
                output,
                Some(tools.clone()),
            )
            .await?;

            // Add SystemIntegrityTool only to Anchor's registry
            use pattern_core::tool::builtin::SystemIntegrityTool;
            let handle = agent.handle().await;
            let integrity_tool = SystemIntegrityTool::new(handle);
            tools.register(integrity_tool);
            output.success(&format!(
                "Emergency halt tool registered for {} agent",
                "Anchor".bright_red()
            ));

            agent
        } else if agent_record.name == "Archive" && !no_tools {
            let tools = ToolRegistry::new();
            // Create Archive agent with its own tools
            let agent = create_agent_from_record_with_tracker(
                agent_record.clone(),
                model.clone(),
                !no_tools,
                &full_config,
                heartbeat_sender.clone(),
                Some(constellation_tracker.clone()),
                output,
                Some(tools.clone()),
            )
            .await?;

            // Add SystemIntegrityTool only to Anchor's registry
            use pattern_core::tool::builtin::ConstellationSearchTool;
            let handle = agent.handle().await;
            let integrity_tool = ConstellationSearchTool::new(handle);
            tools.register(integrity_tool);
            output.success(&format!(
                "Memory specialist search tool registered for {} agent",
                "Archive".bright_cyan()
            ));

            agent
        } else {
            // Create agent with its own tool registry
            create_agent_from_record_with_tracker(
                agent_record.clone(),
                model.clone(),
                !no_tools,
                &full_config,
                heartbeat_sender.clone(),
                Some(constellation_tracker.clone()),
                output,
                Some(tools.clone()),
            )
            .await?
        };

        agents.push(agent);
        agent_tools.push(tools);
    }

    if agents.is_empty() {
        output.error("No agents in group");
        output.info(
            "Hint:",
            "Add agents with: pattern-cli group add-member <group> <agent>",
        );
        return Err(miette::miette!("No agents in group"));
    }

    if pattern_agent_index.is_none() {
        output.warning("Pattern agent not found in group - Jetstream routing unavailable");
        output.info(
            "Hint:",
            "Add Pattern agent with: pattern-cli group add-member <group> Pattern",
        );
    }

    // Save pattern agent reference
    let pattern_agent = pattern_agent_index.map(|idx| agents[idx].clone());

    // Create the appropriate pattern manager based on the group's coordination pattern
    use pattern_core::coordination::selectors::DefaultSelectorRegistry;
    use pattern_core::coordination::types::CoordinationPattern;
    use pattern_core::coordination::{
        DynamicManager, PipelineManager, RoundRobinManager, SleeptimeManager, SupervisorManager,
        VotingManager,
    };

    let pattern_manager: Arc<dyn GroupManager + Send + Sync> = match &group.coordination_pattern {
        CoordinationPattern::RoundRobin { .. } => Arc::new(RoundRobinManager),
        CoordinationPattern::Dynamic { .. } => Arc::new(DynamicManager::new(Arc::new(
            DefaultSelectorRegistry::new(),
        ))),
        CoordinationPattern::Pipeline { .. } => Arc::new(PipelineManager),
        CoordinationPattern::Supervisor { .. } => Arc::new(SupervisorManager),
        CoordinationPattern::Voting { .. } => Arc::new(VotingManager),
        CoordinationPattern::Sleeptime { .. } => Arc::new(SleeptimeManager),
    };

    // Initialize group chat (registers CLI and Group endpoints)
    let agents_with_membership =
        init_group_chat(&group, agents.clone(), &pattern_manager, output).await?;

    // Check config for sleeptime groups that share the same members and start them
    // This is done here so we can reuse the already-loaded agents
    use pattern_core::config::GroupPatternConfig;
    for group_config in &config.groups {
        if group_config.name != group.name {
            if let GroupPatternConfig::Sleeptime { .. } = &group_config.pattern {
                // Check if this sleeptime group has the same members as our current group
                let our_names: std::collections::HashSet<_> =
                    group.members.iter().map(|(a, _)| &a.name).collect();
                let their_names: std::collections::HashSet<_> =
                    group_config.members.iter().map(|m| &m.name).collect();

                if our_names == their_names {
                    output.info(
                        "Starting background monitoring",
                        &format!(
                            "Detected sleeptime group '{}' with same members",
                            group_config.name
                        ),
                    );

                    // Load the sleeptime group from DB to get proper IDs and coordination pattern
                    if let Some(sleeptime_group) =
                        ops::get_group_by_name(&DB, &config.user.id, &group_config.name).await?
                    {
                        // Use members directly from the loaded group
                        let sleeptime_members = &sleeptime_group.members;

                        // Map our loaded agents to the sleeptime group's membership data
                        tracing::info!("Mapping agents to sleeptime group membership:");
                        tracing::info!(
                            "  Loaded agents: {:?}",
                            agents.iter().map(|a| a.name()).collect::<Vec<_>>()
                        );
                        tracing::info!(
                            "  Sleeptime members: {:?}",
                            sleeptime_members
                                .iter()
                                .map(|(a, _)| &a.name)
                                .collect::<Vec<_>>()
                        );

                        let sleeptime_agents: Vec<AgentWithMembership<Arc<dyn Agent>>> = agents
                            .iter()
                            .filter_map(|agent| {
                                // Find the matching membership by agent name
                                let agent_name = agent.name();
                                let found = sleeptime_members
                                    .iter()
                                    .find(|(a, _)| a.name == agent_name)
                                    .map(|(_, membership)| AgentWithMembership {
                                        agent: agent.clone(),
                                        membership: membership.clone(),
                                    });
                                if found.is_none() {
                                    tracing::warn!(
                                        "Could not find sleeptime membership for agent: {}",
                                        agent_name
                                    );
                                }
                                found
                            })
                            .collect();

                        tracing::info!(
                            "Matched {} of {} agents",
                            sleeptime_agents.len(),
                            agents.len()
                        );

                        if sleeptime_agents.len() == agents.len() {
                            // Start background monitoring with a new sleeptime manager
                            let sleeptime_manager: Arc<dyn GroupManager + Send + Sync> =
                                Arc::new(pattern_core::coordination::SleeptimeManager);
                            let monitoring_handle =
                                crate::background_tasks::start_context_sync_monitoring(
                                    sleeptime_group.clone(),
                                    sleeptime_agents,
                                    sleeptime_manager,
                                    output.clone(),
                                )
                                .await?;

                            // Spawn the monitoring task to run in background
                            tokio::spawn(async move {
                                if let Err(e) = monitoring_handle.await {
                                    tracing::error!("Background monitoring task failed: {}", e);
                                }
                            });

                            output.success(&format!(
                                "Background monitoring started for '{}'",
                                sleeptime_group.name
                            ));
                        } else {
                            output.warning(&format!(
                                "Could not match all agents for sleeptime group '{}'",
                                group_config.name
                            ));
                        }
                    }
                }
            }
        }
    }

    Ok(GroupSetup {
        group,
        agents_with_membership,
        pattern_agent,
        agent_tools,
        pattern_manager,
        constellation_tracker,
        heartbeat_sender,
        heartbeat_receiver,
    })
}

/// Initialize group chat and return the agents with membership data
pub async fn init_group_chat(
    group: &AgentGroup,
    agents: Vec<Arc<dyn Agent>>,
    pattern_manager: &Arc<dyn GroupManager + Send + Sync>,
    output: &Output,
) -> Result<Vec<AgentWithMembership<Arc<dyn Agent>>>> {
    use pattern_core::coordination::groups::AgentWithMembership;

    // Register CLI endpoint now that we have the output with SharedWriter
    let cli_endpoint = Arc::new(CliEndpoint::new(output.clone()));
    for agent in &agents {
        agent
            .set_default_user_endpoint(cli_endpoint.clone())
            .await?;
    }

    output.status(&format!(
        "Chatting with group '{}'",
        group.name.bright_cyan()
    ));
    output.info("Pattern:", &format!("{:?}", group.coordination_pattern));
    output.info("Members:", &format!("{} agents", agents.len()));
    output.status("Type 'quit' or 'exit' to leave the chat");
    output.status("Use Ctrl+D for multiline input, Enter to send");

    // Wrap agents with their membership data
    let agents_with_membership: Vec<AgentWithMembership<Arc<dyn Agent>>> = agents
        .into_iter()
        .zip(group.members.iter())
        .map(|(agent, (_, membership))| AgentWithMembership {
            agent,
            membership: membership.clone(),
        })
        .collect();

    // Register GroupCliEndpoint for routing group messages
    let group_endpoint = Arc::new(crate::endpoints::GroupCliEndpoint {
        group: group.clone(),
        agents: agents_with_membership.clone(),
        manager: pattern_manager.clone(),
        output: output.clone(),
    });

    // Register the endpoint with each agent's message router
    tracing::info!(
        "Registering group endpoint for {} agents",
        agents_with_membership.len()
    );
    for awm in &agents_with_membership {
        tracing::debug!("Registering group endpoint for agent: {}", awm.agent.name());
        awm.agent
            .register_endpoint("group".to_string(), group_endpoint.clone())
            .await?;
    }
    tracing::info!("✓ Group endpoint registered for all agents");

    Ok(agents_with_membership)
}

/// Run the group chat loop with initialized agents
pub async fn run_group_chat_loop(
    group: AgentGroup,
    agents_with_membership: Vec<AgentWithMembership<Arc<dyn Agent>>>,
    pattern_manager: Arc<dyn GroupManager + Send + Sync>,
    heartbeat_receiver: heartbeat::HeartbeatReceiver,
    output: Output,
    mut rl: rustyline_async::Readline,
) -> Result<()> {
    use rustyline_async::ReadlineEvent;

    // Clone agents for heartbeat handler
    let agents_for_heartbeat: Vec<Arc<dyn Agent>> = agents_with_membership
        .iter()
        .map(|awm| awm.agent.clone())
        .collect();

    // Use generic heartbeat processor
    let output_clone = output.clone();
    tokio::spawn(pattern_core::context::heartbeat::process_heartbeats(
        heartbeat_receiver,
        agents_for_heartbeat,
        move |event, _agent_id, agent_name| {
            let output = output_clone.clone();
            async move {
                output.status(&format!("💓 Heartbeat continuation from {}:", agent_name));
                print_response_event(event, &output);
            }
        },
    ));

    loop {
        let event = rl.readline().await;
        match event {
            Ok(ReadlineEvent::Line(line)) => {
                if line.trim().is_empty() {
                    continue;
                }

                // Check for slash commands
                if line.trim().starts_with('/') {
                    // Get the default agent (first agent in group for now)
                    let default_agent = agents_with_membership.first().map(|awm| &awm.agent);

                    match handle_slash_command(
                        &line,
                        crate::slash_commands::CommandContext::Group {
                            group: &group,
                            agents: &agents_with_membership,
                            default_agent,
                        },
                        &output,
                    )
                    .await
                    {
                        Ok(should_exit) => {
                            if should_exit {
                                output.status("Goodbye!");
                                break;
                            }
                            continue;
                        }
                        Err(e) => {
                            output.error(&format!("Command error: {}", e));
                            continue;
                        }
                    }
                }

                if line.trim() == "quit" || line.trim() == "exit" {
                    output.status("Goodbye!");
                    break;
                }

                // Add to history
                rl.add_history_entry(line.clone());

                // Create a message
                let message = Message {
                    content: MessageContent::Text(line.clone()),
                    word_count: line.split_whitespace().count() as u32,
                    ..Default::default()
                };

                // Route through the group
                output.status("Routing message through group...");
                let output = output.clone();
                let agents_with_membership = agents_with_membership.clone();
                let group = group.clone();
                let pattern_manager = pattern_manager.clone();
                tokio::spawn(async move {
                    match pattern_manager
                        .route_message(&group, &agents_with_membership, message)
                        .await
                    {
                        Ok(mut stream) => {
                            use tokio_stream::StreamExt;

                            // Process the stream of events
                            while let Some(event) = stream.next().await {
                                print_group_response_event(
                                    event,
                                    &output,
                                    &agents_with_membership,
                                    None,
                                )
                                .await;
                            }
                        }
                        Err(e) => {
                            output.error(&format!("Error routing message: {}", e));
                        }
                    }
                });
            }
            Ok(ReadlineEvent::Interrupted) => {
                output.status("CTRL-C");
                continue;
            }
            Ok(ReadlineEvent::Eof) => {
                output.status("CTRL-D");
                break;
            }
            Err(err) => {
                output.error(&format!("Error: {:?}", err));
                break;
            }
        }
    }

    Ok(())
}

pub async fn print_group_response_event(
    event: GroupResponseEvent,
    output: &Output,
    agents_with_membership: &Vec<AgentWithMembership<Arc<dyn Agent>>>,
    source_tag: Option<&str>,
) {
    match event {
        GroupResponseEvent::Started {
            pattern,
            agent_count,
            ..
        } => {
            let prefix = source_tag.map(|s| format!("[{}] ", s)).unwrap_or_default();
            output.status(&format!(
                "{}Starting {} pattern with {} agents",
                prefix, pattern, agent_count
            ));
        }
        GroupResponseEvent::AgentStarted {
            agent_name, role, ..
        } => {
            let prefix = source_tag.map(|s| format!("[{}] ", s)).unwrap_or_default();
            output.status(&format!(
                "{}Agent {} ({:?}) processing...",
                prefix, agent_name, role
            ));
        }
        GroupResponseEvent::TextChunk {
            agent_id,
            text,
            is_final,
        } => {
            if is_final || !text.is_empty() {
                // Find agent name
                let agent_name = agents_with_membership
                    .iter()
                    .find(|a| a.agent.id() == agent_id)
                    .map(|a| a.agent.name())
                    .unwrap_or("Unknown Agent".to_string());

                output.agent_message(&agent_name, &text);
            }
        }
        GroupResponseEvent::ReasoningChunk {
            agent_id,
            text,
            is_final,
        } => {
            if is_final || !text.is_empty() {
                // Find agent name
                let agent_name = agents_with_membership
                    .iter()
                    .find(|a| a.agent.id() == agent_id)
                    .map(|a| a.agent.name())
                    .unwrap_or("Unknown Agent".to_string());

                output.info(&format!("{} reasoning:", agent_name), &text);
            }
        }
        GroupResponseEvent::ToolCallStarted {
            agent_id,
            fn_name,
            args,
            ..
        } => {
            tracing::debug!(
                "CLI: Received ToolCallStarted {} from agent {}",
                fn_name,
                agent_id
            );

            // Debug: log all agent IDs
            for awm in agents_with_membership {
                tracing::debug!(
                    "Available agent: {} (ID: {})",
                    awm.agent.name(),
                    awm.agent.id()
                );
            }

            let agent_name = agents_with_membership
                .iter()
                .find(|a| a.agent.id() == agent_id)
                .map(|a| a.agent.name())
                .unwrap_or_else(|| {
                    tracing::warn!(
                        "Could not find agent with ID {} in agents_with_membership",
                        agent_id
                    );
                    "Unknown Agent".to_string()
                });

            output.status(&format!("{} calling tool: {}", agent_name, fn_name));
            let args_display = if fn_name == "send_message" {
                let mut display_args = args.clone();
                if let Some(args_obj) = display_args.as_object_mut() {
                    if args_obj.contains_key("content") {
                        args_obj.insert("content".to_string(), serde_json::json!("[shown below]"));
                    }
                }
                serde_json::to_string(&display_args).unwrap_or_else(|_| display_args.to_string())
            } else {
                serde_json::to_string_pretty(&args).unwrap_or_else(|_| args.to_string())
            };

            output.tool_call(&fn_name, &args_display);
        }
        GroupResponseEvent::ToolCallCompleted {
            agent_id: _,
            call_id,
            result,
        } => match result {
            Ok(result) => {
                output.tool_result(&result);
            }
            Err(error) => {
                output.error(&format!("Tool error (call {}): {}", call_id, error));
            }
        },
        GroupResponseEvent::AgentCompleted { agent_name, .. } => {
            output.status(&format!("{} completed", agent_name));
        }
        GroupResponseEvent::Complete { execution_time, .. } => {
            let label = source_tag
                .map(|s| format!("[{}] Execution time", s))
                .unwrap_or_else(|| "Execution time".to_string());
            output.info(&label, &format!("{:?}", execution_time));
        }
        GroupResponseEvent::Error {
            agent_id,
            message,
            recoverable,
        } => {
            let prefix = if let Some(agent_id) = agent_id {
                let agent_name = agents_with_membership
                    .iter()
                    .find(|a| a.agent.id() == agent_id)
                    .map(|a| a.agent.name())
                    .unwrap_or("Unknown Agent".to_string());
                format!("{} error", agent_name)
            } else {
                "Group error".to_string()
            };

            if recoverable {
                output.warning(&format!("{}: {}", prefix, message));
            } else {
                output.error(&format!("{}: {}", prefix, message));
            }
        }
    }
}

/// Set up a group chat with Jetstream data routing to the group
/// This creates agents for the group, registers data sources to route to the group,
/// and starts the chat interface
pub async fn chat_with_group_and_jetstream(
    group_name: &str,
    model: Option<String>,
    no_tools: bool,
    config: &PatternConfig,
) -> Result<()> {
    use rustyline_async::Readline;

    // Create readline and output ONCE here
    let (rl, writer) = Readline::new(format!("{} ", ">".bright_blue())).into_diagnostic()?;

    // Update the global tracing writer to use the SharedWriter
    crate::tracing_writer::set_shared_writer(writer.clone());

    // Create output with SharedWriter for proper concurrent output
    let output = Output::new().with_writer(writer.clone());

    // Use shared setup function
    let group_setup = setup_group(group_name, model, no_tools, config, &output).await?;

    let GroupSetup {
        group,
        agents_with_membership,
        pattern_agent,
        agent_tools,
        pattern_manager,
        constellation_tracker: _,
        heartbeat_sender: _,
        heartbeat_receiver,
    } = group_setup;
    tracing::info!("chat_with_group_and_jetstream group setup complete");

    // Now that group endpoint is registered, set up data sources if we have Pattern agent
    if let Some(pattern_agent) = pattern_agent {
        if config.bluesky.is_some() {
            output.info("Jetstream:", "Setting up data source routing to group...");

            // Set up data sources with group as target
            let group_target = pattern_core::tool::builtin::MessageTarget {
                target_type: pattern_core::tool::builtin::TargetType::Group,
                target_id: Some(group.id.to_record_id()),
            };

            // Get the embedding provider
            let embedding_provider = if let Ok((_, embedding_provider, _)) =
                load_model_embedding_providers(None, config, None, true).await
            {
                embedding_provider
            } else {
                None
            };

            // Register data sources NOW (not in a spawn) since group endpoint is ready
            if let Some(embedding_provider) = embedding_provider {
                tracing::info!(
                    "Registering data sources with group target: {:?}",
                    group_target
                );

                // Set up data sources synchronously (not in a spawn)
                let filter = config
                    .bluesky
                    .as_ref()
                    .and_then(|b| b.default_filter.as_ref())
                    .unwrap_or(&BlueskyFilter {
                        exclude_keywords: vec!["patternstop".to_string()],
                        ..Default::default()
                    })
                    .clone();

                tracing::info!("filter: {:?}", filter);

                let data_sources = DataSourceBuilder::new()
                    .with_bluesky_source("bluesky_jetstream".to_string(), filter, true)
                    .build_with_target(
                        pattern_agent.id(),
                        pattern_agent.name(),
                        DB.clone(),
                        Some(embedding_provider),
                        Some(pattern_agent.handle().await),
                        get_bluesky_credentials(&config).await,
                        group_target,
                    )
                    .await
                    .map_err(|e| miette::miette!("Failed to build data sources: {}", e))?;

                // Note: We'll register endpoints on the data source's router after creating it

                tracing::info!("Starting Jetstream monitoring...");
                data_sources
                    .start_monitoring("bluesky_jetstream")
                    .await
                    .map_err(|e| miette::miette!("Failed to start monitoring: {}", e))?;

                tracing::info!("✓ Jetstream monitoring started successfully");

                // Register endpoints on the data source's router
                let data_sources_router = data_sources.router();

                // Register the CLI endpoint as default user endpoint
                data_sources_router
                    .register_endpoint(
                        "user".to_string(),
                        Arc::new(CliEndpoint::new(output.clone())),
                    )
                    .await;

                // Register the group endpoint so it can route to the group
                let group_endpoint = Arc::new(crate::endpoints::GroupCliEndpoint {
                    group: group.clone(),
                    agents: agents_with_membership.clone(),
                    manager: pattern_manager.clone(),
                    output: output.clone(),
                });
                data_sources_router
                    .register_endpoint("group".to_string(), group_endpoint)
                    .await;

                // Register DataSourceTool on all agent tool registries
                let data_source_tool = DataSourceTool::new(Arc::new(data_sources));
                for tools in &agent_tools {
                    tools.register(data_source_tool.clone());
                }

                output.success("Jetstream routing configured for group with data source tool");
            }
        }
    }

    // Now run the chat loop
    run_group_chat_loop(
        group.clone(),
        agents_with_membership,
        pattern_manager,
        heartbeat_receiver,
        output.clone(),
        rl,
    )
    .await
}
