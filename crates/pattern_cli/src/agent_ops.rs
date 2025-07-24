use miette::{IntoDiagnostic, Result};
use owo_colors::OwoColorize;
use pattern_core::{
    Agent, ModelProvider,
    agent::{AgentRecord, AgentType, DatabaseAgent},
    config::PatternConfig,
    coordination::groups::{AgentGroup, GroupManager},
    db::{client::DB, ops},
    embeddings::cloud::OpenAIEmbedder,
    id::AgentId,
    memory::{Memory, MemoryBlock},
    message::{Message, MessageContent},
    model::{GenAiClient, ResponseOptions},
    tool::ToolRegistry,
};
use std::sync::Arc;
use surrealdb::RecordId;
use tokio::sync::RwLock;
use tracing::info;

use crate::output::Output;

/// Load an existing agent from the database or create a new one
pub async fn load_or_create_agent(
    name: &str,
    model_name: Option<String>,
    enable_tools: bool,
    config: &PatternConfig,
) -> Result<Arc<dyn Agent>> {
    let output = Output::new();

    // First, try to find an existing agent with this name
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
        let mut existing_agent = match AgentRecord::load_with_relations(&DB, agent_id).await {
            Ok(Some(agent)) => {
                tracing::trace!("Full AgentRecord: {:#?}", agent);
                agent
            }
            Ok(None) => return Err(miette::miette!("Agent not found after query")),
            Err(e) => return Err(miette::miette!("Failed to load agent: {}", e)),
        };

        // Manually load message history since the macro doesn't handle edge entities properly yet
        existing_agent.messages = existing_agent
            .load_message_history(&DB, false)
            .await
            .map_err(|e| miette::miette!("Failed to load message history: {}", e))?;

        tracing::debug!(
            "After loading message history: {} messages",
            existing_agent.messages.len()
        );

        // Also manually load memory blocks using the ops function
        let memory_tuples = ops::get_agent_memories(&DB, agent_id)
            .await
            .map_err(|e| miette::miette!("Failed to load memory blocks: {}", e))?;

        output.status(&format!(
            "Found {} memory blocks in database",
            memory_tuples.len().to_string().bright_blue()
        ));

        for (block, _) in &memory_tuples {
            output.list_item(&format!(
                "{} ({} chars)",
                block.label.bright_yellow(),
                block.value.len()
            ));
        }
        println!();

        // Convert to the format expected by AgentRecord
        existing_agent.memories = memory_tuples
            .into_iter()
            .map(|(memory_block, access_level)| {
                let relation = pattern_core::agent::AgentMemoryRelation {
                    id: None,
                    in_id: agent_id,
                    out_id: memory_block.id,
                    access_level,
                    created_at: chrono::Utc::now(),
                };
                (memory_block, relation)
            })
            .collect();

        tracing::debug!(
            "After loading memory blocks: {} memories",
            existing_agent.memories.len()
        );

        output.kv("ID", &existing_agent.id.to_string().dimmed().to_string());
        output.kv(
            "Type",
            &format!("{:?}", existing_agent.agent_type)
                .bright_yellow()
                .to_string(),
        );
        output.kv(
            "History",
            &format!("{} messages", existing_agent.total_messages),
        );
        println!();

        // Create runtime agent from the stored record
        create_agent_from_record(existing_agent.clone(), model_name, enable_tools, config).await
    } else {
        output.info("+", &format!("Creating new agent '{}'", name.bright_cyan()));
        println!();

        // Create a new agent
        create_agent(name, model_name, enable_tools, config).await
    }
}

