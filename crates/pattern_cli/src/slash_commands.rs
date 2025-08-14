//! Slash command handling for interactive chat modes

use miette::Result;
use owo_colors::OwoColorize;
use pattern_core::{
    Agent,
    agent::AgentRecord,
    coordination::groups::{AgentGroup, AgentWithMembership},
    db::{client::DB, ops},
};
use std::sync::Arc;

use crate::{message_display, output::Output};

/// Context for slash command execution
pub enum CommandContext<'a> {
    SingleAgent(&'a Arc<dyn Agent>),
    Group {
        #[allow(dead_code)]
        group: &'a AgentGroup,
        agents: &'a [AgentWithMembership<Arc<dyn Agent>>],
        default_agent: Option<&'a Arc<dyn Agent>>,
    },
}

impl<'a> CommandContext<'a> {
    /// Get an agent by name or use the default/current agent
    async fn get_agent(&self, name: Option<&str>) -> Result<Arc<dyn Agent>> {
        match self {
            CommandContext::SingleAgent(agent) => {
                if name.is_some() {
                    return Err(miette::miette!("Cannot specify agent in single agent chat"));
                }
                Ok((*agent).clone())
            }
            CommandContext::Group { agents, default_agent, .. } => {
                if let Some(agent_name) = name {
                    // Find agent in group by name
                    for agent_with_membership in *agents {
                        if agent_with_membership.agent.name() == agent_name {
                            return Ok(agent_with_membership.agent.clone());
                        }
                    }
                    Err(miette::miette!("Agent '{}' not found in group", agent_name))
                } else if let Some(agent) = default_agent {
                    Ok((*agent).clone())
                } else {
                    Err(miette::miette!("No default agent available, please specify an agent"))
                }
            }
        }
    }

    /// List available agents
    fn list_agents(&self) -> Vec<String> {
        match self {
            CommandContext::SingleAgent(agent) => vec![agent.name().to_string()],
            CommandContext::Group { agents, .. } => {
                agents.iter().map(|a| a.agent.name().to_string()).collect()
            }
        }
    }
}

