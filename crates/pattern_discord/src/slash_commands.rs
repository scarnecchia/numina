//! Discord slash command implementations

use miette::IntoDiagnostic;
use miette::Result;
use pattern_core::{
    Agent,
    agent::AgentRecord,
    coordination::groups::{AgentGroup, AgentWithMembership},
    db::{client::DB, ops},
};
use serenity::{
    builder::{
        CreateAttachment, CreateCommand, CreateCommandOption, CreateEmbed, CreateEmbedFooter,
        CreateInteractionResponse, CreateInteractionResponseMessage,
    },
    client::Context,
    model::{
        application::{CommandInteraction, CommandOptionType},
        colour::Colour,
    },
};
use std::sync::Arc;

/// Create all slash commands for registration
pub fn create_commands() -> Vec<CreateCommand> {
    vec![
        CreateCommand::new("help")
            .description("Show available commands")
            .dm_permission(true),
        CreateCommand::new("status")
            .description("Check agent or group status")
            .dm_permission(true)
            .add_option(
                CreateCommandOption::new(
                    CommandOptionType::String,
                    "agent",
                    "Name of the agent to check (optional)",
                )
                .required(false),
            ),
        CreateCommand::new("memory")
            .description("View or search memory blocks (DMs only)")
            .dm_permission(true)
            .default_member_permissions(serenity::model::permissions::Permissions::empty())
            .add_option(
                CreateCommandOption::new(CommandOptionType::String, "agent", "Name of the agent")
                    .required(false),
            )
            .add_option(
                CreateCommandOption::new(
                    CommandOptionType::String,
                    "block",
                    "Name of the memory block to view",
                )
                .required(false),
            ),
        CreateCommand::new("archival")
            .description("Search archival memory (DMs only)")
            .dm_permission(true)
            .default_member_permissions(serenity::model::permissions::Permissions::empty())
            .add_option(
                CreateCommandOption::new(CommandOptionType::String, "agent", "Name of the agent")
                    .required(false),
            )
            .add_option(
                CreateCommandOption::new(CommandOptionType::String, "query", "Search query")
                    .required(false),
            ),
        CreateCommand::new("context")
            .description("Show recent conversation context (DMs only)")
            .dm_permission(true)
            .default_member_permissions(serenity::model::permissions::Permissions::empty())
            .add_option(
                CreateCommandOption::new(CommandOptionType::String, "agent", "Name of the agent")
                    .required(false),
            ),
        CreateCommand::new("search")
            .description("Search conversation history (DMs only)")
            .dm_permission(true)
            .default_member_permissions(serenity::model::permissions::Permissions::empty())
            .add_option(
                CreateCommandOption::new(CommandOptionType::String, "query", "Search query")
                    .required(true),
            )
            .add_option(
                CreateCommandOption::new(CommandOptionType::String, "agent", "Name of the agent")
                    .required(false),
            ),
        CreateCommand::new("list")
            .description("List all available agents")
            .dm_permission(true),
        CreateCommand::new("permit")
            .description("Approve a pending permission request")
            .dm_permission(true)
            .add_option(
                CreateCommandOption::new(CommandOptionType::String, "id", "Request ID")
                    .required(true),
            )
            .add_option(
                CreateCommandOption::new(
                    CommandOptionType::String,
                    "mode",
                    "once | always | ttl=seconds (default: once)",
                )
                .required(false),
            ),
        CreateCommand::new("deny")
            .description("Deny a pending permission request")
            .dm_permission(true)
            .add_option(
                CreateCommandOption::new(CommandOptionType::String, "id", "Request ID")
                    .required(true),
            ),
        CreateCommand::new("permits")
            .description("List pending permission requests (admin only)")
            .dm_permission(true),
        CreateCommand::new("restart")
            .description("Restart the runtime")
            .dm_permission(true),
    ]
}

