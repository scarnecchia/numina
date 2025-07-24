use miette::{IntoDiagnostic, Result};
use owo_colors::OwoColorize;
use pattern_core::{
    agent::{AgentRecord, AgentState, AgentType},
    config::{self, PatternConfig},
    db::{client::DB, ops},
    id::AgentId,
};

use crate::output::{Output, format_agent_state, format_relative_time};

/// List all agents in the database
pub async fn list() -> Result<()> {
    let output = Output::new();
    let agents = ops::list_entities::<AgentRecord, _>(&DB).await?;

    if agents.is_empty() {
        output.status("No agents found");
        println!(
            "Create an agent with: {} agent create <name>",
            "pattern-cli".bright_green()
        );
    } else {
        output.success(&format!("Found {} agent(s):", agents.len()));
        println!();

        for agent in agents {
            output.info("•", &agent.name.bright_cyan().to_string());
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

    Ok(())
}

/// Create a new agent
pub async fn create(name: &str, agent_type: Option<&str>, config: &PatternConfig) -> Result<()> {
    let output = Output::new();

    output.info("Creating agent:", &name.bright_cyan().to_string());

    // Parse agent type
    let parsed_type = if let Some(type_str) = agent_type {
        match type_str.parse::<AgentType>() {
            Ok(t) => t,
            Err(_) => {
                output.warning(&format!(
                    "Unknown agent type '{}', using 'generic'",
                    type_str
                ));
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
        name: name.to_string(),
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
            output.success("Created agent successfully!");
            println!();
            output.info("Name:", &stored_agent.name.bright_cyan().to_string());
            output.info("ID:", &stored_agent.id.to_string().dimmed().to_string());
            output.info(
                "Type:",
                &format!("{:?}", stored_agent.agent_type)
                    .bright_yellow()
                    .to_string(),
            );
            println!();

            // Save the agent ID back to config if it was generated
            if config.agent.id.is_none() {
                output.status("Saving agent ID to config for future sessions...");
                let mut updated_config = config.clone();
                updated_config.agent.id = Some(stored_agent.id.clone());
                if let Err(e) =
                    config::save_config(&updated_config, &config::config_paths()[0]).await
                {
                    output.warning(&format!("Failed to save agent ID to config: {}", e));
                }
            }

            println!(
                "Start chatting with: {} chat --agent {}",
                "pattern-cli".bright_green(),
                name
            );
        }
        Err(e) => {
            output.error(&format!("Failed to create agent: {}", e));
        }
    }

    Ok(())
}

/// Show detailed status for an agent
pub async fn status(name: &str) -> Result<()> {
    let output = Output::new();

    // Query for the agent by name
    let query = "SELECT * FROM agent WHERE name = $name LIMIT 1";
    let mut response = DB
        .query(query)
        .bind(("name", name.to_string()))
        .await
        .into_diagnostic()?;

    let agents: Vec<AgentRecord> = response.take(0).into_diagnostic()?;

    if let Some(agent) = agents.first() {
        output.section("Agent Status");
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
        output.error(&format!("No agent found with name '{}'", name));
        println!();
        println!("Available agents:");

        // List all agents
        let all_agents = ops::list_entities::<AgentRecord, _>(&DB).await?;
        if all_agents.is_empty() {
            output.status("No agents created yet");
        } else {
            for agent in all_agents {
                output.list_item(&agent.name.bright_cyan().to_string());
            }
        }
    }

    Ok(())
}
