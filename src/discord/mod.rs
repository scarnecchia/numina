use crate::{agent::MultiAgentSystem, agent::UserId};
use miette::{IntoDiagnostic, Result};
use serde::{Deserialize, Serialize};
use serenity::{
    all::{
        Command, CreateInteractionResponse, CreateInteractionResponseMessage,
        EditInteractionResponse,
    },
    async_trait,
    builder::{CreateCommand, CreateEmbed},
    client::{Context, EventHandler},
    model::{
        application::{CommandInteraction, Interaction},
        channel::Message,
        gateway::Ready,
    },
    prelude::*,
};
use std::fmt;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{error, info, warn};

/// Discord channel reference
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ChannelId(pub u64);

impl fmt::Display for ChannelId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Discord bot configuration
#[derive(Debug, Clone)]
pub struct DiscordConfig {
    /// Discord bot token
    pub token: String,
    /// Discord application ID
    pub application_id: Option<u64>,
    /// Optional channel ID to limit bot responses
    pub channel_id: Option<u64>,
    /// Whether to respond to DMs
    pub respond_to_dms: bool,
    /// Whether to respond to mentions
    pub respond_to_mentions: bool,
    /// Maximum message length before chunking
    pub max_message_length: usize,
}

impl Default for DiscordConfig {
    fn default() -> Self {
        Self {
            token: String::new(),
            application_id: None,
            channel_id: None,
            respond_to_dms: true,
            respond_to_mentions: true,
            max_message_length: 2000,
        }
    }
}

/// Shared state for the Discord bot
#[derive(Clone)]
pub struct BotState {
    pub multi_agent_system: Arc<MultiAgentSystem>,
}

/// Discord bot handler
pub struct PatternDiscordBot {
    state: Arc<RwLock<BotState>>,
}

impl PatternDiscordBot {
    pub fn new(multi_agent_system: Arc<MultiAgentSystem>) -> Self {
        Self {
            state: Arc::new(RwLock::new(BotState { multi_agent_system })),
        }
    }
}

#[async_trait]
impl EventHandler for PatternDiscordBot {
    async fn ready(&self, ctx: Context, ready: Ready) {
        info!("{} is connected!", ready.user.name);

        // Register slash commands
        let commands = vec![
            CreateCommand::new("chat")
                .description("Chat with Pattern agents")
                .add_option(
                    serenity::all::CreateCommandOption::new(
                        serenity::all::CommandOptionType::String,
                        "message",
                        "Your message to the agents",
                    )
                    .required(true),
                )
                .add_option(
                    serenity::all::CreateCommandOption::new(
                        serenity::all::CommandOptionType::String,
                        "agent",
                        "Specific agent to talk to (optional)",
                    )
                    .required(false),
                )
                .add_option(
                    serenity::all::CreateCommandOption::new(
                        serenity::all::CommandOptionType::Boolean,
                        "private",
                        "Make this conversation visible only to you (default: false)",
                    )
                    .required(false),
                ),
            CreateCommand::new("vibe").description("Check current vibe/state"),
            CreateCommand::new("tasks").description("List current tasks"),
            CreateCommand::new("memory").description("Show shared memory state"),
            CreateCommand::new("debug_agents")
                .description("Debug: Log agent chat histories to disk")
                .add_option(
                    serenity::all::CreateCommandOption::new(
                        serenity::all::CommandOptionType::Boolean,
                        "private",
                        "Make this response visible only to you (default: true)",
                    )
                    .required(false),
                ),
        ];

        for command in commands {
            if let Err(why) = Command::create_global_command(&ctx.http, command).await {
                error!("Cannot create slash command: {:?}", why);
            }
        }
    }