pub async fn handle_restart_command(
    ctx: &Context,
    command: &CommandInteraction,
    restart_ch: &tokio::sync::mpsc::Sender<()>,
) -> Result<()> {
    let user_id = command.user.id.get();
    if !is_authorized_user(user_id) {
        command
            .create_response(
                &ctx.http,
                CreateInteractionResponse::Message(
                    CreateInteractionResponseMessage::new()
                        .content("🚫 Not authorized to restart the entity runtime.")
                        .ephemeral(true),
                ),
            )
            .await
            .ok();
        return Ok(());
    }
    command
        .create_response(
            &ctx.http,
            CreateInteractionResponse::Message(
                CreateInteractionResponseMessage::new()
                    .content("Restarting...")
                    .ephemeral(true),
            ),
        )
        .await
        .map_err(|e| miette::miette!("Failed to send restart response: {}", e))?;

    restart_ch.send(()).await.into_diagnostic()?;

    Ok(())
}

/// Handle the /help command
pub async fn handle_help_command(
    ctx: &Context,
    command: &CommandInteraction,
    agents: Option<&[AgentWithMembership<Arc<dyn Agent>>]>,
) -> Result<()> {
    let mut embed = CreateEmbed::new()
        .title("Pattern Discord Bot Commands")
        .colour(Colour::from_rgb(100, 150, 200))
        .field(
            "General Commands",
            "`/help` - Show this help message\n\
             `/list` - List all available agents\n\
             `/status [agent]` - Check agent or group status",
            false,
        )
        .field(
            "Memory Commands",
            "`/memory [agent] [block]` - View or list memory blocks\n\
             `/archival [agent] [query]` - Search archival memory\n\
             `/context [agent]` - Show recent conversation context",
            false,
        )
        .field(
            "Search Commands",
            "`/search <query> [agent]` - Search conversation history",
            false,
        );

    // If we have group agents, show them
    if let Some(agents) = agents {
        let agent_list = agents
            .iter()
            .map(|a| format!("• **{}** - {:?}", a.agent.name(), a.membership.role))
            .collect::<Vec<_>>()
            .join("\n");

        embed = embed.field("Available Agents", agent_list, false);
        embed = embed.footer(serenity::builder::CreateEmbedFooter::new(
            "Tip: Specify agent name in commands to target specific agents",
        ));
    }

    command
        .create_response(
            &ctx.http,
            CreateInteractionResponse::Message(
                CreateInteractionResponseMessage::new()
                    .embed(embed)
                    .ephemeral(true),
            ),
        )
        .await
        .map_err(|e| miette::miette!("Failed to send help response: {}", e))?;

    Ok(())
}

/// Handle the /status command
pub async fn handle_status_command(
    ctx: &Context,
    command: &CommandInteraction,
    agents: Option<&[AgentWithMembership<Arc<dyn Agent>>]>,
    group: Option<&AgentGroup>,
) -> Result<()> {
    // Get agent name from options
    let agent_name = command
        .data
        .options
        .first()
        .and_then(|opt| opt.value.as_str());

    let mut embed = CreateEmbed::new()
        .title("Status")
        .colour(Colour::from_rgb(100, 200, 100));

    if let Some(agent_name) = agent_name {
        // Show specific agent status
        if let Some(agents) = agents {
            if let Some(agent_with_membership) =
                agents.iter().find(|a| a.agent.name() == agent_name)
            {
                let agent = &agent_with_membership.agent;

                embed = embed
                    .field("Agent", agent.name(), true)
                    .field("ID", format!("`{}`", agent.id()), true)
                    .field(
                        "Role",
                        format!("{:?}", agent_with_membership.membership.role),
                        true,
                    );

                // Try to get memory stats
                if let Ok(memory_blocks) = agent.list_memory_keys().await {
                    embed = embed.field("Memory Blocks", memory_blocks.len().to_string(), true);
                }
            } else {
                embed = embed
                    .description(format!("Agent '{}' not found", agent_name))
                    .colour(Colour::from_rgb(200, 100, 100));
            }
        } else {
            embed = embed
                .description("No agents available")
                .colour(Colour::from_rgb(200, 100, 100));
        }
    } else {
        // Show group status if available
        if let Some(group) = group {
            embed = embed.field("Group", &group.name, true).field(
                "Pattern",
                format!("{:?}", group.coordination_pattern),
                true,
            );

            if let Some(agents) = agents {
                embed = embed.field("Agents", agents.len().to_string(), true);

                let agent_list = agents
                    .iter()
                    .map(|a| format!("• {}", a.agent.name()))
                    .collect::<Vec<_>>()
                    .join("\n");

                if !agent_list.is_empty() {
                    embed = embed.field("Active Agents", agent_list, false);
                }
            }
        } else {
            embed = embed.description("Use `/status <agent_name>` to check a specific agent");
        }
    }

    command
        .create_response(
            &ctx.http,
            CreateInteractionResponse::Message(
                CreateInteractionResponseMessage::new()
                    .embed(embed)
                    .ephemeral(true),
            ),
        )
        .await
        .map_err(|e| miette::miette!("Failed to send status response: {}", e))?;

    Ok(())
}

