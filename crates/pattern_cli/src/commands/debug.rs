use chrono::DateTime;
use miette::{IntoDiagnostic, Result};
use owo_colors::OwoColorize;
use pattern_core::{
    agent::{AgentRecord, AgentState},
    context::AgentHandle,
    db::{DbEntity, client::DB, ops},
    memory::{Memory, MemoryBlock},
    message::ChatRole,
};
use surrealdb::RecordId;

use crate::output::Output;

/// Search conversation history for an agent
pub async fn search_conversations(
    agent_name: &str,
    query: Option<&str>,
    role: Option<&str>,
    start_time: Option<&str>,
    end_time: Option<&str>,
    limit: usize,
) -> Result<()> {
    let output = Output::new();

    output.section("Searching conversation history");
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
        output.info(
            "Agent:",
            &format!(
                "{} (ID: {})",
                agent_record.name.bright_cyan(),
                agent_record.id.to_string().dimmed()
            ),
        );
        output.kv(
            "Owner",
            &agent_record.owner_id.to_string().dimmed().to_string(),
        );

        // Display search parameters
        if let Some(q) = query {
            output.kv("Query", &format!("\"{}\"", q.bright_yellow()));
        }
        if let Some(r) = role {
            output.kv("Role filter", &r.bright_yellow().to_string());
        }
        if let Some(st) = start_time {
            output.kv("Start time", &st.bright_yellow().to_string());
        }
        if let Some(et) = end_time {
            output.kv("End time", &et.bright_yellow().to_string());
        }
        output.kv("Limit", &limit.to_string().bright_white().to_string());
        println!();

        // Parse role if provided
        let role_filter = if let Some(role_str) = role {
            match role_str.to_lowercase().as_str() {
                "system" => Some(ChatRole::System),
                "user" => Some(ChatRole::User),
                "assistant" => Some(ChatRole::Assistant),
                "tool" => Some(ChatRole::Tool),
                _ => {
                    output.warning(&format!(
                        "Invalid role: {}. Using no role filter.",
                        role_str
                    ));
                    output.status("Valid roles: system, user, assistant, tool");
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
                    output.warning(&format!("Invalid start time format: {}", e));
                    output.status("Expected ISO 8601 format: 2024-01-20T00:00:00Z");
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
                    output.warning(&format!("Invalid end time format: {}", e));
                    output.status("Expected ISO 8601 format: 2024-01-20T23:59:59Z");
                    println!();
                    None
                }
            }
        } else {
            None
        };

        // Create a minimal agent handle for searching
        let memory = Memory::with_owner(&agent_record.owner_id);
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
                output.success(&format!("Found {} messages:", messages.len()));
                println!();

                for (i, msg) in messages.iter().enumerate() {
                    println!(
                        "  {} Message {} {}",
                        match msg.role {
                            ChatRole::System => "[system]   ".bright_blue().to_string(),
                            ChatRole::User => "[user]     ".bright_green().to_string(),
                            ChatRole::Assistant => "[assistant]".bright_cyan().to_string(),
                            ChatRole::Tool => "[tool]     ".bright_yellow().to_string(),
                        },
                        (i + 1).to_string().bright_white(),
                        format!("({})", msg.id.0).dimmed()
                    );
                    output.kv(
                        "Role",
                        &format!("{:?}", msg.role).bright_yellow().to_string(),
                    );
                    output.kv(
                        "Time",
                        &msg.created_at.format("%Y-%m-%d %H:%M:%S UTC").to_string(),
                    );

                    // Extract and display content
                    if let Some(text) = msg.text_content() {
                        let preview = if text.len() > 200 {
                            format!("{}...", &text[..200])
                        } else {
                            text.to_string()
                        };
                        output.status("Content:");
                        for line in preview.lines() {
                            println!("      {}", line.dimmed());
                        }
                    } else {
                        output.status("Content: [Non-text content]");
                    }

                    println!();
                }

                if messages.is_empty() {
                    output.status("No messages found matching the search criteria");
                    println!();
                    output.status("Try:");
                    output.list_item("Using broader search terms");
                    output.list_item("Removing filters to see all messages");
                    output.list_item("Checking if the agent has any messages in the database");
                }
            }
            Err(e) => {
                output.error(&format!("Search failed: {}", e));
                println!();
                output.status("This might mean:");
                output.list_item("The database connection is not available");
                output.list_item("There was an error in the query");
                output.list_item("The message table or indexes are not set up");
            }
        }
    } else {
        output.error(&format!("Agent '{}' not found", agent_name));
        println!();
        output.status("Available agents:");
        let all_agents = ops::list_entities::<AgentRecord, _>(&DB).await?;
        for agent in all_agents {
            output.list_item(&agent.name.bright_cyan().to_string());
        }
    }

    Ok(())
}