    async fn message(&self, ctx: Context, msg: Message) {
        // Ignore bot's own messages
        if msg.author.bot {
            return;
        }

        let should_respond = {
            let _state = self.state.read().await;

            // Check if we should respond based on message type
            let is_dm = msg.guild_id.is_none();
            let is_mention = msg.mentions_me(&ctx.http).await.unwrap_or(false);

            // Only respond to DMs and mentions, not all messages
            is_dm || is_mention
        };

        if !should_respond {
            return;
        }

        // Show typing indicator
        let typing = msg.channel_id.start_typing(&ctx.http);

        // Process message with agents
        match self.process_message(&ctx, &msg).await {
            Ok(response) => {
                // Split response into chunks if needed
                for chunk in split_message(&response, 2000) {
                    if let Err(why) = msg.channel_id.say(&ctx.http, chunk).await {
                        error!("Error sending message: {:?}", why);
                    }
                }
            }
            Err(e) => {
                error!("Error processing message: {:?}", e);
                let _ = msg
                    .reply(
                        &ctx.http,
                        "Sorry, I encountered an error processing your message.",
                    )
                    .await;
            }
        }

        // Stop typing indicator
        typing.stop();
    }

    async fn interaction_create(&self, ctx: Context, interaction: Interaction) {
        if let Interaction::Command(command) = interaction {
            info!(
                "Received slash command: {} from user {}",
                command.data.name, command.user.name
            );
            match command.data.name.as_str() {
                "chat" => self.handle_chat_command(&ctx, &command).await,
                "vibe" => self.handle_vibe_command(&ctx, &command).await,
                "tasks" => self.handle_tasks_command(&ctx, &command).await,
                "memory" => self.handle_memory_command(&ctx, &command).await,
                "debug_agents" => self.handle_debug_agents_command(&ctx, &command).await,
                _ => {
                    warn!("Unknown command: {}", command.data.name);
                }
            }
        }
    }
}

impl PatternDiscordBot {
    async fn process_message(&self, _ctx: &Context, msg: &Message) -> Result<String> {
        let state = self.state.read().await;

        // Convert Discord user ID to our UserId
        let user_id = UserId(msg.author.id.get() as i64);

        // Initialize user if needed
        state.multi_agent_system.initialize_user(user_id).await?;

        // Parse message for agent routing
        let (target_agent, actual_message) = self.parse_agent_routing(&msg.content).await;

        // Update current context with the message
        let context = format!(
            "task: chat | user: {} | message: {} | channel: {} | target: {}",
            msg.author.name,
            actual_message,
            if msg.guild_id.is_none() {
                "DM"
            } else {
                "server"
            },
            target_agent.as_deref().unwrap_or("pattern")
        );

        state
            .multi_agent_system
            .update_shared_memory(user_id, "active_context", &context)
            .await?;

        // Send to the appropriate agent or group and get response
        let response = if let Some(agent) = target_agent {
            // Specific agent requested - send directly
            state
                .multi_agent_system
                .send_message_to_agent(user_id, Some(&agent), &actual_message)
                .await?
        } else {
            // No specific agent - select appropriate group based on message content
            let group_name = self.select_group_for_message(&actual_message);
            info!("Routing message to '{}' group", group_name);

            match state
                .multi_agent_system
                .send_message_to_group(user_id, &group_name, &actual_message)
                .await
            {
                Ok(resp) => {
                    // Extract the best response from the group
                    // For now, just concatenate all responses
                    resp.messages
                        .iter()
                        .filter_map(|msg| match msg {
                            letta::types::LettaMessageUnion::SystemMessage(_) => None,
                            letta::types::LettaMessageUnion::UserMessage(_) => None,
                            letta::types::LettaMessageUnion::AssistantMessage(m) => {
                                Some(m.content.clone())
                            }
                            letta::types::LettaMessageUnion::ReasoningMessage(_) => None,
                            letta::types::LettaMessageUnion::HiddenReasoningMessage(_) => None,
                            letta::types::LettaMessageUnion::ToolCallMessage(_) => None,
                            letta::types::LettaMessageUnion::ToolReturnMessage(_) => None,
                        })
                        .collect::<Vec<_>>()
                        .join("\n\n")
                }
                Err(e) => {
                    warn!(
                        "Failed to send to group, falling back to pattern agent: {}",
                        e
                    );
                    // Fallback to pattern agent
                    state
                        .multi_agent_system
                        .send_message_to_agent(user_id, Some("pattern"), &actual_message)
                        .await?
                }
            }
        };

        Ok(response)
    }