pub async fn handle_permit(ctx: &Context, command: &CommandInteraction) -> Result<()> {
    let user_id = command.user.id.get();
    if !is_authorized_user(user_id) {
        command
            .create_response(
                &ctx.http,
                CreateInteractionResponse::Message(
                    CreateInteractionResponseMessage::new()
                        .content("🚫 Not authorized to approve requests.")
                        .ephemeral(true),
                ),
            )
            .await
            .ok();
        return Ok(());
    }

    let id = command
        .data
        .options
        .iter()
        .find(|o| o.name == "id")
        .and_then(|o| o.value.as_str())
        .unwrap_or("");
    let mode = command
        .data
        .options
        .iter()
        .find(|o| o.name == "mode")
        .and_then(|o| o.value.as_str());

    let decision = match mode.unwrap_or("once").to_lowercase().as_str() {
        "once" => pattern_core::permission::PermissionDecisionKind::ApproveOnce,
        "always" | "scope" => pattern_core::permission::PermissionDecisionKind::ApproveForScope,
        s if s.starts_with("ttl=") => {
            let secs: u64 = s[4..].parse().unwrap_or(600);
            pattern_core::permission::PermissionDecisionKind::ApproveForDuration(
                std::time::Duration::from_secs(secs),
            )
        }
        _ => pattern_core::permission::PermissionDecisionKind::ApproveOnce,
    };

    let ok = pattern_core::permission::broker()
        .resolve(id, decision)
        .await;
    let content = if ok {
        format!("✅ Approved request {}", id)
    } else {
        format!("⚠️ Unknown request id {}", id)
    };

    command
        .create_response(
            &ctx.http,
            CreateInteractionResponse::Message(
                CreateInteractionResponseMessage::new()
                    .content(content)
                    .ephemeral(true),
            ),
        )
        .await
        .ok();

    Ok(())
}

pub async fn handle_deny(ctx: &Context, command: &CommandInteraction) -> Result<()> {
    let user_id = command.user.id.get();
    if !is_authorized_user(user_id) {
        command
            .create_response(
                &ctx.http,
                CreateInteractionResponse::Message(
                    CreateInteractionResponseMessage::new()
                        .content("🚫 Not authorized to deny requests.")
                        .ephemeral(true),
                ),
            )
            .await
            .ok();
        return Ok(());
    }

    let id = command
        .data
        .options
        .iter()
        .find(|o| o.name == "id")
        .and_then(|o| o.value.as_str())
        .unwrap_or("");

    let ok = pattern_core::permission::broker()
        .resolve(id, pattern_core::permission::PermissionDecisionKind::Deny)
        .await;
    let content = if ok {
        format!("🚫 Denied request {}", id)
    } else {
        format!("⚠️ Unknown request id {}", id)
    };

    command
        .create_response(
            &ctx.http,
            CreateInteractionResponse::Message(
                CreateInteractionResponseMessage::new()
                    .content(content)
                    .ephemeral(true),
            ),
        )
        .await
        .ok();

    Ok(())
}