/// Search archival memory as if we were the agent
pub async fn search_archival_memory(agent_name: &str, query: &str, limit: usize) -> Result<()> {
    let output = Output::new();

    output.section("Searching archival memory");
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
        output.info(
            "Agent:",
            &format!(
                "{} (ID: {})",
                agent_record.name.bright_cyan(),
                agent_record.id.to_string().dimmed()
            ),
        );
        output.kv(
            "Owner",
            &agent_record.owner_id.to_string().dimmed().to_string(),
        );
        output.kv("Query", &format!("\"{}\"", query.bright_yellow()));
        output.kv("Limit", &limit.to_string().bright_white().to_string());

        // Debug: Let's check what memories exist for this owner
        let debug_query = format!(
            "SELECT id, label, memory_type FROM mem WHERE owner_id = user:⟨{}⟩",
            agent_record
                .owner_id
                .to_string()
                .trim_start_matches("user_")
        );
        output.status("Debug - checking memories for owner...");
        let debug_response = DB.query(&debug_query).await.into_diagnostic()?;
        output.status(&format!("Debug response: {:?}", debug_response));
        println!();

        // Create a minimal agent handle for searching
        // IMPORTANT: Use the actual owner_id from the database so the search will match
        let memory = Memory::with_owner(&agent_record.owner_id);
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
                output.success(&format!("Found {} results:", results.len()));
                println!();

                for (i, block) in results.iter().enumerate() {
                    println!(
                        "  {} Result {} {}",
                        "[DOC]".bright_blue(),
                        (i + 1).to_string().bright_white(),
                        format!("({})", block.id).dimmed()
                    );
                    output.kv("Label", &block.label.bright_yellow().to_string());
                    output.kv("Type", &format!("{:?}", block.memory_type));
                    output.kv(
                        "Created",
                        &block.created_at.format("%Y-%m-%d %H:%M:%S UTC").to_string(),
                    );
                    output.status("Content preview:");

                    // Show first 200 chars of content
                    let preview = if block.value.len() > 200 {
                        format!("{}...", &block.value[..200])
                    } else {
                        block.value.clone()
                    };

                    for line in preview.lines() {
                        println!("      {}", line.dimmed());
                    }
                    println!();
                }

                if results.is_empty() {
                    output.status(&format!("No archival memories found matching '{}'", query));
                    println!();
                    output.status("Try:");
                    output.list_item("Using broader search terms");
                    output.list_item(&format!(
                        "Checking if the agent has any archival memories with: pattern-cli debug list-archival --agent {}",
                        agent_name
                    ));
                    output.list_item("Verifying the full-text search index exists in the database");
                }
            }
            Err(e) => {
                output.error(&format!("Search failed: {}", e));
                println!();
                output.status("This might mean:");
                output.list_item("The database connection is not available");
                output.list_item("The full-text search index is not set up");
                output.list_item("There was an error in the query");
            }
        }
    } else {
        output.error(&format!("Agent '{}' not found", agent_name));
        println!();
        output.status("Available agents:");
        let all_agents = ops::list_entities::<AgentRecord, _>(&DB).await?;
        for agent in all_agents {
            output.list_item(&agent.name.bright_cyan().to_string());
        }
    }

    Ok(())
}