/// Create a runtime agent from a stored AgentRecord
pub async fn create_agent_from_record(
    record: AgentRecord,
    model_name: Option<String>,
    enable_tools: bool,
    _config: &PatternConfig,
) -> Result<Arc<dyn Agent>> {
    // Create model provider
    let model_provider = Arc::new(RwLock::new(GenAiClient::new().await?));

    // Get available models and select the one to use
    let model_info = {
        let provider = model_provider.read().await;
        let models = provider.list_models().await?;

        // If a specific model was requested, try to find it
        let selected_model = if let Some(requested_model) = &model_name {
            models
                .iter()
                .find(|m| m.id.contains(requested_model) || m.name.contains(requested_model))
                .cloned()
        } else if let Some(stored_model) = &record.model_id {
            // Try to use the agent's stored model preference
            models.iter().find(|m| &m.id == stored_model).cloned()
        } else {
            // Default to Gemini models with free tier
            models
                .iter()
                .find(|m| m.provider == "Gemini" && m.id.contains("gemini-2.5-flash"))
                .cloned()
                .or_else(|| {
                    models
                        .iter()
                        .find(|m| m.provider == "Gemini" && m.id.contains("gemini-2.5-pro"))
                        .cloned()
                })
                .or_else(|| models.into_iter().next())
        };

        selected_model.ok_or_else(|| {
            miette::miette!("No models available. Please set API keys in your .env file")
        })?
    };

    info!("Selected model: {} ({})", model_info.name, model_info.id);

    // Create embedding provider if API key is available
    let embedding_provider = if let Ok(api_key) = std::env::var("OPENAI_API_KEY") {
        Some(Arc::new(OpenAIEmbedder::new(
            "text-embedding-3-small".to_string(),
            api_key,
            None,
        )))
    } else {
        None
    };

    // Create tool registry
    let tools = ToolRegistry::new();

    // Create response options with the selected model
    let response_options = ResponseOptions {
        model_info: model_info.clone(),
        temperature: Some(0.7),
        max_tokens: Some(1000000), // for gemini models
        capture_content: Some(true),
        capture_tool_calls: Some(enable_tools),
        top_p: None,
        stop_sequences: vec![],
        capture_usage: Some(true),
        capture_reasoning_content: None,
        capture_raw_body: None,
        response_format: None,
        normalize_reasoning_content: None,
        reasoning_effort: None,
    };

    // Create agent from the record
    let agent = DatabaseAgent::from_record(
        record,
        DB.clone(),
        model_provider,
        tools,
        embedding_provider,
    )
    .await?;

    // Set the chat options with our selected model
    {
        let mut options = agent.chat_options.write().await;
        *options = Some(response_options);
    }
    agent.start_stats_sync().await?;
    agent.start_memory_sync().await?;

    Ok(Arc::new(agent))
}