pub async fn handle_permits(ctx: &Context, command: &CommandInteraction) -> Result<()> {
    let user_id = command.user.id.get();
    if !is_authorized_user(user_id) {
        command
            .create_response(
                &ctx.http,
                CreateInteractionResponse::Message(
                    CreateInteractionResponseMessage::new()
                        .content("🚫 Not authorized to view permits.")
                        .ephemeral(true),
                ),
            )
            .await
            .ok();
        return Ok(());
    }

    let pending = pattern_core::permission::broker().list_pending().await;
    let mut lines = Vec::new();
    for req in pending.iter().take(25) {
        let agent_name = req
            .metadata
            .as_ref()
            .and_then(|m| m.get("agent_name").and_then(|v| v.as_str()))
            .unwrap_or("(unknown)");
        lines.push(format!("• {} — {} — {}", req.id, agent_name, req.tool_name));
    }
    if lines.is_empty() {
        lines.push("No pending permission requests.".to_string());
    }

    command
        .create_response(
            &ctx.http,
            CreateInteractionResponse::Message(
                CreateInteractionResponseMessage::new()
                    .content(lines.join("\n"))
                    .ephemeral(true),
            ),
        )
        .await
        .ok();

    Ok(())
}
// ===== Permission approvals =====

fn is_authorized_user(user_id: u64) -> bool {
    // Support DISCORD_ADMIN_USERS as comma-separated list
    if let Ok(v) = std::env::var("DISCORD_ADMIN_USERS") {
        let ok = v
            .split(',')
            .map(|s| s.trim())
            .any(|s| s == user_id.to_string());
        if ok {
            return true;
        }
    }
    // Also support DISCORD_DEFAULT_DM_USER as single or comma-separated
    if let Ok(v) = std::env::var("DISCORD_DEFAULT_DM_USER") {
        let ok = v
            .split(',')
            .map(|s| s.trim())
            .any(|s| s == user_id.to_string());
        if ok {
            return true;
        }
    }
    false
}

// duplicate block removed

