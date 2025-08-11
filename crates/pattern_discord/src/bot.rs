use serenity::{
    async_trait,
    builder::{CreateCommand, CreateInteractionResponse, CreateInteractionResponseMessage},
    client::{Context, EventHandler},
    model::{
        application::{Command, CommandInteraction, Interaction},
        channel::Message,
        gateway::Ready,
        id::ChannelId,
    },
};
use std::sync::Arc;
use std::time::Duration;
use tracing::{debug, error, info, warn};

use futures::StreamExt;
use pattern_core::message::Message as PatternMessage;
use pattern_core::{
    Agent, AgentGroup, UserId,
    coordination::groups::{AgentWithMembership, GroupManager},
};

/// The main Discord bot that handles all Discord interactions
pub struct DiscordBot {
    /// Whether we're in CLI mode (single user, no database)
    cli_mode: bool,
    /// Agents with membership data for CLI mode
    agents_with_membership: Option<Vec<AgentWithMembership<Arc<dyn Agent>>>>,
    /// Group for CLI mode
    group: Option<AgentGroup>,
    /// Group manager for CLI mode
    group_manager: Option<Arc<dyn GroupManager>>,
    /// Hardcoded user ID for CLI mode
    #[allow(dead_code)]
    cli_user_id: UserId,
    /// Default channel for responses (when DISCORD_CHANNEL_ID is set)
    default_channel: Option<ChannelId>,
    /// Application ID
    #[allow(dead_code)]
    app_id: Option<String>,

    /// Public key for interactions
    #[allow(dead_code)]
    public_key: Option<String>,
}

/// Configuration for the Discord bot
#[derive(Debug, Clone)]
pub struct DiscordBotConfig {
    pub token: String,
    pub prefix: String,
    pub intents: serenity::all::GatewayIntents,
    pub allowed_channels: Option<Vec<String>>,
    pub admin_users: Option<Vec<String>>,
}

impl DiscordBotConfig {
    pub fn new(token: impl Into<String>) -> Self {
        Self {
            token: token.into(),
            prefix: "!".to_string(),
            intents: serenity::all::GatewayIntents::default(),
            allowed_channels: None,
            admin_users: None,
        }
    }
}

impl DiscordBot {
    /// Create a new Discord bot for CLI mode
    pub fn new_cli_mode(
        agents_with_membership: Vec<AgentWithMembership<Arc<dyn Agent>>>,
        group: AgentGroup,
        group_manager: Arc<dyn GroupManager>,
    ) -> Self {
        let default_channel = std::env::var("DISCORD_CHANNEL_ID")
            .ok()
            .and_then(|id| id.parse::<u64>().ok())
            .map(ChannelId::new);

        let app_id = std::env::var("APP_ID").ok();
        let public_key = std::env::var("PUBLIC_KEY").ok();

        if let Some(ref id) = app_id {
            info!("Discord App ID: {}", id);
        }
        if public_key.is_some() {
            info!("Discord Public Key: configured");
        }

        Self {
            cli_mode: true,
            agents_with_membership: Some(agents_with_membership),
            group: Some(group),
            group_manager: Some(group_manager),
            cli_user_id: UserId("188aa44f0c9f458e8adb4232332ce8fe".to_string()), // Fixed user ID for CLI mode
            default_channel,
            app_id,
            public_key,
        }
    }

    /// Create a new Discord bot for full mode (with database)
    pub fn new_full_mode() -> Self {
        let app_id = std::env::var("APP_ID").ok();
        let public_key = std::env::var("PUBLIC_KEY").ok();

        Self {
            cli_mode: false,
            agents_with_membership: None,
            group: None,
            group_manager: None,
            cli_user_id: UserId::nil(),
            default_channel: None,
            app_id,
            public_key,
        }
    }
}

#[async_trait]
impl EventHandler for DiscordBot {
    async fn ready(&self, ctx: Context, ready: Ready) {
        info!("{} is connected!", ready.user.name);
        info!(
            "Bot user ID: {} (set DISCORD_USER_ID={} to ignore own messages)",
            ready.user.id, ready.user.id
        );

        // Register slash commands
        let commands = vec![
            CreateCommand::new("chat")
                .description("Chat with Pattern agents")
                .dm_permission(true),
            CreateCommand::new("status")
                .description("Check Pattern status")
                .dm_permission(true),
            CreateCommand::new("memory")
                .description("View memory blocks")
                .dm_permission(true),
            CreateCommand::new("help")
                .description("Show available commands")
                .dm_permission(true),
        ];

        for command in commands {
            match Command::create_global_command(&ctx.http, command).await {
                Ok(cmd) => {
                    debug!("Registered command: {}", cmd.name);
                }
                Err(e) => {
                    error!("Failed to register command: {}", e);
                }
            }
        }
    }

