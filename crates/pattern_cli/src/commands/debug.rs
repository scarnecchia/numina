use chrono::DateTime;
use miette::{IntoDiagnostic, Result};
use owo_colors::OwoColorize;
use pattern_core::{
    agent::{AgentRecord, AgentState},
    config::PatternConfig,
    context::AgentHandle,
    db::{DatabaseError, DbEntity, client::DB, ops},
    memory::{Memory, MemoryBlock, MemoryType},
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
                        // Show more details about non-text content
                        use pattern_core::message::{ContentBlock, MessageContent};
                        match &msg.content {
                            MessageContent::Parts(parts) => {
                                output.status(&format!(
                                    "Content: [Multi-part: {} parts]",
                                    parts.len()
                                ));
                                for (j, part) in parts.iter().enumerate().take(3) {
                                    match part {
                                        pattern_core::message::ContentPart::Text(t) => {
                                            let preview = if t.len() > 100 {
                                                format!("{}...", &t[..100])
                                            } else {
                                                t.to_string()
                                            };
                                            println!(
                                                "      Part {}: Text - {}",
                                                j + 1,
                                                preview.dimmed()
                                            );
                                        }
                                        pattern_core::message::ContentPart::Image {
                                            content_type,
                                            ..
                                        } => {
                                            println!(
                                                "      Part {}: [Image: {}]",
                                                j + 1,
                                                content_type
                                            );
                                        }
                                    }
                                }
                                if parts.len() > 3 {
                                    println!("      ... and {} more parts", parts.len() - 3);
                                }
                            }
                            MessageContent::ToolCalls(calls) => {
                                output.status(&format!(
                                    "Content: [Tool calls: {} calls]",
                                    calls.len()
                                ));
                                for (j, call) in calls.iter().enumerate().take(3) {
                                    println!(
                                        "      Call {}: {} (id: {})",
                                        j + 1,
                                        call.fn_name,
                                        call.call_id
                                    );
                                }
                                if calls.len() > 3 {
                                    println!("      ... and {} more calls", calls.len() - 3);
                                }
                            }
                            MessageContent::ToolResponses(responses) => {
                                output.status(&format!(
                                    "Content: [Tool responses: {} responses]",
                                    responses.len()
                                ));
                                for (j, resp) in responses.iter().enumerate().take(3) {
                                    let content_preview = if resp.content.len() > 100 {
                                        format!("{}...", &resp.content[..100])
                                    } else {
                                        resp.content.clone()
                                    };
                                    println!(
                                        "      Response {} (call_id: {}): {}",
                                        j + 1,
                                        resp.call_id,
                                        content_preview.dimmed()
                                    );
                                }
                                if responses.len() > 3 {
                                    println!(
                                        "      ... and {} more responses",
                                        responses.len() - 3
                                    );
                                }
                            }
                            MessageContent::Blocks(blocks) => {
                                output
                                    .status(&format!("Content: [Blocks: {} blocks]", blocks.len()));
                                for (j, block) in blocks.iter().enumerate().take(3) {
                                    match block {
                                        ContentBlock::Text { text } => {
                                            let preview = if text.len() > 100 {
                                                format!("{}...", &text[..100])
                                            } else {
                                                text.clone()
                                            };
                                            println!(
                                                "      Block {}: Text - {}",
                                                j + 1,
                                                preview.dimmed()
                                            );
                                        }
                                        ContentBlock::Thinking { text, .. } => {
                                            let preview = if text.len() > 100 {
                                                format!("{}...", &text[..100])
                                            } else {
                                                text.clone()
                                            };
                                            println!(
                                                "      Block {}: Thinking - {}",
                                                j + 1,
                                                preview.dimmed()
                                            );
                                        }
                                        ContentBlock::RedactedThinking { .. } => {
                                            println!("      Block {}: [Redacted Thinking]", j + 1);
                                        }
                                        ContentBlock::ToolUse { name, id, .. } => {
                                            println!(
                                                "      Block {}: Tool Use - {} (id: {})",
                                                j + 1,
                                                name,
                                                id
                                            );
                                        }
                                        ContentBlock::ToolResult {
                                            tool_use_id,
                                            content,
                                            ..
                                        } => {
                                            let preview = if content.len() > 100 {
                                                format!("{}...", &content[..100])
                                            } else {
                                                content.clone()
                                            };
                                            println!(
                                                "      Block {}: Tool Result (tool_use_id: {}) - {}",
                                                j + 1,
                                                tool_use_id,
                                                preview.dimmed()
                                            );
                                        }
                                    }
                                }
                                if blocks.len() > 3 {
                                    println!("      ... and {} more blocks", blocks.len() - 3);
                                }
                            }
                            _ => {
                                output.status("Content: [Non-text content]");
                            }
                        }
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
            SELECT *, ->agent_memories->mem AS memories FROM $agent_id FETCH memories
        "#;

        let mut mem_response = DB
            .query(mem_query)
            .bind(("agent_id", RecordId::from(&agent_record.id)))
            .await
            .into_diagnostic()?;

        let memories: Vec<Vec<<MemoryBlock as DbEntity>::DbModel>> =
            mem_response.take("memories").into_diagnostic()?;

        let memories: Vec<_> = memories
            .concat()
            .into_iter()
            .map(|m| MemoryBlock::from_db_model(m).expect("db model"))
            .filter(|m| m.memory_type == MemoryType::Core || m.memory_type == MemoryType::Working)
            .collect();

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
pub async fn show_context(agent_name: &str, config: &PatternConfig) -> Result<()> {
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

/// Edit a memory block by exporting to file and reimporting after edits
pub async fn edit_memory(agent_name: &str, label: &str, file_path: Option<&str>) -> Result<()> {
    let output = Output::new();

    output.section(&format!(
        "Editing memory block '{}' for agent '{}'",
        label, agent_name
    ));

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
        // Query for the specific memory block
        let query_sql = r#"
            SELECT * FROM mem
            WHERE label = $label
            LIMIT 1
        "#;

        let mut response = DB
            .query(query_sql)
            .bind(("label", label.to_string()))
            .await
            .into_diagnostic()?;

        let memory_models: Vec<<MemoryBlock as DbEntity>::DbModel> =
            response.take(0).into_diagnostic()?;
        let memories: Vec<MemoryBlock> = memory_models
            .into_iter()
            .filter_map(|m| MemoryBlock::from_db_model(m).ok())
            .collect();

        if let Some(memory) = memories.first() {
            // Extract the memory content
            let content = memory.value.clone();
            let memory_type = format!("{:?}", memory.memory_type);
            let memory_id = memory.id.clone();

            // Determine file path
            let file_path_string = format!("memory_{}.txt", label);
            let file_path = file_path.unwrap_or(&file_path_string);

            // Write to file
            std::fs::write(&file_path, &content).into_diagnostic()?;

            output.info("Memory content written to:", file_path);
            output.info("Memory type:", &memory_type);
            output.info("Current length:", &format!("{} chars", content.len()));
            println!();

            // Get user confirmation to proceed with edit
            output.warning(
                "Edit the file and save it, then press Enter to update the memory block...",
            );
            let mut input = String::new();
            std::io::stdin().read_line(&mut input).into_diagnostic()?;

            // Read the edited content
            let new_content = std::fs::read_to_string(&file_path).into_diagnostic()?;

            // Update the memory block
            let update_sql = r#"
                UPDATE mem SET value = $value WHERE id = $id
            "#;

            let mut response = DB
                .query(update_sql)
                .bind(("value", new_content.clone()))
                .bind(("id", RecordId::from(&memory_id)))
                .await
                .into_diagnostic()?;

            let memory_models: Vec<<MemoryBlock as DbEntity>::DbModel> =
                response.take(0).into_diagnostic()?;
            let _memories: Vec<MemoryBlock> = memory_models
                .into_iter()
                .filter_map(|m| MemoryBlock::from_db_model(m).ok())
                .collect();

            output.success(&format!(
                "Memory block '{}' updated successfully!",
                label.bright_cyan()
            ));
            output.info("New length:", &format!("{} chars", new_content.len()));

            // Optionally delete the temp file
            output.info("Temporary file kept at:", file_path);
        } else {
            output.error(&format!(
                "Memory block '{}' not found for agent '{}'",
                label, agent_name
            ));

            // List available memory blocks
            println!(
                "
Available memory blocks:"
            );
            let query_sql = r#"
                SELECT * FROM mem
                WHERE id IN (
                    SELECT out FROM agent_memories
                    WHERE in = $agent_id
                )
                ORDER BY label
            "#;

            let mut response = DB
                .query(query_sql)
                .bind(("agent_id", RecordId::from(&agent_record.id)))
                .await
                .into_diagnostic()?;

            let memory_models: Vec<<MemoryBlock as DbEntity>::DbModel> =
                response.take(0).into_diagnostic()?;
            let memories: Vec<MemoryBlock> = memory_models
                .into_iter()
                .filter_map(|m| MemoryBlock::from_db_model(m).ok())
                .collect();

            for memory in memories {
                println!(
                    "  - {} ({:?})",
                    memory.label.bright_cyan(),
                    memory.memory_type
                );
            }
        }
    } else {
        output.error(&format!("Agent '{}' not found", agent_name));
    }

    Ok(())
}

pub async fn modify_memory(
    agent: &String,
    label: &String,
    new_label: &Option<String>,
    permission: &Option<String>,
    memory_type: &Option<String>,
) -> miette::Result<()> {
    let output = Output::new();
    // First, find the agent
    let query_sql = "SELECT * FROM agent WHERE name = $name LIMIT 1";
    let mut response = DB
        .query(query_sql)
        .bind(("name", agent.to_string()))
        .await
        .into_diagnostic()?;

    let agents: Vec<<AgentRecord as DbEntity>::DbModel> = response.take(0).into_diagnostic()?;
    let agents: Vec<_> = agents
        .into_iter()
        .map(|e| AgentRecord::from_db_model(e).unwrap())
        .collect();

    if let Some(_agent_record) = agents.first() {
        // Query for the specific memory block
        let query_sql = r#"
            SELECT id FROM mem
            WHERE label = $label
            LIMIT 1
        "#;

        let mut response = DB
            .query(query_sql)
            .bind(("label", label.to_string()))
            .await
            .into_diagnostic()?;

        let id: Vec<RecordId> = response.take("id").map_err(DatabaseError::from)?;

        let mut modify_query = r#"
            UPDATE $memory SET"#
            .to_string();
        if let Some(new_label) = new_label {
            modify_query.push_str(&format!("\n label = '{}'", new_label));
        }
        if let Some(permission) = permission {
            modify_query.push_str(&format!("\n permission = '{}',", permission));
        }

        if let Some(memory_type) = memory_type {
            modify_query.push_str(&format!("\n memory_type = '{}'", memory_type));
        }
        modify_query.push_str(";");

        let _response = DB
            .query(modify_query)
            .bind(("memory", id))
            .await
            .into_diagnostic()?;
    } else {
        output.error(&format!("Agent '{}' not found", agent));
    }
    Ok(())
}

/// Clean up message context by removing unpaired/out-of-order messages
pub async fn context_cleanup(
    agent_name: &str,
    interactive: bool,
    dry_run: bool,
    limit: Option<usize>,
) -> Result<()> {
    use pattern_core::message::{ContentBlock, Message, MessageContent};
    use std::collections::HashSet;
    use std::io::{self, Write};

    let output = Output::new();

    output.section("Context Cleanup");
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

    if let Some(agent_record) = agents.first() {
        output.info(
            "Agent:",
            &format!(
                "{} (ID: {})",
                agent_record.name.bright_cyan(),
                agent_record.id.to_string().dimmed()
            ),
        );

        if dry_run {
            output.warning("DRY RUN - No messages will be deleted");
        }
        println!();

        // Create an agent handle to search messages properly
        let memory = Memory::with_owner(&agent_record.owner_id);
        let mut handle = AgentHandle::default();
        handle.name = agent_record.name.clone();
        handle.agent_id = agent_record.id.clone();
        handle.agent_type = agent_record.agent_type.clone();
        handle.memory = memory;
        handle.state = AgentState::Ready;
        let handle = handle.with_db(DB.clone());

        // Get all messages for this agent (no text query, just get everything)
        let messages = match handle
            .search_conversations(None, None, None, None, limit.unwrap_or(1000))
            .await
        {
            Ok(msgs) => msgs,
            Err(e) => {
                output.error(&format!("Failed to get messages: {}", e));
                return Ok(());
            }
        };

        output.info("Messages", &format!("Found {} messages", messages.len()));

        // Analyze messages for issues
        let mut unpaired_tool_calls: Vec<&Message> = Vec::new();
        let mut unpaired_tool_results: Vec<&Message> = Vec::new();
        let mut out_of_order: Vec<(&Message, &Message)> = Vec::new();
        let mut tool_call_ids: HashSet<String> = HashSet::new();
        let mut tool_result_ids: HashSet<String> = HashSet::new();

        // First pass: collect all tool call and result IDs
        for message in &messages {
            match &message.content {
                MessageContent::ToolCalls(calls) => {
                    for call in calls {
                        tool_call_ids.insert(call.call_id.clone());
                    }
                }
                MessageContent::ToolResponses(responses) => {
                    for response in responses {
                        tool_result_ids.insert(response.call_id.clone());
                    }
                }
                MessageContent::Blocks(blocks) => {
                    for block in blocks {
                        match block {
                            ContentBlock::ToolUse { id, .. } => {
                                tool_call_ids.insert(id.clone());
                            }
                            ContentBlock::ToolResult { tool_use_id, .. } => {
                                tool_result_ids.insert(tool_use_id.clone());
                            }
                            _ => {}
                        }
                    }
                }
                _ => {}
            }
        }

        // Second pass: find unpaired and check ordering
        // Messages are in reverse chronological order (newest first)
        for (i, message) in messages.iter().enumerate() {
            match &message.content {
                MessageContent::ToolCalls(calls) => {
                    for call in calls {
                        if !tool_result_ids.contains(&call.call_id) {
                            unpaired_tool_calls.push(message);
                            break;
                        }
                        // Check ordering: result should come before call in reverse chrono
                        // (i.e., at a lower index since newer messages have lower indices)
                        if i > 0 {
                            let found_result_before = messages[..i].iter().any(|m| {
                                match &m.content {
                                    MessageContent::ToolResponses(responses) => {
                                        responses.iter().any(|r| r.call_id == call.call_id)
                                    }
                                    MessageContent::Blocks(blocks) => {
                                        blocks.iter().any(|b| matches!(b,
                                            ContentBlock::ToolResult { tool_use_id, .. } if tool_use_id == &call.call_id
                                        ))
                                    }
                                    _ => false
                                }
                            });
                            if !found_result_before {
                                // Find the result message that's out of order
                                if let Some(result_msg) = messages[i+1..].iter().find(|m| {
                                    match &m.content {
                                        MessageContent::ToolResponses(responses) => {
                                            responses.iter().any(|r| r.call_id == call.call_id)
                                        }
                                        MessageContent::Blocks(blocks) => {
                                            blocks.iter().any(|b| matches!(b,
                                                ContentBlock::ToolResult { tool_use_id, .. } if tool_use_id == &call.call_id
                                            ))
                                        }
                                        _ => false
                                    }
                                }) {
                                    out_of_order.push((message, result_msg));
                                }
                            }
                        }
                    }
                }
                MessageContent::ToolResponses(responses) => {
                    for response in responses {
                        if !tool_call_ids.contains(&response.call_id) {
                            unpaired_tool_results.push(message);
                            break;
                        }
                    }
                }
                MessageContent::Blocks(blocks) => {
                    for block in blocks {
                        match block {
                            ContentBlock::ToolUse { id, .. } => {
                                if !tool_result_ids.contains(id) {
                                    unpaired_tool_calls.push(message);
                                    break;
                                }
                                // Check ordering for blocks too
                                if i > 0 {
                                    let found_result_before = messages[..i].iter().any(|m| {
                                        match &m.content {
                                            MessageContent::ToolResponses(responses) => {
                                                responses.iter().any(|r| &r.call_id == id)
                                            }
                                            MessageContent::Blocks(blocks) => {
                                                blocks.iter().any(|b| matches!(b,
                                                    ContentBlock::ToolResult { tool_use_id, .. } if tool_use_id == id
                                                ))
                                            }
                                            _ => false
                                        }
                                    });
                                    if !found_result_before {
                                        if let Some(result_msg) = messages[i+1..].iter().find(|m| {
                                            match &m.content {
                                                MessageContent::ToolResponses(responses) => {
                                                    responses.iter().any(|r| &r.call_id == id)
                                                }
                                                MessageContent::Blocks(blocks) => {
                                                    blocks.iter().any(|b| matches!(b,
                                                        ContentBlock::ToolResult { tool_use_id, .. } if tool_use_id == id
                                                    ))
                                                }
                                                _ => false
                                            }
                                        }) {
                                            out_of_order.push((message, result_msg));
                                        }
                                    }
                                }
                            }
                            ContentBlock::ToolResult { tool_use_id, .. } => {
                                if !tool_call_ids.contains(tool_use_id) {
                                    unpaired_tool_results.push(message);
                                    break;
                                }
                            }
                            _ => {}
                        }
                    }
                }
                _ => {}
            }
        }

        // Display issues found
        println!("{}", "Issues Found:".bright_yellow());
        println!("{}", "─".repeat(50).dimmed());

        if unpaired_tool_calls.is_empty()
            && unpaired_tool_results.is_empty()
            && out_of_order.is_empty()
        {
            output.success("No issues found! All tool calls are paired and properly ordered.");
            return Ok(());
        }

        let mut messages_to_delete: Vec<&Message> = Vec::new();

        if !unpaired_tool_calls.is_empty() {
            println!(
                "  {} Unpaired tool calls (no matching results):",
                "⚠".bright_yellow()
            );
            for msg in &unpaired_tool_calls {
                println!("    - Message ID: {}", msg.id.to_string().dimmed());
                println!(
                    "      Role: {}",
                    format!("{:?}", msg.role).bright_yellow().to_string()
                );
                println!(
                    "      Time: {}",
                    msg.created_at.format("%Y-%m-%d %H:%M:%S UTC").to_string()
                );

                // Display content using same logic as search_conversations
                if let Some(text) = msg.text_content() {
                    let preview = if text.len() > 200 {
                        format!("{}...", &text[..200])
                    } else {
                        text.to_string()
                    };
                    println!("      Content:");
                    for line in preview.lines() {
                        println!("        {}", line.dimmed());
                    }
                } else {
                    // Show details about non-text content
                    match &msg.content {
                        MessageContent::ToolCalls(calls) => {
                            println!("      Content: [Tool calls: {} calls]", calls.len());
                            for (j, call) in calls.iter().enumerate().take(3) {
                                println!(
                                    "        Call {}: {} (id: {})",
                                    j + 1,
                                    call.fn_name,
                                    call.call_id
                                );
                            }
                            if calls.len() > 3 {
                                println!("        ... and {} more calls", calls.len() - 3);
                            }
                        }
                        MessageContent::ToolResponses(responses) => {
                            println!(
                                "      Content: [Tool responses: {} responses]",
                                responses.len()
                            );
                            for (j, resp) in responses.iter().enumerate().take(3) {
                                let content_preview = if resp.content.len() > 100 {
                                    format!("{}...", &resp.content[..100])
                                } else {
                                    resp.content.clone()
                                };
                                println!(
                                    "        Response {} (call_id: {}): {}",
                                    j + 1,
                                    resp.call_id,
                                    content_preview.dimmed()
                                );
                            }
                            if responses.len() > 3 {
                                println!("        ... and {} more responses", responses.len() - 3);
                            }
                        }
                        MessageContent::Blocks(blocks) => {
                            println!("      Content: [Blocks: {} blocks]", blocks.len());
                            for (j, block) in blocks.iter().enumerate().take(3) {
                                match block {
                                    ContentBlock::Text { text } => {
                                        let preview = if text.len() > 100 {
                                            format!("{}...", &text[..100])
                                        } else {
                                            text.clone()
                                        };
                                        println!(
                                            "        Block {}: Text - {}",
                                            j + 1,
                                            preview.dimmed()
                                        );
                                    }
                                    ContentBlock::ToolUse { name, id, .. } => {
                                        println!(
                                            "        Block {}: Tool Use - {} (id: {})",
                                            j + 1,
                                            name,
                                            id
                                        );
                                    }
                                    ContentBlock::ToolResult {
                                        tool_use_id,
                                        content,
                                        ..
                                    } => {
                                        let preview = if content.len() > 100 {
                                            format!("{}...", &content[..100])
                                        } else {
                                            content.clone()
                                        };
                                        println!(
                                            "        Block {}: Tool Result (tool_use_id: {}) - {}",
                                            j + 1,
                                            tool_use_id,
                                            preview.dimmed()
                                        );
                                    }
                                    _ => {}
                                }
                            }
                            if blocks.len() > 3 {
                                println!("        ... and {} more blocks", blocks.len() - 3);
                            }
                        }
                        _ => {
                            println!("      Content: [Non-text content]");
                        }
                    }
                }

                messages_to_delete.push(msg);
                println!();
            }
            println!();
        }

        if !unpaired_tool_results.is_empty() {
            println!(
                "  {} Unpaired tool results (no matching calls):",
                "⚠".bright_yellow()
            );
            for msg in &unpaired_tool_results {
                println!("    - Message ID: {}", msg.id.to_string().dimmed());
                if let MessageContent::ToolResponses(responses) = &msg.content {
                    for response in responses {
                        println!(
                            "      Tool response (Call ID: {})",
                            response.call_id.dimmed()
                        );
                    }
                }
                messages_to_delete.push(msg);
            }
            println!();
        }

        if !out_of_order.is_empty() {
            println!(
                "  {} Out-of-order tool call/result pairs:",
                "⚠".bright_yellow()
            );
            // Deduplicate the pairs
            let mut seen = HashSet::new();
            for (call_msg, result_msg) in &out_of_order {
                let key = (call_msg.id.to_string(), result_msg.id.to_string());
                if seen.insert(key) {
                    println!(
                        "    - Call: {} ({})",
                        call_msg.id.to_string().dimmed(),
                        call_msg.created_at.format("%H:%M:%S").to_string().dimmed()
                    );
                    println!(
                        "      Result: {} ({})",
                        result_msg.id.to_string().dimmed(),
                        result_msg
                            .created_at
                            .format("%H:%M:%S")
                            .to_string()
                            .dimmed()
                    );
                    println!(
                        "      {} Result came AFTER call (should be before)",
                        "⚠".bright_red()
                    );
                    // Add both to delete list
                    messages_to_delete.push(call_msg);
                    messages_to_delete.push(result_msg);
                }
            }
            println!();
        }

        // Ask for confirmation or handle non-interactive mode
        if !messages_to_delete.is_empty() {
            println!(
                "{}",
                format!("Found {} messages to clean up", messages_to_delete.len()).bright_white()
            );

            if interactive && !dry_run {
                print!("Proceed with cleanup? [y/N]: ");
                io::stdout().flush().unwrap();

                let mut input = String::new();
                io::stdin().read_line(&mut input).unwrap();

                if !input.trim().eq_ignore_ascii_case("y") {
                    output.status("Cleanup cancelled");
                    return Ok(());
                }
            }

            // Delete messages (or show what would be deleted)
            for msg in messages_to_delete {
                if dry_run {
                    output.status(&format!("Would delete message: {}", msg.id));
                } else {
                    let _: Option<<Message as DbEntity>::DbModel> = DB
                        .delete(RecordId::from(msg.id.clone()))
                        .await
                        .into_diagnostic()?;

                    output.success(&format!("Deleted message: {}", msg.id));
                }
            }

            if !dry_run {
                output.success("Context cleanup complete!");
            } else {
                output.info("Dry run complete", "No messages were deleted");
            }
        }
    } else {
        output.error(&format!("Agent '{}' not found", agent_name));
    }

    Ok(())
}