/// Handle the /memory command
pub async fn handle_memory_command(
    ctx: &Context,
    command: &CommandInteraction,
    agents: Option<&[AgentWithMembership<Arc<dyn Agent>>]>,
) -> Result<()> {
    // Check if command is in DM - reject if not
    if command.guild_id.is_some() {
        command
            .create_response(
                &ctx.http,
                CreateInteractionResponse::Message(
                    CreateInteractionResponseMessage::new()
                        .content("🔒 This command is only available in DMs for privacy.")
                        .ephemeral(true),
                ),
            )
            .await
            .map_err(|e| miette::miette!("Failed to send DM-only response: {}", e))?;
        return Ok(());
    }
    // Get parameters
    let agent_name = command
        .data
        .options
        .iter()
        .find(|opt| opt.name == "agent")
        .and_then(|opt| opt.value.as_str());

    let block_name = command
        .data
        .options
        .iter()
        .find(|opt| opt.name == "block")
        .and_then(|opt| opt.value.as_str());

    let mut embed = CreateEmbed::new()
        .title("Memory Blocks")
        .colour(Colour::from_rgb(150, 100, 200));

    // Find the agent
    let agent = if let Some(agent_name) = agent_name {
        agents.and_then(|agents| {
            agents
                .iter()
                .find(|a| a.agent.name() == agent_name)
                .map(|a| &a.agent)
        })
    } else {
        // Use default agent (Pattern or first)
        agents.and_then(|agents| {
            // Prefer supervisor-role agent as default, else first
            let supervisor = agents.iter().find(|a| {
                matches!(
                    a.membership.role,
                    pattern_core::coordination::types::GroupMemberRole::Supervisor
                )
            });
            supervisor.or_else(|| agents.first()).map(|a| &a.agent)
        })
    };

    if let Some(agent) = agent {
        embed = embed.field("Agent", agent.name(), true);

        if let Some(block_name) = block_name {
            // Show specific block
            match agent.get_memory(block_name).await {
                Ok(Some(block)) => {
                    embed = embed.field("Label", block.label.as_str(), true).field(
                        "Size",
                        format!("{} chars", block.value.len()),
                        true,
                    );

                    if let Some(desc) = &block.description {
                        embed = embed.field("Description", desc, false);
                    }

                    // Handle long content with file attachment
                    if block.value.len() > 800 {
                        // Create file attachment for long content
                        let filename = format!("{}-{}.txt", agent.name(), block.label);
                        let attachment = CreateAttachment::bytes(block.value.as_bytes(), &filename);

                        embed = embed.field("Content", "📎 See attached file", false);

                        command
                            .create_response(
                                &ctx.http,
                                CreateInteractionResponse::Message(
                                    CreateInteractionResponseMessage::new()
                                        .embed(embed)
                                        .add_file(attachment)
                                        .ephemeral(true),
                                ),
                            )
                            .await
                            .map_err(|e| {
                                miette::miette!("Failed to send memory response: {}", e)
                            })?;
                        return Ok(());
                    } else {
                        embed = embed.field("Content", format!("```\n{}\n```", block.value), false);
                    }
                }
                Ok(None) => {
                    embed = embed
                        .description(format!("Memory block '{}' not found", block_name))
                        .colour(Colour::from_rgb(200, 100, 100));
                }
                Err(e) => {
                    embed = embed
                        .description(format!("Error: {}", e))
                        .colour(Colour::from_rgb(200, 100, 100));
                }
            }
        } else {
            // List all blocks
            match agent.list_memory_keys().await {
                Ok(blocks) => {
                    if blocks.is_empty() {
                        embed = embed.description("No memory blocks found");
                    } else {
                        let block_list = blocks
                            .iter()
                            .map(|b| format!("• `{}`", b))
                            .collect::<Vec<_>>()
                            .join("\n");

                        embed = embed.field("Available Blocks", block_list, false).footer(
                            CreateEmbedFooter::new(
                                "Use /memory <agent> <block_name> to view a specific block",
                            ),
                        );
                    }
                }
                Err(e) => {
                    embed = embed
                        .description(format!("Error: {}", e))
                        .colour(Colour::from_rgb(200, 100, 100));
                }
            }
        }
    } else {
        embed = embed
            .description("No agent available")
            .colour(Colour::from_rgb(200, 100, 100));
    }

    command
        .create_response(
            &ctx.http,
            CreateInteractionResponse::Message(
                CreateInteractionResponseMessage::new()
                    .embed(embed)
                    .ephemeral(true),
            ),
        )
        .await
        .map_err(|e| miette::miette!("Failed to send memory response: {}", e))?;

    Ok(())
}