    async fn message(&self, ctx: Context, msg: Message) {
        // Ignore bot's own messages
        if msg.author.bot {
            return;
        }

        // Check if we should respond
        let should_respond = {
            let is_dm = msg.guild_id.is_none();
            let is_mention = msg.mentions_me(&ctx.http).await.unwrap_or(false);

            // In CLI mode with a configured channel, respond to all messages in that channel
            if self.cli_mode {
                if let Some(channel) = self.default_channel {
                    if msg.channel_id == channel {
                        true
                    } else {
                        is_dm || is_mention
                    }
                } else {
                    is_dm || is_mention
                }
            } else {
                // Otherwise respond to DMs and mentions
                is_dm || is_mention
            }
        };

        if !should_respond {
            return;
        }

        // Show typing indicator
        let _ = msg.channel_id.start_typing(&ctx.http);

        // Process the message
        if let Err(e) = self.process_message(&ctx, &msg).await {
            error!("Error processing message: {}", e);
            let _ = msg
                .reply(
                    &ctx.http,
                    "Sorry, I encountered an error processing your message.",
                )
                .await;
        }
    }

    async fn interaction_create(&self, ctx: Context, interaction: Interaction) {
        if let Interaction::Command(command) = interaction {
            info!(
                "Received slash command: {} from {}",
                command.data.name, command.user.name
            );

            match command.data.name.as_str() {
                "chat" => self.handle_chat_command(&ctx, &command).await,
                "status" => self.handle_status_command(&ctx, &command).await,
                "memory" => self.handle_memory_command(&ctx, &command).await,
                "help" => self.handle_help_command(&ctx, &command).await,
                _ => {
                    warn!("Unknown command: {}", command.data.name);
                }
            }
        }
    }
}

impl DiscordBot {
    /// Select the appropriate group based on message content
    #[allow(dead_code)]
    fn select_group_for_message(&self, message: &str) -> String {
        let message_lower = message.to_lowercase();

        // Crisis detection - urgent/panic language
        if self.is_crisis_message(&message_lower) {
            return "crisis".to_string();
        }

        // Planning detection - task/organization keywords
        if self.is_planning_message(&message_lower) {
            return "planning".to_string();
        }

        // Memory/recall detection
        if self.is_memory_message(&message_lower) {
            return "memory".to_string();
        }

        // Default to main group
        "Pattern Cluster".to_string()
    }

    fn is_crisis_message(&self, message: &str) -> bool {
        let crisis_keywords = [
            "help",
            "panic",
            "spiral",
            "can't",
            "overwhelming",
            "freaking out",
            "emergency",
            "crisis",
            "meltdown",
            "losing it",
            "falling apart",
            "too much",
            "stuck",
        ];

        crisis_keywords.iter().any(|&kw| message.contains(kw))
    }

    fn is_planning_message(&self, message: &str) -> bool {
        let planning_keywords = [
            "plan",
            "organize",
            "schedule",
            "prioritize",
            "break down",
            "todo",
            "task",
            "project",
            "deadline",
            "steps",
            "how do i",
            "where do i start",
            "need to",
        ];

        planning_keywords.iter().any(|&kw| message.contains(kw))
    }

    fn is_memory_message(&self, message: &str) -> bool {
        let memory_keywords = [
            "remember",
            "recall",
            "forgot",
            "what was",
            "what did",
            "last time",
            "yesterday",
            "earlier",
            "before",
            "previous",
            "working on",
            "talked about",
            "mentioned",
        ];

        memory_keywords.iter().any(|&kw| message.contains(kw))
    }

