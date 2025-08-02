use miette::{IntoDiagnostic, Result};
use owo_colors::OwoColorize;
use pattern_core::{
    agent::{AgentRecord, AgentState, AgentType, tool_rules::ToolRule},
    config::{self, AgentConfig, MemoryBlockConfig, PatternConfig, ToolRuleConfig},
    db::{DbEntity, client::DB, ops},
    id::AgentId,
};
use std::{
    collections::HashMap,
    io::{self, Write},
    path::Path,
    time::Duration,
};
use surrealdb::RecordId;

use crate::{
    agent_ops::load_agent_memories_and_messages,
    output::{Output, format_agent_state, format_relative_time},
};

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

    use pattern_core::db::entity::DbEntity;
    let agent_db_models: Vec<<AgentRecord as DbEntity>::DbModel> =
        response.take(0).into_diagnostic()?;

    // Convert DB models to domain types
    let mut agents: Vec<AgentRecord> = agent_db_models
        .into_iter()
        .map(|db_model| {
            AgentRecord::from_db_model(db_model)
                .map_err(|e| miette::miette!("Failed to convert agent: {}", e))
        })
        .collect::<Result<Vec<_>>>()?;

    for agent in agents.iter_mut() {
        load_agent_memories_and_messages(agent).await?;
    }

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
            context: None,
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
                id: None,
                shared: false,
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

/// Add a workflow rule to an agent
pub async fn add_rule(
    agent_name: &str,
    rule_type: &str,
    tool_name: &str,
    params: Option<&str>,
    conditions: Option<&str>,
    priority: u8,
) -> Result<()> {
    let output = Output::new();

    // If no rule type provided, make it interactive
    let (rule_type, tool_name, params, conditions, priority) = if rule_type.is_empty() {
        interactive_rule_builder(&output).await?
    } else {
        (
            rule_type.to_string(),
            tool_name.to_string(),
            params.map(|s| s.to_string()),
            conditions.map(|s| s.to_string()),
            priority,
        )
    };

    output.section("Adding Workflow Rule");
    println!();

    // Find the agent
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

    if let Some(mut agent_record) = agents.into_iter().next() {
        // Parse rule type and create ToolRule
        let tool_rule = match rule_type.as_str() {
            "start-constraint" => ToolRule::start_constraint(tool_name.clone()),
            "exit-loop" => ToolRule::exit_loop(tool_name.clone()),
            "continue-loop" => ToolRule::continue_loop(tool_name.clone()),
            "max-calls" => {
                let count = params
                    .as_ref()
                    .and_then(|p| p.parse::<u32>().ok())
                    .ok_or_else(|| miette::miette!("max-calls requires a numeric parameter"))?;
                ToolRule::max_calls(tool_name.clone(), count)
            }
            "cooldown" => {
                let seconds = params
                    .as_ref()
                    .and_then(|p| p.parse::<u64>().ok())
                    .ok_or_else(|| miette::miette!("cooldown requires duration in seconds"))?;
                ToolRule::cooldown(tool_name.clone(), Duration::from_secs(seconds))
            }
            "requires-preceding" => {
                let condition_list = conditions
                    .as_ref()
                    .ok_or_else(|| miette::miette!("requires-preceding needs conditions"))?
                    .split(',')
                    .map(|s| s.trim().to_string())
                    .collect();
                ToolRule::requires_preceding_tools(tool_name.clone(), condition_list)
            }
            _ => {
                return Err(miette::miette!(
                    "Unknown rule type: {}. Valid types: start-constraint, exit-loop, continue-loop, max-calls, cooldown, requires-preceding",
                    rule_type
                ));
            }
        };

        // Set priority if specified
        let tool_rule = tool_rule.with_priority(priority);

        // Convert to config format and add to agent record
        let config_rule = ToolRuleConfig::from_tool_rule(&tool_rule);
        agent_record.tool_rules.push(config_rule);

        // Save updated agent record
        ops::update_entity(&DB, &agent_record)
            .await
            .into_diagnostic()?;

        output.success(&format!(
            "Added {} rule for tool '{}' to agent '{}'",
            rule_type, tool_name, agent_name
        ));

        // Show the rule that was added
        let description = tool_rule.to_usage_description();
        output.info("Rule", &description);
    } else {
        output.error(&format!("Agent '{}' not found", agent_name));

        // List available agents
        let query_sql = "SELECT name FROM agent ORDER BY name";
        let mut response = DB.query(query_sql).await.into_diagnostic()?;
        let agent_names: Vec<String> = response
            .take::<Vec<surrealdb::sql::Value>>(0)
            .into_diagnostic()?
            .into_iter()
            .filter_map(|v| Some(v.as_string()))
            .collect();

        if !agent_names.is_empty() {
            output.info("Available agents:", &agent_names.join(", "));
        }
    }

    Ok(())
}

