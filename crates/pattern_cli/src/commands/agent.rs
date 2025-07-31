use miette::{IntoDiagnostic, Result};
use owo_colors::OwoColorize;
use pattern_core::{
    agent::{AgentRecord, AgentState, AgentType},
    config::{self, AgentConfig, MemoryBlockConfig, PatternConfig},
    db::{client::DB, ops},
    id::AgentId,
};
use std::{collections::HashMap, path::Path};
use surrealdb::RecordId;

use crate::output::{Output, format_agent_state, format_relative_time};

/// List all agents in the database
pub async fn list() -> Result<()> {
    let output = Output::new();
    let agents = ops::list_entities::<AgentRecord, _>(&DB).await?;

    if agents.is_empty() {
        output.status("No agents found");
        output.status(&format!(
            "Create an agent with: {} agent create <name>",
            "pattern-cli".bright_green()
        ));
    } else {
        output.success(&format!("Found {} agent(s):", agents.len()));
        println!();

        for agent in agents {
            output.info("•", &agent.name.bright_cyan().to_string());
            output.kv("ID", &agent.id.to_string().dimmed().to_string());
            output.kv(
                "Type",
                &format!("{:?}", agent.agent_type)
                    .bright_yellow()
                    .to_string(),
            );
            output.kv("State", &format_agent_state(&agent.state));
            output.kv(
                "Stats",
                &format!(
                    "{} messages, {} tool calls",
                    agent.total_messages.to_string().bright_blue(),
                    agent.total_tool_calls.to_string().bright_blue()
                ),
            );
            output.kv("Last active", &format_relative_time(agent.last_active));
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

    // Use config ID for first agent, generate new ID if it already exists
    let agent_id = if let Some(config_id) = &config.agent.id {
        // Check if an agent with this ID already exists
        if ops::get_entity::<AgentRecord, _>(&DB, config_id)
            .await?
            .is_some()
        {
            // Agent exists, generate a new ID
            AgentId::generate()
        } else {
            // First agent, use config ID
            config_id.clone()
        }
    } else {
        // No config ID, generate new
        AgentId::generate()
    };

    // Use system prompt from config or generate default
    let base_instructions = if let Some(system_prompt) = &config.agent.system_prompt {
        system_prompt.clone()
    } else {
        String::new() // blank to use the existing default prompt
    };

    let agent = AgentRecord {
        id: agent_id.clone(),
        name: name.to_string(),
        agent_type: parsed_type.clone(),
        state: AgentState::Ready,
        base_instructions,
        owner_id: user_id.clone(),
        created_at: now,
        updated_at: now,
        last_active: now,
        ..Default::default()
    };

    // Save to database using store_with_relations since AgentRecord has relations
    match agent.store_with_relations(&DB).await {
        Ok(stored_agent) => {
            // Add agent to constellation
            let constellation = ops::get_or_create_constellation(&DB, &user_id).await?;
            let membership = pattern_core::coordination::groups::ConstellationMembership {
                id: pattern_core::id::RelationId::generate(),
                in_id: constellation.id,
                out_id: stored_agent.id.clone(),
                joined_at: now,
                is_primary: false,
            };
            ops::create_relation_typed(&DB, &membership).await?;

            println!();
            output.success("Created agent successfully!\n");
            output.info("Name:", &stored_agent.name.bright_cyan().to_string());
            output.info("ID:", &stored_agent.id.to_string().dimmed().to_string());
            output.info(
                "Type:",
                &format!("{:?}\n", stored_agent.agent_type)
                    .bright_yellow()
                    .to_string(),
            );

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

            output.status(&format!(
                "Start chatting with: {} chat --agent {}",
                "pattern-cli".bright_green(),
                name
            ));
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
        output.info("Name:", &agent.name.bright_cyan().bold().to_string());
        output.kv("ID", &agent.id.to_string().dimmed().to_string());
        output.kv(
            "Type",
            &format!("{:?}", agent.agent_type)
                .bright_yellow()
                .to_string(),
        );
        output.kv("State", &format_agent_state(&agent.state));
        println!();

        // Instructions
        output.section("Instructions");
        output.status(&agent.base_instructions);
        println!();

        // Statistics
        output.section("Statistics");
        output.kv("Messages", &agent.total_messages.to_string());
        output.kv("Tool calls", &agent.total_tool_calls.to_string());
        output.kv("Context rebuilds", &agent.context_rebuilds.to_string());
        output.kv("Compression events", &agent.compression_events.to_string());
        println!();

        // Memory blocks
        output.section("Memory");
        output.kv("Total blocks", &agent.memories.len().to_string());
        if !agent.memories.is_empty() {
            for (memory, _relation) in &agent.memories {
                output.list_item(&format!(
                    "{} ({})",
                    memory.label.bright_yellow(),
                    format!("{} chars", memory.value.len()).dimmed()
                ));
            }
        }
        println!();

        // Timestamps
        output.section("Timestamps");
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

/// Export agent configuration (persona and memory only)
pub async fn export(name: &str, output_path: Option<&Path>) -> Result<()> {
    let output = Output::new();

    // Query for the agent by name
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
        let agent = match AgentRecord::load_with_relations(&DB, &agent_id).await {
            Ok(Some(agent)) => agent,
            Ok(None) => {
                output.error(&format!("No agent found with name '{}'", name));
                return Ok(());
            }
            Err(e) => return Err(miette::miette!("Failed to load agent: {}", e)),
        };

        output.info("Exporting agent:", &agent.name.bright_cyan().to_string());

        // Create the agent config structure
        let agent_config = AgentConfig {
            id: Some(agent.id.clone()),
            name: agent.name.clone(),
            system_prompt: if agent.base_instructions.is_empty() {
                None
            } else {
                Some(agent.base_instructions.clone())
            },
            persona: None, // Will be extracted from memory blocks
            instructions: None,
            bluesky_handle: None,
            memory: HashMap::new(), // Will be populated from memory blocks
            tool_rules: Vec::new(),
            tools: Vec::new(),
            model: None,
        };

        // Get memory blocks using ops function
        let memories = ops::get_agent_memories(&DB, &agent.id).await?;

        // Convert memory blocks to config format
        let mut memory_configs = HashMap::new();
        let mut persona_content = None;

        for (memory_block, permission) in &memories {
            // Check if this is the persona block
            if memory_block.label == "persona" {
                persona_content = Some(memory_block.value.clone());
                continue;
            }

            let memory_config = MemoryBlockConfig {
                content: Some(memory_block.value.clone()),
                content_path: None,
                permission: permission.clone(),
                memory_type: memory_block.memory_type.clone(),
                description: memory_block.description.clone(),
            };

            memory_configs.insert(memory_block.label.to_string(), memory_config);
        }

        // Create the final config with persona and memory
        let mut final_config = agent_config;
        final_config.persona = persona_content;
        final_config.memory = memory_configs;

        // Serialize to TOML
        let toml_str = toml::to_string_pretty(&final_config).into_diagnostic()?;

        // Determine output path
        let output_file = if let Some(path) = output_path {
            path.to_path_buf()
        } else {
            std::path::PathBuf::from(format!("{}.toml", agent.name))
        };

        // Write to file
        tokio::fs::write(&output_file, toml_str)
            .await
            .into_diagnostic()?;

        output.success(&format!(
            "Exported agent configuration to: {}",
            output_file.display().to_string().bright_green()
        ));
        output.status("Note: Only persona and memory blocks were exported");
        output.status("Message history and statistics are not included");
    } else {
        output.error(&format!("No agent found with name '{}'", name));
    }

    Ok(())
}