/// Create an agent with the specified configuration
pub async fn create_agent(
    name: &str,
    model_name: Option<String>,
    enable_tools: bool,
    config: &PatternConfig,
) -> Result<Arc<dyn Agent>> {
    let output = Output::new();

    // Create model provider
    let model_provider = Arc::new(RwLock::new(GenAiClient::new().await?));

    // Get available models and select the one to use
    let model_info = {
        let provider = model_provider.read().await;
        let models = provider.list_models().await?;

        // Debug: print available models
        for model in &models {
            info!(
                "Available model: {} (id: {}, provider: {})",
                model.name, model.id, model.provider
            );
        }

        // If a specific model was requested, try to find it
        let selected_model = if let Some(requested_model) = &model_name {
            models
                .iter()
                .find(|m| m.id.contains(requested_model) || m.name.contains(requested_model))
                .cloned()
        } else {
            // Default to Gemini models with free tier, prioritizing Flash for better rate limits
            models
                .iter()
                .find(|m| m.provider == "Gemini" && m.id.contains("gemini-2.5-pro"))
                .cloned()
                .or_else(|| {
                    models
                        .iter()
                        .find(|m| m.provider == "Gemini" && m.id.contains("gemini-2.5-flash"))
                        .cloned()
                })
                .or_else(|| models.into_iter().next())
        };

        let model_info = selected_model.ok_or_else(|| {
            miette::miette!(
                "No models available. Please set one of the following environment variables:\n\
                - OPENAI_API_KEY\n\
                - ANTHROPIC_API_KEY\n\
                - GEMINI_API_KEY\n\
                - GROQ_API_KEY\n\
                - COHERE_API_KEY\n\n\
                You can add these to a .env file in your project root."
            )
        })?;

        info!("Selected model: {} ({})", model_info.name, model_info.id);
        model_info
    };

    // Create embedding provider if API key is available
    let embedding_provider = if let Ok(api_key) = std::env::var("OPENAI_API_KEY") {
        Some(Arc::new(OpenAIEmbedder::new(
            "text-embedding-3-small".to_string(),
            api_key,
            None,
        )))
    } else {
        None
    };

    // Create memory with the configured user as owner
    let memory = Memory::with_owner(config.user.id.clone());

    // Create tool registry
    let tools = ToolRegistry::new();

    // Use IDs from config or generate new ones
    let agent_id = config.agent.id.clone().unwrap_or_else(AgentId::generate);
    let user_id = config.user.id.clone();

    // Create response options with the selected model
    let response_options = ResponseOptions {
        model_info: model_info.clone(),
        temperature: Some(0.7),
        max_tokens: Some(1000000), // for gemini models
        capture_content: Some(true),
        capture_tool_calls: Some(enable_tools),
        top_p: None,
        stop_sequences: vec![],
        capture_usage: Some(true),
        capture_reasoning_content: None,
        capture_raw_body: None,
        response_format: None,
        normalize_reasoning_content: None,
        reasoning_effort: None,
    };

    // Create agent
    let agent = DatabaseAgent::new(
        agent_id,
        user_id,
        AgentType::Generic,
        name.to_string(),
        // Empty base instructions - will be set later from context
        String::new(),
        memory,
        DB.clone(),
        model_provider,
        tools,
        embedding_provider,
    );

    // Set the chat options with our selected model
    {
        let mut options = agent.chat_options.write().await;
        *options = Some(response_options);
    }

    // Store the agent in the database
    match agent.store().await {
        Ok(_) => {
            output.success(&format!(
                "Saved new agent '{}' to database",
                name.bright_cyan()
            ));
            println!();
        }
        Err(e) => {
            output.warning(&format!("Failed to save agent to database: {}", e));
            output.status("Agent will work for this session but won't persist");
            println!();
        }
    }

    agent.start_stats_sync().await?;
    agent.start_memory_sync().await?;

    // Add persona as a core memory block if configured
    if let Some(persona) = &config.agent.persona {
        output.status("Adding persona to agent's core memory...");
        let persona_block = MemoryBlock::owned(config.user.id.clone(), "persona", persona.clone())
            .with_description("Agent's persona and identity")
            .with_permission(pattern_core::memory::MemoryPermission::ReadOnly);

        if let Err(e) = agent.update_memory("persona", persona_block).await {
            output.warning(&format!("Failed to add persona memory: {}", e));
        }
    }

    // Add any pre-configured memory blocks
    for (label, block_config) in &config.agent.memory {
        output.info("Adding memory block", &label.bright_yellow().to_string());
        let memory_block = MemoryBlock::owned(
            config.user.id.clone(),
            label.clone(),
            block_config.content.clone(),
        )
        .with_memory_type(block_config.memory_type)
        .with_permission(block_config.permission);

        let memory_block = if let Some(desc) = &block_config.description {
            memory_block.with_description(desc.clone())
        } else {
            memory_block
        };

        if let Err(e) = agent.update_memory(label, memory_block).await {
            output.warning(&format!("Failed to add memory block '{}': {}", label, e));
        }
    }

    Ok(Arc::new(agent))
}