/// Handle the /archival command
pub async fn handle_archival_command(
    ctx: &Context,
    command: &CommandInteraction,
    agents: Option<&[AgentWithMembership<Arc<dyn Agent>>]>,
) -> Result<()> {
    // Check if command is in DM - reject if not
    if command.guild_id.is_some() {
        command
            .create_response(
                &ctx.http,
                CreateInteractionResponse::Message(
                    CreateInteractionResponseMessage::new()
                        .content("🔒 This command is only available in DMs for privacy.")
                        .ephemeral(true),
                ),
            )
            .await
            .map_err(|e| miette::miette!("Failed to send DM-only response: {}", e))?;
        return Ok(());
    }
    // Get parameters
    let agent_name = command
        .data
        .options
        .iter()
        .find(|opt| opt.name == "agent")
        .and_then(|opt| opt.value.as_str());

    let query = command
        .data
        .options
        .iter()
        .find(|opt| opt.name == "query")
        .and_then(|opt| opt.value.as_str());

    let mut embed = CreateEmbed::new()
        .title("Archival Memory")
        .colour(Colour::from_rgb(200, 150, 100));

    // Find the agent
    let agent = if let Some(agent_name) = agent_name {
        agents.and_then(|agents| {
            agents
                .iter()
                .find(|a| a.agent.name() == agent_name)
                .map(|a| &a.agent)
        })
    } else {
        agents.and_then(|agents| {
            // Prefer supervisor-role agent as default, else first
            let supervisor = agents.iter().find(|a| {
                matches!(
                    a.membership.role,
                    pattern_core::coordination::types::GroupMemberRole::Supervisor
                )
            });
            supervisor.or_else(|| agents.first()).map(|a| &a.agent)
        })
    };

    if let Some(agent) = agent {
        embed = embed.field("Agent", agent.name(), true);

        if let Some(query) = query {
            // Search archival memory
            let handle = agent.handle().await;
            match handle.search_archival_memories(query, 5).await {
                Ok(results) => {
                    if results.is_empty() {
                        embed = embed.description(format!(
                            "No archival memories found matching '{}'",
                            query
                        ));
                    } else {
                        embed = embed.field("Results", results.len().to_string(), true);

                        for (i, memory) in results.iter().enumerate().take(3) {
                            let preview = if memory.value.len() > 200 {
                                format!("{}...", &memory.value[..200])
                            } else {
                                memory.value.clone()
                            };

                            embed = embed.field(
                                format!("{}. {}", i + 1, memory.label),
                                format!("```\n{}\n```", preview),
                                false,
                            );
                        }

                        if results.len() > 3 {
                            embed = embed.footer(CreateEmbedFooter::new(format!(
                                "... and {} more results",
                                results.len() - 3
                            )));
                        }
                    }
                }
                Err(e) => {
                    embed = embed
                        .description(format!("Error: {}", e))
                        .colour(Colour::from_rgb(200, 100, 100));
                }
            }
        } else {
            // Show count
            let handle = agent.handle().await;
            match handle.count_archival_memories().await {
                Ok(count) => {
                    embed = embed
                        .field("Total Entries", count.to_string(), true)
                        .footer(CreateEmbedFooter::new(
                            "Use /archival <agent> <query> to search",
                        ));
                }
                Err(e) => {
                    embed = embed
                        .description(format!("Error: {}", e))
                        .colour(Colour::from_rgb(200, 100, 100));
                }
            }
        }
    } else {
        embed = embed
            .description("No agent available")
            .colour(Colour::from_rgb(200, 100, 100));
    }

    command
        .create_response(
            &ctx.http,
            CreateInteractionResponse::Message(
                CreateInteractionResponseMessage::new()
                    .embed(embed)
                    .ephemeral(true),
            ),
        )
        .await
        .map_err(|e| miette::miette!("Failed to send archival response: {}", e))?;

    Ok(())
}