    /// Process a Discord message and route it to Pattern agents
    async fn process_message(&self, ctx: &Context, msg: &Message) -> Result<(), String> {
        if self.cli_mode {
            // Create Pattern message
            let pattern_msg = PatternMessage::user(msg.content.clone());

            // Check if we have a group setup
            if let (Some(group), Some(agents_with_membership), Some(group_manager)) = (
                &self.group,
                &self.agents_with_membership,
                &self.group_manager,
            ) {
                // Log which coordination pattern we're using
                info!(
                    "Routing message using {:?} coordination pattern",
                    group.coordination_pattern
                );

                // Route through group manager using the real agents with membership
                let mut response_stream = group_manager
                    .route_message(group, agents_with_membership, pattern_msg)
                    .await
                    .map_err(|e| format!("Failed to route message: {}", e))?;

                // Set up idle timeout - resets on any activity
                let idle_timeout = Duration::from_secs(120); // 2 minutes of inactivity
                let mut last_activity = tokio::time::Instant::now();

                // Track state
                let mut current_message = String::new();
                let mut has_sent_initial_response = false;
                let mut active_agents: usize = 0;
                let mut completed_agents = 0;

                // Process stream with idle timeout
                loop {
                    match tokio::time::timeout_at(
                        last_activity + idle_timeout,
                        response_stream.next(),
                    )
                    .await
                    {
                        Ok(Some(event)) => {
                            // Reset idle timer on any event
                            last_activity = tokio::time::Instant::now();

                            match event {
                                pattern_core::coordination::groups::GroupResponseEvent::TextChunk { agent_id: _, text, is_final } => {
                                    if !text.is_empty() {
                                        current_message.push_str(&text);

                                        // Send complete sentences/paragraphs as they arrive
                                        if is_final || text.ends_with('\n') || text.ends_with(". ") || current_message.len() > 1500 {
                                            if !current_message.trim().is_empty() {
                                                for chunk in split_message(&current_message, 2000) {
                                                    if let Err(e) = msg.reply(&ctx.http, chunk).await {
                                                        warn!("Failed to send response chunk: {}", e);
                                                    }
                                                    has_sent_initial_response = true;
                                                }
                                                current_message.clear();
                                            }
                                        }
                                    }
                                },
                                pattern_core::coordination::groups::GroupResponseEvent::ToolCallStarted { agent_id: _, call_id, fn_name, args } => {
                                    debug!("Tool call started: {} ({})", fn_name, call_id);

                                    // Check if this is a send_message tool call
                                    if fn_name == "send_message" {
                                        // Try to extract the content from args
                                        if let Some(content) = args.get("content").and_then(|v| v.as_str()) {
                                            // Send the message content directly
                                            for chunk in split_message(content, 2000) {
                                                if let Err(e) = msg.reply(&ctx.http, chunk).await {
                                                    warn!("Failed to send message from send_message tool: {}", e);
                                                }
                                            }
                                            has_sent_initial_response = true;
                                        }
                                    } else if !has_sent_initial_response {
                                        // Show tool activity if we haven't sent anything yet
                                        let tool_msg = match fn_name.as_str() {
                                            "context" => "💭 Agent is accessing memory...".to_string(),
                                            "recall" => "🔍 Agent is searching memories...".to_string(),
                                            "search" => "🔎 Agent is searching...".to_string(),
                                            _ => format!("🔧 Agent is using {}", fn_name)
                                        };
                                        if let Err(e) = msg.reply(&ctx.http, tool_msg).await {
                                            debug!("Failed to send tool activity: {}", e);
                                        }
                                        has_sent_initial_response = true;
                                    }
                                },
                                pattern_core::coordination::groups::GroupResponseEvent::ToolCallCompleted { agent_id: _, call_id, result } => {
                                    debug!("Tool call completed: {} - {:?}", call_id, result);

                                    // Check if this was a send_message tool that succeeded
                                    if let Ok(result_str) = &result {
                                        if result_str.contains("Message sent successfully") {
                                            debug!("send_message tool completed successfully");
                                        }
                                    }
                                },
                                pattern_core::coordination::groups::GroupResponseEvent::Error { agent_id: _, message, recoverable } => {
                                    warn!("Agent error: {} (recoverable: {})", message, recoverable);
                                    let error_msg = if recoverable {
                                        format!("⚠️ {}", message)
                                    } else {
                                        format!("❌ Error: {}", message)
                                    };
                                    if let Err(e) = msg.reply(&ctx.http, error_msg).await {
                                        warn!("Failed to send error message: {}", e);
                                    }
                                    if !recoverable {
                                        break; // Stop processing on non-recoverable errors
                                    }
                                },
                                pattern_core::coordination::groups::GroupResponseEvent::AgentStarted { agent_name, .. } => {
                                    debug!("Agent {} started processing", agent_name);
                                    active_agents += 1;
                                },
                                pattern_core::coordination::groups::GroupResponseEvent::AgentCompleted { agent_name, .. } => {
                                    debug!("Agent {} completed processing", agent_name);
                                    completed_agents += 1;
                                    active_agents = active_agents.saturating_sub(1);

                                    // If all agents have completed, we can exit
                                    if active_agents == 0 && completed_agents > 0 {
                                        break;
                                    }
                                },
                                _ => {} // Ignore other events for now
                            }
                        }
                        Ok(None) => {
                            // Stream ended normally
                            break;
                        }
                        Err(_) => {
                            // Idle timeout
                            let timeout_msg = if has_sent_initial_response {
                                "⏱️ (No further activity for 2 minutes - agents may still be processing)"
                            } else {
                                "⏱️ Request timed out after 2 minutes of inactivity. No agents responded."
                            };
                            if let Err(e) = msg.reply(&ctx.http, timeout_msg).await {
                                warn!("Failed to send timeout message: {}", e);
                            }
                            break;
                        }
                    }
                }

                // Send any remaining buffered content
                if !current_message.trim().is_empty() {
                    for chunk in split_message(&current_message, 2000) {
                        if let Err(e) = msg.reply(&ctx.http, chunk).await {
                            warn!("Failed to send final response chunk: {}", e);
                        }
                    }
                }

                // If we never sent anything, send a status message
                if !has_sent_initial_response {
                    let status_msg = if active_agents > 0 {
                        "The agents started processing but produced no response."
                    } else {
                        "No agents were available to process your message."
                    };
                    if let Err(e) = msg.reply(&ctx.http, status_msg).await {
                        warn!("Failed to send status message: {}", e);
                    }
                }
            } else {
                // Fallback for single agent mode
                if let Some(agents_with_membership) = &self.agents_with_membership {
                    if let Some(awm) = agents_with_membership.first() {
                        let agent = &awm.agent;
                        // Direct agent call
                        let mut stream = agent
                            .clone()
                            .process_message_stream(pattern_msg)
                            .await
                            .map_err(|e| format!("Failed to process message: {}", e))?;

                        let mut response = String::new();
                        while let Some(event) = stream.next().await {
                            match event {
                                pattern_core::agent::ResponseEvent::TextChunk { text, .. } => {
                                    response.push_str(&text);
                                }
                                _ => {} // Ignore other events
                            }
                        }

                        if response.is_empty() {
                            response = "No response from agent.".to_string();
                        }

                        msg.reply(&ctx.http, response)
                            .await
                            .map_err(|e| format!("Failed to send reply: {}", e))?;
                    }
                }
            }
        } else {
            // TODO: Implement full database mode with user lookup
            msg.reply(&ctx.http, "Full mode not yet implemented")
                .await
                .map_err(|e| format!("Failed to reply: {}", e))?;
        }
        Ok(())
    }