/// Chat with an agent
pub async fn chat_with_agent(agent: Arc<dyn Agent>) -> Result<()> {
    use rustyline::DefaultEditor;

    let output = Output::new();

    output.status("Type 'quit' or 'exit' to leave the chat");
    output.status("Use Ctrl+D for multiline input, Enter to send");
    println!();

    let mut rl = DefaultEditor::new().into_diagnostic()?;
    let prompt = format!("{} ", ">".bright_blue());

    loop {
        let readline = rl.readline(&prompt);
        match readline {
            Ok(line) => {
                if line.trim().is_empty() {
                    continue;
                }

                if line.trim() == "quit" || line.trim() == "exit" {
                    output.status("Goodbye!");
                    break;
                }

                // Add to history
                let _ = rl.add_history_entry(line.as_str());

                // Create a message using the actual Message structure
                let message = Message {
                    content: MessageContent::Text(line.clone()),
                    word_count: line.split_whitespace().count() as u32,
                    ..Default::default()
                };

                // Process message
                output.status("Thinking...");
                match agent.process_message(message).await {
                    Ok(response) => {
                        // Print response
                        let agent_name = agent.name();
                        for content in &response.content {
                            match content {
                                MessageContent::Text(text) => {
                                    output.agent_message(&agent_name, text);
                                }
                                MessageContent::ToolCalls(calls) => {
                                    for call in calls {
                                        output.tool_call(
                                            &call.fn_name,
                                            &serde_json::to_string_pretty(&call.fn_arguments)
                                                .unwrap_or_else(|_| call.fn_arguments.to_string()),
                                        );
                                    }
                                }
                                MessageContent::ToolResponses(responses) => {
                                    for resp in responses {
                                        output.tool_result(&resp.content);
                                    }
                                }
                                MessageContent::Parts(_) => {
                                    // Multi-part content, just show text parts for now
                                    output.status("[Multi-part content]");
                                }
                            }
                        }

                        // Also show tool calls from the response object
                        if !response.tool_calls.is_empty() {
                            for call in &response.tool_calls {
                                output.tool_call(
                                    &call.fn_name,
                                    &serde_json::to_string_pretty(&call.fn_arguments)
                                        .unwrap_or_else(|_| call.fn_arguments.to_string()),
                                );
                            }
                        }
                    }
                    Err(e) => {
                        output.error(&format!("Error: {}", e));
                    }
                }
            }
            Err(rustyline::error::ReadlineError::Interrupted) => {
                output.status("CTRL-C");
                continue;
            }
            Err(rustyline::error::ReadlineError::Eof) => {
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

/// Chat with a group of agents
pub async fn chat_with_group<M: GroupManager>(
    group: AgentGroup,
    agents: Vec<Arc<dyn Agent>>,
    pattern_manager: M,
) -> Result<()> {
    use pattern_core::coordination::groups::AgentWithMembership;
    use rustyline::DefaultEditor;

    let output = Output::new();

    output.status(&format!(
        "Chatting with group '{}'",
        group.name.bright_cyan()
    ));
    output.info("Pattern:", &format!("{:?}", group.coordination_pattern));
    output.info("Members:", &format!("{} agents", agents.len()));
    output.status("Type 'quit' or 'exit' to leave the chat");
    output.status("Use Ctrl+D for multiline input, Enter to send");
    println!();

    let mut rl = DefaultEditor::new().into_diagnostic()?;
    let prompt = format!("{} ", ">".bright_blue());

    // Wrap agents with their membership data
    let agents_with_membership: Vec<AgentWithMembership<Arc<dyn Agent>>> = agents
        .into_iter()
        .zip(group.members.iter())
        .map(|(agent, (_, membership))| AgentWithMembership {
            agent,
            membership: membership.clone(),
        })
        .collect();

    loop {
        let readline = rl.readline(&prompt);
        match readline {
            Ok(line) => {
                if line.trim().is_empty() {
                    continue;
                }

                if line.trim() == "quit" || line.trim() == "exit" {
                    output.status("Goodbye!");
                    break;
                }

                // Add to history
                let _ = rl.add_history_entry(line.as_str());

                // Create a message
                let message = Message {
                    content: MessageContent::Text(line.clone()),
                    word_count: line.split_whitespace().count() as u32,
                    ..Default::default()
                };

                // Route through the group
                output.status("Routing message through group...");
                match pattern_manager
                    .route_message(&group, &agents_with_membership, message)
                    .await
                {
                    Ok(group_response) => {
                        // Display responses from each agent
                        for agent_response in &group_response.responses {
                            // Find agent name
                            let agent_name = agents_with_membership
                                .iter()
                                .find(|a| a.agent.id() == agent_response.agent_id)
                                .map(|a| a.agent.name())
                                .unwrap_or("Unknown Agent".to_string());

                            // Display the response
                            for content in &agent_response.response.content {
                                match content {
                                    MessageContent::Text(text) => {
                                        output.agent_message(&agent_name, text);
                                    }
                                    MessageContent::ToolCalls(calls) => {
                                        for call in calls {
                                            output.tool_call(
                                                &call.fn_name,
                                                &serde_json::to_string_pretty(&call.fn_arguments)
                                                    .unwrap_or_else(|_| {
                                                        call.fn_arguments.to_string()
                                                    }),
                                            );
                                        }
                                    }
                                    MessageContent::ToolResponses(responses) => {
                                        for resp in responses {
                                            output.tool_result(&resp.content);
                                        }
                                    }
                                    MessageContent::Parts(_) => {
                                        output.status("[Multi-part content]");
                                    }
                                }
                            }
                        }

                        // Show execution time
                        output.info(
                            "Execution time:",
                            &format!("{:?}", group_response.execution_time),
                        );
                    }
                    Err(e) => {
                        output.error(&format!("Error routing message: {}", e));
                    }
                }
            }
            Err(rustyline::error::ReadlineError::Interrupted) => {
                output.status("CTRL-C");
                continue;
            }
            Err(rustyline::error::ReadlineError::Eof) => {
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