/// Handle the /context command
pub async fn handle_context_command(
    ctx: &Context,
    command: &CommandInteraction,
    agents: Option<&[AgentWithMembership<Arc<dyn Agent>>]>,
) -> Result<()> {
    // Check if command is in DM - reject if not
    if command.guild_id.is_some() {
        command
            .create_response(
                &ctx.http,
                CreateInteractionResponse::Message(
                    CreateInteractionResponseMessage::new()
                        .content("🔒 This command is only available in DMs for privacy.")
                        .ephemeral(true),
                ),
            )
            .await
            .map_err(|e| miette::miette!("Failed to send DM-only response: {}", e))?;
        return Ok(());
    }
    // Get agent name
    let agent_name = command
        .data
        .options
        .first()
        .and_then(|opt| opt.value.as_str());

    let mut embed = CreateEmbed::new()
        .title("Conversation Context")
        .colour(Colour::from_rgb(100, 150, 150));

    // Find the agent
    let agent = if let Some(agent_name) = agent_name {
        agents.and_then(|agents| {
            agents
                .iter()
                .find(|a| a.agent.name() == agent_name)
                .map(|a| &a.agent)
        })
    } else {
        agents.and_then(|agents| {
            agents
                .iter()
                .find(|a| a.agent.name() == "Pattern")
                .or_else(|| agents.first())
                .map(|a| &a.agent)
        })
    };

    if let Some(agent) = agent {
        embed = embed.field("Agent", agent.name(), true);

        let handle = agent.handle().await;
        match handle
            .search_conversations(None, None, None, None, 100)
            .await
        {
            Ok(messages) => {
                if messages.is_empty() {
                    embed = embed.description("No messages in context");
                } else {
                    embed = embed.field("Recent Messages", messages.len().to_string(), true);

                    // Handle large message lists with file attachment
                    if messages.len() > 10 {
                        // Create file attachment for full context
                        let mut content_lines = Vec::new();
                        for (i, msg) in messages.iter().rev().enumerate() {
                            let role = format!("{:?}", msg.role);
                            let content = msg.display_content();
                            content_lines.push(format!("{}. [{}] {}", i + 1, role, content));
                        }

                        let filename = format!("{}-context.txt", agent.name());
                        let content = content_lines.join("\n\n");
                        let attachment = CreateAttachment::bytes(content.as_bytes(), &filename);

                        embed =
                            embed.field("Context", "📎 See attached file for full context", false);

                        command
                            .create_response(
                                &ctx.http,
                                CreateInteractionResponse::Message(
                                    CreateInteractionResponseMessage::new()
                                        .embed(embed)
                                        .add_file(attachment)
                                        .ephemeral(true),
                                ),
                            )
                            .await
                            .map_err(|e| {
                                miette::miette!("Failed to send context response: {}", e)
                            })?;
                        return Ok(());
                    } else {
                        // Show last few messages inline
                        for (i, msg) in messages.iter().rev().enumerate().take(10) {
                            let role = format!("{:?}", msg.role);
                            let content = msg.display_content();
                            let preview = if content.len() > 200 {
                                let content: String = content.chars().take(200).collect();
                                format!("{}...", content)
                            } else {
                                content
                            };

                            embed = embed.field(format!("{}. [{}]", i + 1, role), preview, false);
                        }
                    }
                }
            }
            Err(e) => {
                embed = embed
                    .description(format!("Error: {}", e))
                    .colour(Colour::from_rgb(200, 100, 100));
            }
        }
    } else {
        embed = embed
            .description("No agent available")
            .colour(Colour::from_rgb(200, 100, 100));
    }

    command
        .create_response(
            &ctx.http,
            CreateInteractionResponse::Message(
                CreateInteractionResponseMessage::new()
                    .embed(embed)
                    .ephemeral(true),
            ),
        )
        .await
        .map_err(|e| miette::miette!("Failed to send context response: {}", e))?;

    Ok(())
}