/// List all archival memories for an agent
pub async fn list_archival_memory(agent_name: &str) -> Result<()> {
    let output = Output::new();

    output.section("Listing archival memories");
    println!();

    // First, find the agent
    let query_sql = "SELECT * FROM agent WHERE name = $name LIMIT 1";
    let mut response = DB
        .query(query_sql)
        .bind(("name", agent_name.to_string()))
        .await
        .into_diagnostic()?;

    tracing::trace!("response: {:?}", response);

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

        tracing::trace!("Debug - mem_response: {:?}", mem_response);

        let memories: Vec<Vec<<MemoryBlock as DbEntity>::DbModel>> =
            mem_response.take("memories").into_diagnostic()?;

        let memories: Vec<_> = memories
            .concat()
            .into_iter()
            .map(|m| MemoryBlock::from_db_model(m).expect("db model"))
            .collect();

        output.success(&format!("Found {} archival memories", memories.len()));
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
            output.status("No archival memories found for this agent");
            println!();
            println!("Archival memories can be created:");
            println!("  • By the agent using the recall tool");
            println!("  • Through the API");
            println!("  • By importing from external sources");
        }
    } else {
        output.error(&format!("Agent '{}' not found", agent_name));
    }

    Ok(())
}

/// List all core memory blocks for an agent
pub async fn list_core_memory(agent_name: &str) -> Result<()> {
    let output = Output::new();

    output.section("Listing core memory blocks");
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

        output.success(&format!("Found {} core memory blocks", memories.len()));
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
            output.status("No core memory blocks found for this agent");
            println!();
            println!("Core memory blocks are usually created:");
            println!("  • Automatically when an agent is initialized");
            println!("  • By the agent using the context tool");
            println!("  • Through direct API calls");
        }
    } else {
        output.error(&format!("Agent '{}' not found", agent_name));
    }

    Ok(())
}

/// List all memory blocks for an agent (both core and archival)
pub async fn list_all_memory(agent_name: &str) -> Result<()> {
    let output = Output::new();

    output.section("Listing all memory blocks");
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
        output.success(&format!("Found {} total memory blocks", total));
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
            output.status("No memory blocks found for this agent");
            println!();
            println!("Memory blocks are created:");
            println!("  • Automatically when an agent is used in chat");
            println!("  • By the agent using memory management tools");
            println!("  • Through direct API calls");
        }
    } else {
        output.error(&format!("Agent '{}' not found", agent_name));
    }

    Ok(())
}

/// Show the current context that would be passed to the LLM
pub async fn show_context(agent_name: &str) -> Result<()> {
    let output = Output::new();

    output.section("Agent Context Inspection");
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
        output.info(
            "Agent:",
            &format!(
                "{} (ID: {})",
                agent_record.name.bright_cyan(),
                agent_record.id.to_string().dimmed()
            ),
        );
        output.info("State:", &format!("{:?}", agent_record.state));
        println!();

        // Load the agent using the standard function
        let config = crate::config::PatternConfig::default();
        let (heartbeat_sender, _) = pattern_core::context::heartbeat::heartbeat_channel();

        match crate::agent_ops::load_or_create_agent(
            agent_name,
            None, // Use default model
            true, // Enable tools
            &config,
            heartbeat_sender,
        )
        .await
        {
            Ok(agent) => {
                // Get system prompt sections from agent
                let system_prompt_parts = agent.system_prompt().await;
                let system_prompt = system_prompt_parts.join("\n\n");

                output.section("Current System Prompt (as passed to LLM)");
                println!();
                println!("{}", system_prompt);
                println!();

                // Get available tools
                let available_tools = agent.available_tools().await;
                if !available_tools.is_empty() {
                    output.section(&format!("Available Tools ({})", available_tools.len()));
                    println!();
                    for tool in &available_tools {
                        println!("• {} - {}", tool.name().cyan(), tool.description());
                    }
                    println!();
                }

                output.success("Context inspection complete");
            }
            Err(e) => {
                output.error(&format!("Failed to load agent: {}", e));
            }
        }
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