    /// Handle the /chat command
    async fn handle_chat_command(&self, ctx: &Context, command: &CommandInteraction) {
        // TODO: Implement chat command
        if let Err(e) = command
            .create_response(
                &ctx.http,
                CreateInteractionResponse::Message(
                    CreateInteractionResponseMessage::new()
                        .content("Chat command not yet implemented"),
                ),
            )
            .await
        {
            error!("Failed to respond to chat command: {}", e);
        }
    }

    /// Handle the /status command
    async fn handle_status_command(&self, ctx: &Context, command: &CommandInteraction) {
        // TODO: Implement status command
        if let Err(e) = command
            .create_response(
                &ctx.http,
                CreateInteractionResponse::Message(
                    CreateInteractionResponseMessage::new()
                        .content("Status command not yet implemented"),
                ),
            )
            .await
        {
            error!("Failed to respond to status command: {}", e);
        }
    }

    /// Handle the /memory command
    async fn handle_memory_command(&self, ctx: &Context, command: &CommandInteraction) {
        // TODO: Implement memory command
        if let Err(e) = command
            .create_response(
                &ctx.http,
                CreateInteractionResponse::Message(
                    CreateInteractionResponseMessage::new()
                        .content("Memory command not yet implemented"),
                ),
            )
            .await
        {
            error!("Failed to respond to memory command: {}", e);
        }
    }

    /// Handle the /help command
    async fn handle_help_command(&self, ctx: &Context, command: &CommandInteraction) {
        let help_text = "**Pattern Discord Bot Commands**\n\
            `/chat` - Chat with Pattern agents\n\
            `/status` - Check bot and agent status\n\
            `/memory` - View memory blocks\n\
            `/help` - Show this help message";

        if let Err(e) = command
            .create_response(
                &ctx.http,
                CreateInteractionResponse::Message(
                    CreateInteractionResponseMessage::new().content(help_text),
                ),
            )
            .await
        {
            error!("Failed to respond to help command: {}", e);
        }
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
                chunks.push(current.trim().to_string());
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
        chunks.push(current.trim().to_string());
    }

    chunks
}