/// Handle the /search command
pub async fn handle_search_command(
    ctx: &Context,
    command: &CommandInteraction,
    agents: Option<&[AgentWithMembership<Arc<dyn Agent>>]>,
) -> Result<()> {
    // Check if command is in DM - reject if not
    if command.guild_id.is_some() {
        command
            .create_response(
                &ctx.http,
                CreateInteractionResponse::Message(
                    CreateInteractionResponseMessage::new()
                        .content("🔒 This command is only available in DMs for privacy.")
                        .ephemeral(true),
                ),
            )
            .await
            .map_err(|e| miette::miette!("Failed to send DM-only response: {}", e))?;
        return Ok(());
    }
    // Get parameters
    let query = command
        .data
        .options
        .iter()
        .find(|opt| opt.name == "query")
        .and_then(|opt| opt.value.as_str())
        .unwrap_or("");

    let agent_name = command
        .data
        .options
        .iter()
        .find(|opt| opt.name == "agent")
        .and_then(|opt| opt.value.as_str());

    let mut embed = CreateEmbed::new()
        .title("Search Results")
        .colour(Colour::from_rgb(150, 150, 100));

    // Find the agent
    let agent = if let Some(agent_name) = agent_name {
        agents.and_then(|agents| {
            agents
                .iter()
                .find(|a| a.agent.name() == agent_name)
                .map(|a| &a.agent)
        })
    } else {
        agents.and_then(|agents| {
            agents
                .iter()
                .find(|a| a.agent.name() == "Pattern")
                .or_else(|| agents.first())
                .map(|a| &a.agent)
        })
    };

    if let Some(agent) = agent {
        embed =
            embed
                .field("Agent", agent.name(), true)
                .field("Query", format!("`{}`", query), true);

        let handle = agent.handle().await;
        match handle
            .search_conversations(Some(query), None, None, None, 5)
            .await
        {
            Ok(messages) => {
                if messages.is_empty() {
                    embed = embed.description(format!("No messages found matching '{}'", query));
                } else {
                    embed = embed.field("Results", messages.len().to_string(), true);

                    // Handle large result sets with file attachment
                    if messages.len() > 5 {
                        // Create file attachment for full search results
                        let mut content_lines = Vec::new();
                        for (i, msg) in messages.iter().enumerate() {
                            let role = format!("{:?}", msg.role);
                            let content = msg
                                .text_content()
                                .unwrap_or_else(|| "(no text content)".to_string());
                            content_lines.push(format!(
                                "{}. [{}] - {}\n{}",
                                i + 1,
                                role,
                                msg.created_at.format("%Y-%m-%d %H:%M"),
                                content
                            ));
                        }

                        let filename =
                            format!("{}-search-{}.txt", agent.name(), query.replace(" ", "_"));
                        let content = content_lines.join("\n\n---\n\n");
                        let attachment = CreateAttachment::bytes(content.as_bytes(), &filename);

                        embed = embed.field(
                            "Search Results",
                            "📎 See attached file for full results",
                            false,
                        );

                        command
                            .create_response(
                                &ctx.http,
                                CreateInteractionResponse::Message(
                                    CreateInteractionResponseMessage::new()
                                        .embed(embed)
                                        .add_file(attachment)
                                        .ephemeral(true),
                                ),
                            )
                            .await
                            .map_err(|e| {
                                miette::miette!("Failed to send search response: {}", e)
                            })?;
                        return Ok(());
                    } else {
                        // Show results inline
                        for (i, msg) in messages.iter().enumerate().take(5) {
                            let role = format!("{:?}", msg.role);
                            let content = msg
                                .text_content()
                                .unwrap_or_else(|| "(no text content)".to_string());
                            let preview = if content.len() > 200 {
                                format!("{}...", &content[..200])
                            } else {
                                content
                            };

                            embed = embed.field(
                                format!(
                                    "{}. [{}] - {}",
                                    i + 1,
                                    role,
                                    msg.created_at.format("%Y-%m-%d %H:%M")
                                ),
                                preview,
                                false,
                            );
                        }
                    }
                }
            }
            Err(e) => {
                embed = embed
                    .description(format!("Error: {}", e))
                    .colour(Colour::from_rgb(200, 100, 100));
            }
        }
    } else {
        embed = embed
            .description("No agent available")
            .colour(Colour::from_rgb(200, 100, 100));
    }

    command
        .create_response(
            &ctx.http,
            CreateInteractionResponse::Message(
                CreateInteractionResponseMessage::new()
                    .embed(embed)
                    .ephemeral(true),
            ),
        )
        .await
        .map_err(|e| miette::miette!("Failed to send search response: {}", e))?;

    Ok(())
}

/// Handle the /list command
pub async fn handle_list_command(ctx: &Context, command: &CommandInteraction) -> Result<()> {
    let mut embed = CreateEmbed::new()
        .title("Available Agents")
        .colour(Colour::from_rgb(100, 200, 150));

    match ops::list_entities::<AgentRecord, _>(&DB).await {
        Ok(agents) => {
            if agents.is_empty() {
                embed = embed.description("No agents found");
            } else {
                let agent_list = agents
                    .iter()
                    .map(|a| format!("• **{}** - `{}`", a.name, a.id))
                    .collect::<Vec<_>>()
                    .join("\n");

                embed = embed
                    .field("Agents", agent_list, false)
                    .footer(CreateEmbedFooter::new(format!(
                        "Total: {} agents",
                        agents.len()
                    )));
            }
        }
        Err(e) => {
            embed = embed
                .description(format!("Error: {}", e))
                .colour(Colour::from_rgb(200, 100, 100));
        }
    }

    command
        .create_response(
            &ctx.http,
            CreateInteractionResponse::Message(
                CreateInteractionResponseMessage::new()
                    .embed(embed)
                    .ephemeral(true),
            ),
        )
        .await
        .map_err(|e| miette::miette!("Failed to send list response: {}", e))?;

    Ok(())
}