    async fn handle_chat_command(&self, ctx: &Context, command: &CommandInteraction) {
        info!("Processing chat command from user {}", command.user.name);

        // Get the private option
        let is_private = command
            .data
            .options
            .iter()
            .find(|opt| opt.name == "private")
            .and_then(|opt| opt.value.as_bool())
            .unwrap_or(false);

        // Defer response for long operations
        if let Err(why) = command
            .create_response(
                &ctx.http,
                CreateInteractionResponse::Defer(
                    CreateInteractionResponseMessage::new().ephemeral(is_private),
                ),
            )
            .await
        {
            error!("Cannot defer response: {:?}", why);
            return;
        }

        // Get the message from options
        let message = command
            .data
            .options
            .iter()
            .find(|opt| opt.name == "message")
            .and_then(|opt| opt.value.as_str())
            .unwrap_or("Hello");

        let agent = command
            .data
            .options
            .iter()
            .find(|opt| opt.name == "agent")
            .and_then(|opt| opt.value.as_str());

        // Process with agents
        let user_id = UserId(command.user.id.get() as i64);

        // Send to agents
        let state = self.state.read().await;

        // Initialize user if needed (ensures all agents are created)
        if let Err(e) = state.multi_agent_system.initialize_user(user_id).await {
            error!("Failed to initialize user: {:?}", e);
        }

        // Send progress updates for long operations
        let ctx_clone = ctx.clone();
        let command_clone = command.clone();
        let progress_task = tokio::spawn(async move {
            tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
            let _ = command_clone
                .edit_response(
                    &ctx_clone.http,
                    EditInteractionResponse::new()
                        .content("🤔 Still thinking... (Letta can be slow sometimes)"),
                )
                .await;

            tokio::time::sleep(tokio::time::Duration::from_secs(10)).await;
            let _ = command_clone
                .edit_response(
                    &ctx_clone.http,
                    EditInteractionResponse::new()
                        .content("⏳ Taking longer than usual... (cloud agents need time)"),
                )
                .await;

            tokio::time::sleep(tokio::time::Duration::from_secs(15)).await;
            let _ = command_clone
                .edit_response(
                    &ctx_clone.http,
                    EditInteractionResponse::new()
                        .content("😅 Almost there... (seriously, any moment now)"),
                )
                .await;
        });

        let response = match tokio::time::timeout(
            tokio::time::Duration::from_secs(90), // 90 second timeout for cloud agents
            state
                .multi_agent_system
                .send_message_to_agent(user_id, agent, message),
        )
        .await
        {
            Ok(Ok(resp)) => {
                progress_task.abort(); // Cancel progress update
                resp
            }
            Ok(Err(e)) => {
                progress_task.abort();
                error!("Error sending to agent: {:?}", e);

                // Provide detailed error in debug mode
                #[cfg(debug_assertions)]
                let error_msg = format!("Error details:\n```\n{:?}\n```\nThis usually means the agent is still initializing or Letta is slow to respond. Try again in a few seconds.", e);

                #[cfg(not(debug_assertions))]
                let error_msg = format!("Sorry, I encountered an error: {}", e);

                error_msg
            }
            Err(_) => {
                progress_task.abort();
                error!("Request timed out after 90 seconds");
                "Sorry, the request timed out. Letta might be overloaded. Please try again in a moment.".to_string()
            }
        };

        // Send followup
        if let Err(why) = command
            .edit_response(&ctx.http, EditInteractionResponse::new().content(response))
            .await
        {
            error!("Cannot send followup: {:?}", why);
        }
    }