/// Handle slash commands in chat
pub async fn handle_slash_command(
    command: &str,
    context: CommandContext<'_>,
    output: &Output,
) -> Result<bool> {
    let parts: Vec<&str> = command.trim().split_whitespace().collect();
    if parts.is_empty() {
        return Ok(false);
    }

    match parts[0] {
        "/help" | "/?" => {
            output.section("Available Commands");
            output.list_item("/help or /? - Show this help message");
            output.list_item("/exit or /quit - Exit the chat");
            output.list_item("/list - List all agents");
            output.list_item("/status [agent] - Show agent status");
            output.print("");

            // Show available agents in group chat
            if let CommandContext::Group { agents, .. } = &context {
                output.section("Agents in Group");
                for agent_with_membership in *agents {
                    output.list_item(&format!(
                        "{} - Role: {:?}",
                        agent_with_membership.agent.name().bright_cyan(),
                        agent_with_membership.membership.role
                    ));
                }
                output.print("");
                output.status("Tip: Specify agent name with commands, e.g., /memory Pattern system");
                output.print("");
            }

            output.section("Memory Commands");
            output.list_item("/memory [agent] [block] - List or show memory blocks");
            output.list_item("/archival [agent] [query] - Search archival memory");
            output.list_item("/context [agent] - Show conversation context");
            output.list_item("/search [agent] <query> - Search conversation history");
            output.print("");
            output.section("Database Commands");
            output.list_item("/query <sql> - Run a database query");
            output.print("");
            Ok(false)
        }
        "/exit" | "/quit" => Ok(true),
        "/list" => {
            output.status("Fetching agent list...");
            match ops::list_entities::<AgentRecord, _>(&DB).await {
                Ok(agents) => {
                    if agents.is_empty() {
                        output.status("No agents found");
                    } else {
                        output.section("Available Agents");
                        for agent in agents {
                            output.list_item(&format!(
                                "{} ({})",
                                agent.name.bright_cyan(),
                                agent.id.to_string().dimmed()
                            ));
                        }
                    }
                }
                Err(e) => {
                    output.error(&format!("Failed to list agents: {}", e));
                }
            }
            Ok(false)
        }
        "/status" => {
            // Parse optional agent name for group context
            let agent_name = if parts.len() > 1 { Some(parts[1]) } else { None };

            match context.get_agent(agent_name).await {
                Ok(agent) => {
                    output.section("Agent Status");
                    output.kv("Name", &agent.name().bright_cyan().to_string());
                    output.kv("ID", &agent.id().to_string().dimmed().to_string());

                    // Try to get memory stats
                    match agent.list_memory_keys().await {
                        Ok(memory_blocks) => {
                            output.kv("Memory blocks", &memory_blocks.len().to_string());
                        }
                        Err(_) => {
                            output.kv("Memory blocks", "error loading");
                        }
                    }
                }
                Err(e) => {
                    output.error(&format!("Failed to get agent: {}", e));
                }
            }

            output.print("");
            Ok(false)
        }
        "/memory" => {
            // Parse command format: /memory [agent_name] [block_name]
            let (agent_name, block_name) = if parts.len() == 1 {
                (None, None)
            } else if parts.len() == 2 {
                // Could be agent name or block name - try to determine
                // If it matches an agent name in group, treat as agent name
                if context.list_agents().contains(&parts[1].to_string()) {
                    (Some(parts[1]), None)
                } else {
                    (None, Some(parts[1..].join(" ")))
                }
            } else {
                // parts[1] is agent name, rest is block name
                (Some(parts[1]), Some(parts[2..].join(" ")))
            };

            match context.get_agent(agent_name).await {
                Ok(agent) => {
                    if let Some(block_name) = block_name {
                        // Show specific block content
                        match agent.get_memory(&block_name).await {
                            Ok(Some(block)) => {
                                output.section(&format!("Memory Block: {}", block_name));
                                output.kv("Label", &block.label.to_string());
                                output.kv("Characters", &block.value.len().to_string());
                                if let Some(desc) = &block.description {
                                    output.kv("Description", desc);
                                }
                                output.print("");
                                output.print(&block.value);
                            }
                            Ok(None) => {
                                output.error(&format!("Memory block '{}' not found", block_name));
                            }
                            Err(e) => {
                                output.error(&format!("Failed to get memory block: {}", e));
                            }
                        }
                    } else {
                        // List all blocks
                        match agent.list_memory_keys().await {
                            Ok(blocks) => {
                                output.section(&format!("Memory Blocks for {}", agent.name()));
                                if blocks.is_empty() {
                                    output.status("No memory blocks found");
                                } else {
                                    for block_name in blocks {
                                        output.list_item(&block_name.bright_cyan().to_string());
                                    }
                                }
                            }
                            Err(e) => {
                                output.error(&format!("Failed to list memory blocks: {}", e));
                            }
                        }
                    }
                }
                Err(e) => {
                    output.error(&format!("Failed to get agent: {}", e));
                }
            }
            Ok(false)
        }
        "/archival" => {
            // Parse command format: /archival [agent_name] [query]
            let (agent_name, query) = if parts.len() == 1 {
                (None, None)
            } else if parts.len() == 2 {
                // Could be agent name or query - try to determine
                if context.list_agents().contains(&parts[1].to_string()) {
                    (Some(parts[1]), None)
                } else {
                    (None, Some(parts[1..].join(" ")))
                }
            } else {
                // parts[1] is agent name, rest is query
                (Some(parts[1]), Some(parts[2..].join(" ")))
            };

            match context.get_agent(agent_name).await {
                Ok(agent) => {
                    if let Some(search_query) = query {
                        // Search archival memory
                        output.status(&format!("Searching archival memory for: {}", search_query));

                        let handle = agent.handle().await;
                        match handle.search_archival_memories(&search_query, 10).await {
                            Ok(results) => {
                                if results.is_empty() {
                                    output.status("No matching archival memories found");
                                } else {
                                    output.section(&format!("Found {} archival memories", results.len()));
                                    for memory in results {
                                        output.list_item(&format!(
                                            "{}: {} ({})",
                                            memory.label.bright_cyan(),
                                            memory.value.chars().take(100).collect::<String>(),
                                            memory.value.len().to_string().dimmed()
                                        ));
                                    }
                                }
                            }
                            Err(e) => {
                                output.error(&format!("Failed to search archival memory: {}", e));
                            }
                        }
                    } else {
                        // List archival count
                        let handle = agent.handle().await;
                        match handle.count_archival_memories().await {
                            Ok(count) => {
                                output.section(&format!("Archival Memory for {}", agent.name()));
                                output.kv("Total entries", &count.to_string());
                                output.status("Use /archival <query> to search archival memory");
                            }
                            Err(e) => {
                                output.error(&format!("Failed to count archival memory: {}", e));
                            }
                        }
                    }
                }
                Err(e) => {
                    output.error(&format!("Failed to get agent: {}", e));
                }
            }
            Ok(false)
        }
        "/context" => {
            // Parse optional agent name for group context
            let agent_name = if parts.len() > 1 { Some(parts[1]) } else { None };

            match context.get_agent(agent_name).await {
                Ok(agent) => {
                    output.section(&format!("Conversation Context for {}", agent.name()));

                    // Get the agent's handle to access conversation history
                    let handle = agent.handle().await;

                    // Search recent messages without a query to get context
                    match handle.search_conversations(None, None, None, None, 20).await {
                        Ok(messages) => {
                            if messages.is_empty() {
                                output.status("No messages in context");
                            } else {
                                output.status(&format!("Showing {} recent messages", messages.len()));
                                output.status("");  // Empty line for spacing

                                // Display messages in chronological order (reverse since they come newest first)
                                for msg in messages.iter().rev() {
                                    message_display::display_message(msg, &output, false, Some(200));
                                    output.status("");  // Empty line between messages
                                }
                            }
                        }
                        Err(e) => {
                            output.error(&format!("Failed to get conversation context: {}", e));
                        }
                    }
                }
                Err(e) => {
                    output.error(&format!("Failed to get agent: {}", e));
                }
            }
            Ok(false)
        }
        "/search" => {
            // Parse command format: /search [agent_name] <query>
            let (agent_name, query) = if parts.len() < 2 {
                output.error("Usage: /search <query>");
                return Ok(false);
            } else if parts.len() == 2 {
                // Just a query
                (None, parts[1..].join(" "))
            } else {
                // Check if first part is an agent name
                if context.list_agents().contains(&parts[1].to_string()) {
                    if parts.len() < 3 {
                        output.error("Usage: /search [agent_name] <query>");
                        return Ok(false);
                    }
                    (Some(parts[1]), parts[2..].join(" "))
                } else {
                    (None, parts[1..].join(" "))
                }
            };

            match context.get_agent(agent_name).await {
                Ok(agent) => {
                    output.status(&format!("Searching {} conversations for: {}", agent.name(), query));

                    let handle = agent.handle().await;
                    match handle.search_conversations(Some(&query), None, None, None, 10).await {
                        Ok(messages) => {
                            if messages.is_empty() {
                                output.status("No matching conversations found");
                            } else {
                                output.section(&format!("Found {} matching messages", messages.len()));
                                for msg in messages {
                                    output.status(&message_display::display_message_summary(&msg, 100));
                                    output.status(&format!("  {}", msg.created_at.to_string().dimmed()));
                                    output.status("");
                                }
                            }
                        }
                        Err(e) => {
                            output.error(&format!("Failed to search conversations: {:?}", e));
                        }
                    }
                }
                Err(e) => {
                    output.error(&format!("Failed to get agent: {}", e));
                }
            }
            Ok(false)
        }
        "/query" => {
            if parts.len() < 2 {
                output.error("Usage: /query <sql>");
                return Ok(false);
            }

            let sql = parts[1..].join(" ");
            output.status(&format!("Running query: {}", sql.bright_cyan()));

            // Run the query directly
            match crate::commands::db::query(&sql, output).await {
                Ok(_) => {
                    // Query output is handled by the query function
                }
                Err(e) => {
                    output.error(&format!("Query failed: {}", e));
                }
            }
            Ok(false)
        }
        _ => {
            output.error(&format!("Unknown command: {}", parts[0]));
            output.status("Type /help for available commands");
            Ok(false)
        }
    }
}