/// Interactive rule builder for step-by-step rule creation
async fn interactive_rule_builder(
    output: &Output,
) -> Result<(String, String, Option<String>, Option<String>, u8)> {
    output.section("Interactive Workflow Rule Builder");
    println!();

    // Get tool name
    print!("Tool name: ");
    io::stdout().flush().unwrap();
    let mut tool_name = String::new();
    io::stdin().read_line(&mut tool_name).into_diagnostic()?;
    let tool_name = tool_name.trim().to_string();

    // Show available rule types
    output.info("Available rule types:", "");
    println!("  1. start-constraint  - Call this tool first before any other tools");
    println!("  2. exit-loop        - End the conversation after calling this tool");
    println!("  3. continue-loop    - Continue the conversation after calling this tool");
    println!("  4. max-calls        - Limit how many times this tool can be called");
    println!("  5. cooldown         - Minimum time between calls to this tool");
    println!("  6. requires-preceding - This tool can only be called after specific other tools");
    println!();

    // Get rule type
    print!("Choose rule type (1-6): ");
    io::stdout().flush().unwrap();
    let mut choice = String::new();
    io::stdin().read_line(&mut choice).into_diagnostic()?;
    let choice = choice.trim();

    let (rule_type, params, conditions) = match choice {
        "1" => ("start-constraint".to_string(), None, None),
        "2" => ("exit-loop".to_string(), None, None),
        "3" => ("continue-loop".to_string(), None, None),
        "4" => {
            print!("Maximum number of calls: ");
            io::stdout().flush().unwrap();
            let mut max_calls = String::new();
            io::stdin().read_line(&mut max_calls).into_diagnostic()?;
            (
                "max-calls".to_string(),
                Some(max_calls.trim().to_string()),
                None,
            )
        }
        "5" => {
            print!("Cooldown duration in seconds: ");
            io::stdout().flush().unwrap();
            let mut cooldown = String::new();
            io::stdin().read_line(&mut cooldown).into_diagnostic()?;
            (
                "cooldown".to_string(),
                Some(cooldown.trim().to_string()),
                None,
            )
        }
        "6" => {
            print!("Required preceding tools (comma-separated): ");
            io::stdout().flush().unwrap();
            let mut preceding = String::new();
            io::stdin().read_line(&mut preceding).into_diagnostic()?;
            (
                "requires-preceding".to_string(),
                None,
                Some(preceding.trim().to_string()),
            )
        }
        _ => return Err(miette::miette!("Invalid choice. Please choose 1-6.")),
    };

    // Get priority
    print!("Priority (0-255, default 100): ");
    io::stdout().flush().unwrap();
    let mut priority_input = String::new();
    io::stdin()
        .read_line(&mut priority_input)
        .into_diagnostic()?;
    let priority = if priority_input.trim().is_empty() {
        100
    } else {
        priority_input
            .trim()
            .parse::<u8>()
            .map_err(|_| miette::miette!("Priority must be a number between 0-255"))?
    };

    Ok((rule_type, tool_name, params, conditions, priority))
}

/// List workflow rules for an agent
pub async fn list_rules(agent_name: &str) -> Result<()> {
    let output = Output::new();

    output.section("Agent Workflow Rules");
    println!();

    // Find the agent
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

    if let Some(agent_record) = agents.into_iter().next() {
        output.info(
            "Agent",
            &format!("{} ({})", agent_record.name, agent_record.id),
        );
        println!();

        if agent_record.tool_rules.is_empty() {
            output.info("Workflow Rules", "No custom workflow rules configured");
        } else {
            output.section(&format!(
                "Workflow Rules ({})",
                agent_record.tool_rules.len()
            ));
            println!();

            for (i, config_rule) in agent_record.tool_rules.iter().enumerate() {
                match config_rule.to_tool_rule() {
                    Ok(rule) => {
                        let description = rule.to_usage_description();
                        println!("{}. {}", (i + 1).to_string().dimmed(), description);

                        // Show additional details
                        println!("   Tool: {}", rule.tool_name.cyan());
                        println!("   Type: {:?}", rule.rule_type);
                        println!("   Priority: {}", rule.priority);
                        if !rule.conditions.is_empty() {
                            println!("   Conditions: {}", rule.conditions.join(", ").dimmed());
                        }
                        println!();
                    }
                    Err(e) => {
                        println!(
                            "{}. {}: {}",
                            (i + 1).to_string().dimmed(),
                            "Invalid rule".red(),
                            e.to_string().dimmed()
                        );
                    }
                }
            }
        }
    } else {
        output.error(&format!("Agent '{}' not found", agent_name));
    }

    Ok(())
}

/// Remove workflow rules from an agent
pub async fn remove_rule(agent_name: &str, tool_name: &str, rule_type: Option<&str>) -> Result<()> {
    let output = Output::new();

    output.section("Removing Workflow Rule");
    println!();

    // Find the agent
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

    if let Some(mut agent_record) = agents.into_iter().next() {
        let original_count = agent_record.tool_rules.len();

        // Remove matching rules
        agent_record.tool_rules.retain(|config_rule| {
            if config_rule.tool_name != tool_name {
                return true; // Keep rules for other tools
            }

            if let Some(target_type) = rule_type {
                // Only remove specific rule type
                let rule_type_str = format!("{:?}", config_rule.rule_type).to_lowercase();
                let target_type_normalized = target_type.replace("-", "").to_lowercase();

                // Keep if rule types don't match
                !rule_type_str.contains(&target_type_normalized)
            } else {
                // Remove all rules for this tool
                false
            }
        });

        let removed_count = original_count - agent_record.tool_rules.len();

        if removed_count > 0 {
            // Save updated agent record
            ops::update_entity(&DB, &agent_record)
                .await
                .into_diagnostic()?;

            if let Some(rt) = rule_type {
                output.success(&format!(
                    "Removed {} {} rule(s) for tool '{}' from agent '{}'",
                    removed_count, rt, tool_name, agent_name
                ));
            } else {
                output.success(&format!(
                    "Removed {} rule(s) for tool '{}' from agent '{}'",
                    removed_count, tool_name, agent_name
                ));
            }
        } else {
            if let Some(rt) = rule_type {
                output.info(
                    "No changes",
                    &format!(
                        "No {} rules found for tool '{}' on agent '{}'",
                        rt, tool_name, agent_name
                    ),
                );
            } else {
                output.info(
                    "No changes",
                    &format!(
                        "No rules found for tool '{}' on agent '{}'",
                        tool_name, agent_name
                    ),
                );
            }
        }
    } else {
        output.error(&format!("Agent '{}' not found", agent_name));
    }

    Ok(())
}