    async fn handle_vibe_command(&self, ctx: &Context, command: &CommandInteraction) {
        let state = self.state.read().await;
        let user_id = UserId(command.user.id.get() as i64);

        // Defer response
        if let Err(why) = command
            .create_response(
                &ctx.http,
                CreateInteractionResponse::Defer(CreateInteractionResponseMessage::new()),
            )
            .await
        {
            error!("Cannot defer response: {:?}", why);
            return;
        }

        // Get current state from shared memory
        match state
            .multi_agent_system
            .get_shared_memory(user_id, "current_state")
            .await
        {
            Ok(current_state) => {
                let embed = CreateEmbed::new()
                    .title("Current Vibe Check")
                    .description(&current_state)
                    .color(0x00ff00);

                if let Err(why) = command
                    .edit_response(&ctx.http, EditInteractionResponse::new().embed(embed))
                    .await
                {
                    error!("Cannot send vibe response: {:?}", why);
                }
            }
            Err(_e) => {
                let _ = command
                    .edit_response(
                        &ctx.http,
                        EditInteractionResponse::new()
                            .content("Couldn't check vibe right now. Try again later?"),
                    )
                    .await;
            }
        }
    }

    async fn handle_tasks_command(&self, ctx: &Context, command: &CommandInteraction) {
        // TODO: Implement when we have task management
        let _ = command
            .create_response(
                &ctx.http,
                CreateInteractionResponse::Message(
                    CreateInteractionResponseMessage::new().content("Task management coming soon!"),
                ),
            )
            .await;
    }

    async fn handle_memory_command(&self, ctx: &Context, command: &CommandInteraction) {
        let state = self.state.read().await;
        let user_id = UserId(command.user.id.get() as i64);

        // Defer response
        if let Err(why) = command
            .create_response(
                &ctx.http,
                CreateInteractionResponse::Defer(CreateInteractionResponseMessage::new()),
            )
            .await
        {
            error!("Cannot defer response: {:?}", why);
            return;
        }

        // Get all shared memory
        match state
            .multi_agent_system
            .get_all_shared_memory(user_id)
            .await
        {
            Ok(memory) => {
                let mut embed = CreateEmbed::new()
                    .title("Shared Memory State")
                    .color(0x3498db);

                for (key, value) in memory {
                    embed = embed.field(&key, &value, false);
                }

                if let Err(why) = command
                    .edit_response(&ctx.http, EditInteractionResponse::new().embed(embed))
                    .await
                {
                    error!("Cannot send memory response: {:?}", why);
                }
            }
            Err(_e) => {
                let _ = command
                    .edit_response(
                        &ctx.http,
                        EditInteractionResponse::new()
                            .content("Couldn't retrieve memory state right now."),
                    )
                    .await;
            }
        }
    }

    async fn handle_debug_agents_command(&self, ctx: &Context, command: &CommandInteraction) {
        info!(
            "Processing debug_agents command from user {}",
            command.user.name
        );

        // Get the private option (default true for debug)
        let is_private = command
            .data
            .options
            .iter()
            .find(|opt| opt.name == "private")
            .and_then(|opt| opt.value.as_bool())
            .unwrap_or(true);

        // Defer response
        if let Err(why) = command
            .create_response(
                &ctx.http,
                CreateInteractionResponse::Defer(
                    CreateInteractionResponseMessage::new().ephemeral(is_private),
                ),
            )
            .await
        {
            error!("Cannot defer response: {:?}", why);
            return;
        }

        // Get user ID
        let user_id = UserId(command.user.id.get() as i64);

        // Log agent histories
        let state = self.state.read().await;
        let log_dir = "logs/agent_histories";

        match state
            .multi_agent_system
            .log_agent_histories(user_id, log_dir)
            .await
        {
            Ok(_) => {
                let response = format!(
                    "✅ Agent histories logged successfully!\n\
                    📁 Check the `{}` directory for timestamped log files.\n\
                    📝 Each agent's full conversation history has been saved.",
                    log_dir
                );

                let _ = command
                    .edit_response(&ctx.http, EditInteractionResponse::new().content(&response))
                    .await;
            }
            Err(e) => {
                error!("Failed to log agent histories: {:?}", e);
                let _ = command
                    .edit_response(
                        &ctx.http,
                        EditInteractionResponse::new().content(
                            "❌ Failed to log agent histories. Check server logs for details.",
                        ),
                    )
                    .await;
            }
        }
    }

    /// Parse message for agent routing directives
    /// Supports formats like:
    /// - "@entropy help me break down this task"
    /// - "flux: when should i schedule this?"
    /// - "/agent momentum how's my energy?"
    /// - Regular message (goes to default agent)
    async fn parse_agent_routing(&self, content: &str) -> (Option<String>, String) {
        // Get configured agents from the system
        let state = self.state.read().await;
        let agent_configs = state.multi_agent_system.agent_configs();
        let valid_agents: Vec<String> = agent_configs.keys().map(|id| id.to_string()).collect();

        // Check for @agent format
        if let Some(rest) = content.strip_prefix('@') {
            if let Some((agent, message)) = rest.split_once(' ') {
                let agent_lower = agent.to_lowercase();
                if valid_agents.contains(&agent_lower) {
                    return (Some(agent_lower), message.trim().to_string());
                }
            }
        }

        // Check for agent: format
        if let Some((possible_agent, message)) = content.split_once(':') {
            let agent = possible_agent.trim().to_lowercase();
            if valid_agents.contains(&agent) {
                return (Some(agent), message.trim().to_string());
            }
        }

        // Check for /agent format
        if let Some(rest) = content.strip_prefix("/agent ") {
            if let Some((agent, message)) = rest.split_once(' ') {
                let agent_lower = agent.to_lowercase();
                if valid_agents.contains(&agent_lower) {
                    return (Some(agent_lower), message.trim().to_string());
                }
            }
        }

        // Default: route to first agent (usually the orchestrator) or None
        let default_agent = valid_agents.first().cloned();
        (default_agent, content.to_string())
    }
}

/// Split a message into chunks that fit Discord's message length limit
fn split_message(content: &str, max_length: usize) -> Vec<String> {
    if content.len() <= max_length {
        return vec![content.to_string()];
    }

    let mut chunks = Vec::new();
    let mut current = String::new();

    for line in content.lines() {
        if current.len() + line.len() + 1 > max_length {
            if !current.is_empty() {
                chunks.push(current);
                current = String::new();
            }

            // If a single line is too long, split it
            if line.len() > max_length {
                for chunk in line.chars().collect::<Vec<_>>().chunks(max_length) {
                    chunks.push(chunk.iter().collect());
                }
            } else {
                current = line.to_string();
            }
        } else {
            if !current.is_empty() {
                current.push('\n');
            }
            current.push_str(line);
        }
    }

    if !current.is_empty() {
        chunks.push(current);
    }

    chunks
}

/// Create the Discord client (without starting it)
pub async fn create_discord_client(
    config: &DiscordConfig,
    multi_agent_system: Arc<MultiAgentSystem>,
) -> Result<serenity::Client> {
    let handler = PatternDiscordBot::new(multi_agent_system);

    let intents = GatewayIntents::GUILD_MESSAGES
        | GatewayIntents::DIRECT_MESSAGES
        | GatewayIntents::MESSAGE_CONTENT;

    let mut client_builder = Client::builder(&config.token, intents).event_handler(handler);

    if let Some(app_id) = config.application_id {
        client_builder = client_builder.application_id(app_id.into());
    }

    let client = client_builder.await.into_diagnostic()?;

    Ok(client)
}

/// Create and run the Discord bot
pub async fn run_discord_bot(
    config: DiscordConfig,
    multi_agent_system: Arc<MultiAgentSystem>,
) -> Result<()> {
    let mut client = create_discord_client(&config, multi_agent_system).await?;

    info!("Starting Discord bot...");
    client.start().await.into_diagnostic()?;

    Ok(())
}
