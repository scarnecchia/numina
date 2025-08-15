use serenity::{
    async_trait,
    builder::{CreateInteractionResponse, CreateInteractionResponseMessage},
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

use std::collections::{HashMap, VecDeque};
use tokio::sync::Mutex;

/// Buffered reaction for batch processing
#[derive(Debug, Clone)]
struct BufferedReaction {
    emoji: String,
    user_name: String,
    message_preview: String,
    #[allow(dead_code)]
    channel_id: u64,
    timestamp: std::time::Instant,
}

/// Queued message for processing
#[derive(Debug, Clone)]
struct QueuedMessage {
    msg_id: u64,
    channel_id: u64,
    author_name: String,
    content: String,
    timestamp: std::time::Instant,
}

/// The main Discord bot that handles all Discord interactions
#[derive(Clone)]
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

    /// Buffer for reactions to batch process
    reaction_buffer: Arc<Mutex<VecDeque<BufferedReaction>>>,
    /// Whether we're currently processing a message
    is_processing: Arc<Mutex<bool>>,
    /// Last message time for debouncing
    last_message_time: Arc<Mutex<std::time::Instant>>,
    /// Queue of messages to process
    message_queue: Arc<Mutex<VecDeque<QueuedMessage>>>,
    /// Currently processing message ID (for reply attachment)
    current_message_id: Arc<Mutex<Option<u64>>>,
    /// When we started processing the current message
    current_message_start: Arc<Mutex<Option<std::time::Instant>>>,
    /// Handle for typing indicator task
    typing_handle: Arc<Mutex<Option<tokio::task::JoinHandle<()>>>>,
    /// Track status reactions we've added (message_id -> reaction)
    status_reactions: Arc<Mutex<HashMap<u64, char>>>,
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
            reaction_buffer: Arc::new(Mutex::new(VecDeque::new())),
            is_processing: Arc::new(Mutex::new(false)),
            last_message_time: Arc::new(Mutex::new(std::time::Instant::now())),
            message_queue: Arc::new(Mutex::new(VecDeque::new())),
            current_message_id: Arc::new(Mutex::new(None)),
            current_message_start: Arc::new(Mutex::new(None)),
            typing_handle: Arc::new(Mutex::new(None)),
            status_reactions: Arc::new(Mutex::new(HashMap::new())),
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
            reaction_buffer: Arc::new(Mutex::new(VecDeque::new())),
            is_processing: Arc::new(Mutex::new(false)),
            last_message_time: Arc::new(Mutex::new(std::time::Instant::now())),
            message_queue: Arc::new(Mutex::new(VecDeque::new())),
            current_message_id: Arc::new(Mutex::new(None)),
            current_message_start: Arc::new(Mutex::new(None)),
            typing_handle: Arc::new(Mutex::new(None)),
            status_reactions: Arc::new(Mutex::new(HashMap::new())),
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

        // Register slash commands using the new comprehensive implementations
        let commands = crate::slash_commands::create_commands();

        for command in commands {
            match Command::create_global_command(&ctx.http, command).await {
                Ok(cmd) => {
                    info!("Registered command: {}", cmd.name);
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

        // Log message context for debugging
        info!(
            "Received message - Guild: {:?}, Channel: {}, Author: {} ({}), Content length: {}",
            msg.guild_id,
            msg.channel_id,
            msg.author.name,
            msg.author.id,
            msg.content.len()
        );

        // Check if this is a thread and log it
        if let Ok(channel) = msg.channel_id.to_channel(&ctx).await {
            match channel {
                serenity::model::channel::Channel::Guild(guild_channel) => {
                    if guild_channel.thread_metadata.is_some() {
                        info!(
                            "Message is in a thread: {} (parent: {:?})",
                            guild_channel.name, guild_channel.parent_id
                        );
                    }
                }
                _ => {}
            }
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

        // Check if we're currently processing a message
        let is_busy = *self.is_processing.lock().await;

        if is_busy {
            // Add to queue instead of processing immediately
            let mut queue = self.message_queue.lock().await;

            // Try to merge with existing message from same user in same channel
            let mut merged = false;
            for queued_msg in queue.iter_mut() {
                if queued_msg.author_name == msg.author.name
                    && queued_msg.channel_id == msg.channel_id.get()
                {
                    // Merge messages from same user
                    info!(
                        "Merging message from {} into existing queue entry",
                        msg.author.name
                    );

                    // Calculate time since the original message
                    let time_diff = queued_msg.timestamp.elapsed().as_secs();

                    // Append the new content with separator and timestamp
                    queued_msg
                        .content
                        .push_str(&format!("\n--- [+{}s later] ---\n", time_diff));
                    queued_msg.content.push_str(&msg.content);

                    // Update the message ID to the latest one for reply purposes
                    queued_msg.msg_id = msg.id.get();

                    merged = true;
                    break;
                }
            }

            if !merged {
                // Check if we have room for a new entry
                if queue.len() >= 4 {
                    // Try to find an entry to merge with based on channel
                    let mut channel_merged = false;
                    for queued_msg in queue.iter_mut() {
                        if queued_msg.channel_id == msg.channel_id.get() {
                            info!(
                                "Queue full, merging message from {} into existing channel entry",
                                msg.author.name
                            );

                            // Calculate time since the original message
                            let time_diff = queued_msg.timestamp.elapsed().as_secs();

                            // Merge as different user in same channel with timestamp
                            queued_msg.content.push_str(&format!(
                                "\n\n[Also from {} - +{}s later]:\n{}",
                                msg.author.name, time_diff, msg.content
                            ));
                            queued_msg.msg_id = msg.id.get();

                            channel_merged = true;
                            break;
                        }
                    }

                    if !channel_merged {
                        // Last resort: merge into the last entry
                        if let Some(last_msg) = queue.back_mut() {
                            info!(
                                "Queue full, force-merging message from {} into last entry",
                                msg.author.name
                            );
                            last_msg.content.push_str(&format!(
                                "\n\n[Also from {} in different context]:\n{}",
                                msg.author.name, msg.content
                            ));
                            last_msg.msg_id = msg.id.get();
                        }
                    }
                } else {
                    // Add as new queue entry
                    queue.push_back(QueuedMessage {
                        msg_id: msg.id.get(),
                        channel_id: msg.channel_id.get(),
                        author_name: msg.author.name.clone(),
                        content: msg.content.clone(),
                        timestamp: std::time::Instant::now(),
                    });
                }
            }

            info!(
                "Queue status: {} entries after processing message from {}",
                queue.len(),
                msg.author.name
            );

            // Show indicator based on whether we merged or queued
            let indicator = if merged { '🔄' } else { '📥' };
            if msg.react(&ctx.http, indicator).await.is_ok() {
                // Track this reaction so we can remove it later
                let mut reactions = self.status_reactions.lock().await;
                reactions.insert(msg.id.get(), indicator);
            }
            return;
        }

        // Show typing indicator
        let _ = msg.channel_id.start_typing(&ctx.http);

        // Process the message
        if let Err(e) = self.process_message(&ctx, &msg).await {
            error!("Error processing message: {}", e);
            let _ = msg
                .channel_id
                .say(
                    &ctx.http,
                    "Sorry, I encountered an error processing your message.",
                )
                .await;
        }
    }

    async fn reaction_add(&self, ctx: Context, reaction: serenity::model::channel::Reaction) {
        // Skip bot's own reactions
        if let Some(user_id) = reaction.user_id {
            if let Ok(current_user) = ctx.http.get_current_user().await {
                if user_id == current_user.id {
                    return;
                }
            }
        }

        // Log reaction for debugging
        info!(
            "Reaction added: {} on message {} by user {:?}",
            reaction.emoji, reaction.message_id, reaction.user_id
        );

        // Get the original message to see if it was from our bot
        if let Ok(msg) = ctx
            .http
            .get_message(reaction.channel_id, reaction.message_id)
            .await
        {
            info!(
                "Retrieved message for reaction - author: {}, bot check starting",
                msg.author.name
            );

            // Check if the message was from our bot
            if let Ok(current_user) = ctx.http.get_current_user().await {
                info!(
                    "Current bot user: {}, message author: {}",
                    current_user.name, msg.author.name
                );

                if msg.author.id == current_user.id {
                    info!("Reaction is on bot's message - processing");
                    // Someone reacted to our bot's message
                    // Get the user who reacted
                    if let Some(user_id) = reaction.user_id {
                        if let Ok(user) = ctx.http.get_user(user_id).await {
                            // Check if we're currently processing
                            let is_busy = *self.is_processing.lock().await;

                            if is_busy {
                                // Buffer the reaction for later
                                let mut buffer = self.reaction_buffer.lock().await;
                                buffer.push_back(BufferedReaction {
                                    emoji: reaction.emoji.to_string(),
                                    user_name: user.name.clone(),
                                    message_preview: msg
                                        .content
                                        .chars()
                                        .take(100)
                                        .collect::<String>(),
                                    channel_id: reaction.channel_id.get(),
                                    timestamp: std::time::Instant::now(),
                                });

                                // Keep buffer size reasonable
                                if buffer.len() > 20 {
                                    buffer.pop_front();
                                }

                                info!(
                                    "Buffered reaction from {} (currently processing)",
                                    user.name
                                );
                            } else {
                                // Process immediately
                                let notification = format!(
                                    "discord reaction from '{}'\n\
                                    emoji: {}\n\
                                    on your message: {}\n\n\
                                    you may acknowledge this with a reaction (or message) of your own if appropriate.\n\
                                    to react, send a message with just an emoji to channel {} and it will attach to the most recent message",
                                    user.name,
                                    reaction.emoji,
                                    msg.content.chars().take(100).collect::<String>(),
                                    reaction.channel_id.get()
                                );

                                // Route this as a Pattern message to the agents
                                if self.cli_mode {
                                    let mut pattern_msg = PatternMessage::user(notification);
                                    pattern_msg.metadata.custom = serde_json::json!({
                                        "discord_channel_id": reaction.channel_id.get(),
                                        "discord_message_id": reaction.message_id.get(),
                                        "is_reaction": true,
                                    });

                                    // Route through the group
                                    if let (
                                        Some(group),
                                        Some(agents_with_membership),
                                        Some(group_manager),
                                    ) = (
                                        &self.group,
                                        &self.agents_with_membership,
                                        &self.group_manager,
                                    ) {
                                        info!(
                                            "Routing reaction notification through {} group",
                                            group.name
                                        );

                                        // Create a simple task to route the message
                                        let group_clone = group.clone();
                                        let agents_clone = agents_with_membership.clone();
                                        let manager_clone = group_manager.clone();
                                        let pattern_msg_clone = pattern_msg.clone();

                                        // Clone what we need for the async block
                                        let ctx_clone = ctx.clone();
                                        let channel_id = reaction.channel_id;

                                        // Spawn task to handle reaction routing without blocking
                                        tokio::spawn(async move {
                                            match manager_clone
                                                .route_message(
                                                    &group_clone,
                                                    &agents_clone,
                                                    pattern_msg_clone,
                                                )
                                                .await
                                            {
                                                Ok(mut stream) => {
                                                    use futures::StreamExt;
                                                    let mut response_text = String::new();

                                                    while let Some(event) = stream.next().await {
                                                        match event {
                                                        pattern_core::coordination::groups::GroupResponseEvent::TextChunk { text, is_final, .. } => {
                                                            if text.len() > 1 {
                                                                response_text.push_str(&text);

                                                                // Send complete chunks as they arrive
                                                                if is_final || text.ends_with('\n') || response_text.len() > 1000 {
                                                                    if !response_text.trim().is_empty() {
                                                                        // Send the response to Discord
                                                                        if let Err(e) = channel_id.say(&ctx_clone.http, &response_text).await {
                                                                            warn!("Failed to send reaction response to Discord: {}", e);
                                                                        }
                                                                        response_text.clear();
                                                                    }
                                                                }
                                                            }
                                                        }
                                                        pattern_core::coordination::groups::GroupResponseEvent::ToolCallStarted { fn_name, .. } => {
                                                            // Show tool activity for reactions too
                                                            let tool_msg = match fn_name.as_str() {
                                                                "context" => "💭 Processing reaction context...".to_string(),
                                                                "recall" => "🔍 Searching reaction history...".to_string(),
                                                                _ => format!("🔧 Processing with {}...", fn_name)
                                                            };
                                                            if let Err(e) = channel_id.say(&ctx_clone.http, tool_msg).await {
                                                                debug!("Failed to send tool activity: {}", e);
                                                            }
                                                        }
                                                        pattern_core::coordination::groups::GroupResponseEvent::Error { message, .. } => {
                                                            warn!("Error processing reaction: {}", message);
                                                        }
                                                        _ => {}
                                                    }
                                                    }

                                                    // Send any remaining text
                                                    if !response_text.trim().is_empty() {
                                                        if let Err(e) = channel_id
                                                            .say(&ctx_clone.http, &response_text)
                                                            .await
                                                        {
                                                            warn!(
                                                                "Failed to send final reaction response: {}",
                                                                e
                                                            );
                                                        }
                                                    }
                                                }
                                                Err(e) => {
                                                    warn!(
                                                        "Failed to route reaction notification: {}",
                                                        e
                                                    );
                                                }
                                            }
                                        });
                                    } else {
                                        info!("No group configured to handle reactions");
                                    }
                                }
                            }
                        }
                    }
                }
            }
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
    /// Get the elapsed time since we started processing the current message
    pub async fn get_current_processing_time(&self) -> Option<std::time::Duration> {
        let start_time = self.current_message_start.lock().await;
        start_time.as_ref().map(|start| start.elapsed())
    }

    /// Get the current message ID being processed
    pub async fn get_current_message_id(&self) -> Option<u64> {
        let current = self.current_message_id.lock().await;
        *current
    }

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

    /// Process queued messages (without recursion)
    async fn process_message_queue(&self, ctx: &Context) {
        // Wait a bit before processing queue
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;

        // Get ALL queued messages at once
        let queued_messages = {
            let mut queue = self.message_queue.lock().await;

            // Check if we're still processing
            let is_processing = *self.is_processing.lock().await;
            if is_processing {
                // Still busy, don't take from queue yet
                return;
            }

            // Drain all messages from queue
            queue.drain(..).collect::<Vec<_>>()
        };

        if queued_messages.is_empty() {
            return;
        }

        info!(
            "Processing {} queued messages as batch",
            queued_messages.len()
        );

        // Mark as processing
        {
            let mut processing = self.is_processing.lock().await;
            *processing = true;

            // Track the first message ID for replies
            let mut current = self.current_message_id.lock().await;
            *current = Some(queued_messages[0].msg_id);
        }

        // Show typing in the channel
        if let Some(first_msg) = queued_messages.first() {
            let _ = ChannelId::new(first_msg.channel_id).start_typing(&ctx.http);
        }

        // Get channel info for the messages
        let channel_id = queued_messages[0].channel_id;

        // Build concatenated message with special framing
        let mut combined_content =
            String::from("=== Multiple Discord messages arrived while you were busy ===\n\n");

        // Store message IDs for reference
        let mut message_ids = Vec::new();

        for (i, msg) in queued_messages.iter().enumerate() {
            let delay_secs = msg.timestamp.elapsed().as_secs();
            message_ids.push(msg.msg_id);

            // Check if this is already a merged message (contains separators or [Also from...])
            let is_pre_merged = msg.content.contains("---") || msg.content.contains("[Also from");

            if is_pre_merged {
                // This is already a batch, format it differently
                combined_content.push_str(&format!(
                    "[Batch {} - started by '{}' - {}s ago]:\n{}\n\n",
                    i + 1,
                    msg.author_name,
                    delay_secs,
                    msg.content
                ));
            } else {
                // Single message
                combined_content.push_str(&format!(
                    "[Message {} from '{}' - {}s ago]:\n{}\n\n",
                    i + 1,
                    msg.author_name,
                    delay_secs,
                    msg.content
                ));
            }
        }

        // Get channel name for context
        let channel_name = if let Ok(channel) = ChannelId::new(channel_id).to_channel(&ctx).await {
            match channel {
                serenity::model::channel::Channel::Guild(gc) => format!("#{}", gc.name),
                _ => format!("channel {}", channel_id),
            }
        } else {
            format!("channel {}", channel_id)
        };

        combined_content.push_str(&format!(
            "You can respond to these messages as a batch. Use send_message with target_type: \"channel\" \
            and target_id: \"{}\" (or the channel name {}) to reply. Since these messages are delayed, your response will be sent as a reply to the last message.\n\n",
            channel_id,
            channel_name
        ));

        // Create Pattern message
        let mut pattern_msg = PatternMessage::user(combined_content);
        // Use the last message ID for replies (most recent message to reply to)
        let last_msg_id = queued_messages
            .last()
            .map(|m| m.msg_id)
            .unwrap_or(queued_messages[0].msg_id);
        pattern_msg.metadata.custom = serde_json::json!({
            "discord_channel_id": channel_id,
            "discord_message_id": last_msg_id,  // Reply to the last message in batch
            "is_batch": true,
            "batch_size": queued_messages.len(),
            "response_delay_ms": queued_messages[0].timestamp.elapsed().as_millis(),
        });

        // Route through agents
        if self.cli_mode {
            if let (Some(group), Some(agents_with_membership), Some(group_manager)) = (
                &self.group,
                &self.agents_with_membership,
                &self.group_manager,
            ) {
                match group_manager
                    .route_message(group, agents_with_membership, pattern_msg)
                    .await
                {
                    Ok(mut stream) => {
                        use futures::StreamExt;
                        let mut response = String::new();
                        let mut has_response = false;

                        while let Some(event) = stream.next().await {
                            match event {
                                pattern_core::coordination::groups::GroupResponseEvent::TextChunk { text, is_final, .. } => {
                                    if !text.is_empty() && text.trim() != "." {
                                        response.push_str(&text);
                                        has_response = true;

                                        // Send complete chunks
                                        if is_final || text.ends_with('\n') || response.len() > 1500 {
                                            if !response.trim().is_empty() {
                                                for chunk in split_message(&response, 2000) {
                                                    if let Err(e) = ChannelId::new(channel_id).say(&ctx.http, chunk).await {
                                                        warn!("Failed to send batch response: {}", e);
                                                    }
                                                }
                                                response.clear();
                                            }
                                        }
                                    }
                                }
                                _ => {} // Ignore other events for batch processing
                            }
                        }

                        // Send any remaining response
                        if !response.trim().is_empty() {
                            for chunk in split_message(&response, 2000) {
                                if let Err(e) =
                                    ChannelId::new(channel_id).say(&ctx.http, chunk).await
                                {
                                    warn!("Failed to send final batch response: {}", e);
                                }
                            }
                        }

                        if !has_response {
                            // No response to batch, send indicator
                            let _ = ChannelId::new(channel_id).say(&ctx.http, "💭 ...").await;
                        }
                    }
                    Err(e) => {
                        error!("Failed to process message batch: {}", e);
                        let _ = ChannelId::new(channel_id).say(&ctx.http, "💭 ...").await;
                    }
                }
            }
        }

        // Mark as done
        {
            let mut processing = self.is_processing.lock().await;
            *processing = false;

            let mut current = self.current_message_id.lock().await;
            *current = None;
        }

        // Remove status reactions from processed messages
        {
            let mut reactions = self.status_reactions.lock().await;
            for msg_id in &message_ids {
                if let Some(emoji) = reactions.remove(msg_id) {
                    // Try to remove the reaction
                    let reaction_type = serenity::all::ReactionType::Unicode(emoji.to_string());
                    if let Ok(current_user) = ctx.http.get_current_user().await {
                        let _ = ctx
                            .http
                            .delete_reaction(
                                ChannelId::new(channel_id),
                                serenity::all::MessageId::new(*msg_id),
                                current_user.id,
                                &reaction_type,
                            )
                            .await;
                    }
                }
            }
        }

        // Process any buffered reactions
        self.flush_reaction_buffer(ctx).await;
    }

    /// Flush buffered reactions as a batch
    async fn flush_reaction_buffer(&self, _ctx: &Context) {
        let reactions = {
            let mut buffer = self.reaction_buffer.lock().await;
            if buffer.is_empty() {
                return;
            }
            buffer.drain(..).collect::<Vec<_>>()
        };

        if reactions.is_empty() {
            return;
        }

        // Format batch notification with more context
        let mut notification = String::from("=== Batched Discord Reactions ===\n\n");
        for reaction in &reactions {
            let age_secs = reaction.timestamp.elapsed().as_secs();
            notification.push_str(&format!(
                "• {} from {} ({}s ago)\n  On message: {}\n\n",
                reaction.emoji,
                reaction.user_name,
                age_secs,
                if reaction.message_preview.len() > 50 {
                    format!("{}...", &reaction.message_preview[..50])
                } else {
                    reaction.message_preview.clone()
                }
            ));
        }

        notification.push_str("These reactions arrived while you were processing. You may acknowledge if appropriate.");

        // Send batch to agents
        if self.cli_mode {
            if let (Some(group), Some(agents_with_membership), Some(group_manager)) = (
                &self.group,
                &self.agents_with_membership,
                &self.group_manager,
            ) {
                let pattern_msg = PatternMessage::user(notification);

                // Fire and forget - don't wait for response
                let group_clone = group.clone();
                let agents_clone = agents_with_membership.clone();
                let manager_clone = group_manager.clone();

                tokio::spawn(async move {
                    let _ = manager_clone
                        .route_message(&group_clone, &agents_clone, pattern_msg)
                        .await;
                });

                info!("Flushed {} buffered reactions to agents", reactions.len());
            }
        }
    }

    /// Process a Discord message and route it to Pattern agents
    async fn process_message(&self, ctx: &Context, msg: &Message) -> Result<(), String> {
        // Debounce rapid messages - wait a bit if last message was very recent
        {
            let mut last_time = self.last_message_time.lock().await;
            let now = std::time::Instant::now();
            let time_since_last = now.duration_since(*last_time);

            // If less than 500ms since last message, wait a bit
            if time_since_last < std::time::Duration::from_millis(500) {
                let wait_time = std::time::Duration::from_millis(500) - time_since_last;
                info!("Debouncing message - waiting {}ms", wait_time.as_millis());
                tokio::time::sleep(wait_time).await;
            }

            *last_time = std::time::Instant::now();
        }

        // Mark as processing and track current message and timing
        {
            let mut processing = self.is_processing.lock().await;
            *processing = true;

            let mut current = self.current_message_id.lock().await;
            *current = Some(msg.id.get());

            let mut start_time = self.current_message_start.lock().await;
            *start_time = Some(std::time::Instant::now());

            info!("Processing message {} from {}", msg.id, msg.author.name);
        }

        // Start typing indicator that refreshes every 8 seconds
        {
            let mut typing_handle = self.typing_handle.lock().await;

            // Cancel any existing typing task
            if let Some(handle) = typing_handle.take() {
                handle.abort();
            }

            let channel_id = msg.channel_id;
            let http = ctx.http.clone();

            // Spawn task to keep typing indicator alive
            let handle = tokio::spawn(async move {
                loop {
                    // Send typing indicator
                    let _ = channel_id.start_typing(&http);

                    // Wait 8 seconds (typing lasts 10 seconds, so refresh at 8)
                    tokio::time::sleep(std::time::Duration::from_secs(8)).await;
                }
            });

            *typing_handle = Some(handle);
        }

        // Ensure we mark as not processing when done
        let result = self.process_message_inner(ctx, msg).await;

        // Mark as done and clear current message and timing
        {
            let mut processing = self.is_processing.lock().await;
            *processing = false;

            let mut current = self.current_message_id.lock().await;
            *current = None;

            let mut start_time = self.current_message_start.lock().await;
            *start_time = None;

            // Stop typing indicator
            let mut typing_handle = self.typing_handle.lock().await;
            if let Some(handle) = typing_handle.take() {
                handle.abort();
            }
        }

        // Process any buffered reactions
        self.flush_reaction_buffer(ctx).await;

        // Process any queued messages
        self.process_message_queue(ctx).await;

        result
    }

    /// Inner message processing logic
    async fn process_message_inner(&self, ctx: &Context, msg: &Message) -> Result<(), String> {
        // Track when we started processing for delay calculation
        let processing_start = std::time::Instant::now();

        if self.cli_mode {
            // Create Pattern message with Discord metadata
            let discord_channel_id = msg.channel_id.get();

            // Resolve mentions to usernames
            let mut resolved_content = msg.content.clone();
            for user in &msg.mentions {
                let mention_pattern = format!("<@{}>", user.id);
                let alt_mention_pattern = format!("<@!{}>", user.id); // Nickname mentions
                resolved_content = resolved_content
                    .replace(&mention_pattern, &format!("@{}", user.name))
                    .replace(&alt_mention_pattern, &format!("@{}", user.name));
            }

            // Get current bot user for self-mentions
            if let Ok(current_user) = ctx.http.get_current_user().await {
                let bot_mention = format!("<@{}>", current_user.id);
                let bot_alt_mention = format!("<@!{}>", current_user.id);
                resolved_content = resolved_content
                    .replace(&bot_mention, "@Pattern")
                    .replace(&bot_alt_mention, "@Pattern");
            }

            // Get channel name if possible (moved outside to be accessible)
            let channel_name = if let Ok(channel) = msg.channel_id.to_channel(&ctx).await {
                match channel {
                    serenity::model::channel::Channel::Guild(gc) => format!("#{}", gc.name),
                    _ => format!("channel {}", msg.channel_id),
                }
            } else {
                format!("channel {}", msg.channel_id)
            };

            // Get display name hierarchy: server nickname > global display name > username
            // Always show username in brackets for clarity
            let display_name_with_username = if let Some(guild_id) = msg.guild_id {
                // Try to get member to access server nickname
                match ctx.http.get_member(guild_id, msg.author.id).await {
                    Ok(member) => {
                        if let Some(nick) = member.nick {
                            // Server nickname available
                            format!("{} [{}]", nick, msg.author.name)
                        } else if let Some(ref global_name) = msg.author.global_name {
                            // No nickname, use global display name
                            format!("{} [{}]", global_name, msg.author.name)
                        } else {
                            // Just username
                            msg.author.name.clone()
                        }
                    }
                    Err(e) => {
                        debug!("Failed to get member for nickname: {}", e);
                        // Fall back to global display name or username
                        if let Some(ref global_name) = msg.author.global_name {
                            format!("{} [{}]", global_name, msg.author.name)
                        } else {
                            msg.author.name.clone()
                        }
                    }
                }
            } else {
                // DM - use global display name if available, always show username
                if let Some(ref global_name) = msg.author.global_name {
                    format!("{} [{}]", global_name, msg.author.name)
                } else {
                    msg.author.name.clone()
                }
            };

            // Build context string with better framing
            let discord_context = if msg.guild_id.is_none() {
                format!(
                    "Direct message from Discord user '{}'",
                    display_name_with_username
                )
            } else {
                format!(
                    "Message from '{}' in Discord {}",
                    display_name_with_username, channel_name
                )
            };

            // Create framing prompt that makes responding optional
            let framed_message = format!(
                "{}\n\
                Message: {}\n\n\
                you can respond if you have something to add, or if you're directly mentioned.
                if you do, use send_message with target_type: \"channel\" and target_id: \"{}\" (or the channel name {}) or direct text

                ",
                discord_context,
                resolved_content,
                discord_channel_id,
                channel_name
            );

            let mut pattern_msg = PatternMessage::user(framed_message);

            // Add Discord context to metadata so send_message knows where to reply
            pattern_msg.metadata.custom = serde_json::json!({
                "discord_channel_id": msg.channel_id.get(),
                "discord_guild_id": msg.guild_id.map(|g| g.get()),
                "discord_user_id": msg.author.id.get(),
                "discord_username": msg.author.name.clone(),
                "discord_message_id": msg.id.get(),  // Track the original message for replies
                "is_dm": msg.guild_id.is_none(),
                "processing_start_ms": processing_start.elapsed().as_millis(),  // Track when we started
            });

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
                let idle_timeout = Duration::from_secs(600); // 10 minutes of inactivity
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
                            has_sent_initial_response = true; // ANY activity counts as a response

                            match event {
                                pattern_core::coordination::groups::GroupResponseEvent::TextChunk { agent_id: _, text, is_final } => {
                                    // Filter out single '.' which is a null response
                                    if !text.is_empty() && text.trim() != "." {
                                        current_message.push_str(&text);

                                        // Send complete sentences/paragraphs as they arrive
                                        if is_final || text.ends_with('\n') || text.ends_with(". ") || current_message.len() > 1500 {
                                            if !current_message.trim().is_empty() && current_message.trim() != "." {
                                                for chunk in split_message(&current_message, 2000) {
                                                    // Use channel.say() to respond in the same channel instead of DM
                                                    if let Err(e) = msg.channel_id.say(&ctx.http, chunk).await {
                                                        warn!("Failed to send response chunk to channel {}: {}", msg.channel_id, e);
                                                        // Log more details about the context
                                                        warn!("Message context - Guild: {:?}, Channel: {}, Is DM: {}",
                                                            msg.guild_id, msg.channel_id, msg.guild_id.is_none());
                                                    }
                                                    has_sent_initial_response = true;
                                                }
                                                current_message.clear();
                                            }
                                        }
                                    }
                                },
                                pattern_core::coordination::groups::GroupResponseEvent::ToolCallStarted { agent_id: _, call_id, fn_name, args: _ } => {
                                    info!("Tool call started: {} ({})", fn_name, call_id);

                                    // Don't intercept send_message tool calls - let them go through the agent's router
                                    // This ensures proper routing based on the target specified in the tool call
                                    if fn_name != "send_message" {
                                        // Show tool activity if we haven't sent anything yet
                                        let tool_msg = match fn_name.as_str() {
                                            "context" => "💭 Agent is accessing memory...".to_string(),
                                            "recall" => "🔍 Agent is searching memories...".to_string(),
                                            "search" => "🔎 Agent is searching...".to_string(),
                                            _ => format!("🔧 Agent is using {}", fn_name)
                                        };
                                        // Use channel.say() to respond in the same channel instead of DM
                                        if let Err(e) = msg.channel_id.say(&ctx.http, tool_msg).await {
                                            debug!("Failed to send tool activity: {}", e);
                                        }
                                        has_sent_initial_response = true;
                                    }
                                },
                                pattern_core::coordination::groups::GroupResponseEvent::ToolCallCompleted { agent_id: _, call_id, result } => {
                                    info!("Tool call completed: {} - {:?}", call_id, result);

                                    // Check if this was a send_message tool that succeeded
                                    if let Ok(result_str) = &result {
                                        if result_str.contains("Message sent successfully") || result_str.contains("channel:") {
                                            info!("send_message tool completed successfully");
                                            has_sent_initial_response = true; // Mark as having responded
                                        }
                                    }
                                },
                                pattern_core::coordination::groups::GroupResponseEvent::Error { agent_id: _, message, recoverable } => {
                                    warn!("Agent error: {} (recoverable: {})", message, recoverable);
                                    // Don't send error details to Discord, just log them
                                    if !recoverable {
                                        // For non-recoverable errors, maybe send a generic message
                                        let _ = msg.channel_id.say(&ctx.http, "💭 ... !").await;
                                        break; // Stop processing on non-recoverable errors
                                    }
                                    // For recoverable errors, just continue silently
                                },
                                pattern_core::coordination::groups::GroupResponseEvent::AgentStarted { agent_name, .. } => {
                                    debug!("Agent {} started processing", agent_name);
                                    active_agents += 1;

                                    // Start typing indicator when agent starts thinking
                                    let _ = msg.channel_id.start_typing(&ctx.http);
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
                                "⏱️ (No further activity for 10 minutes - entities may still be processing)"
                            } else {
                                "⏱️ Request timed out after 10 minutes of inactivity. No entities responded."
                            };
                            if let Err(e) = msg.channel_id.say(&ctx.http, timeout_msg).await {
                                warn!("Failed to send timeout message: {}", e);
                            }
                            break;
                        }
                    }
                }

                // Send any remaining buffered content
                if !current_message.trim().is_empty() {
                    for chunk in split_message(&current_message, 2000) {
                        if let Err(e) = msg.channel_id.say(&ctx.http, chunk).await {
                            warn!("Failed to send final response chunk: {}", e);
                        }
                    }
                }

                // If we never sent anything, send a status message
                if !has_sent_initial_response {
                    let status_msg = if active_agents > 0 {
                        "The entities started processing but produced no response."
                    } else {
                        "No entities were available to process your message."
                    };
                    if let Err(e) = msg.channel_id.say(&ctx.http, status_msg).await {
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
                            response = "No response from entity.".to_string();
                        }

                        msg.channel_id
                            .say(&ctx.http, response)
                            .await
                            .map_err(|e| format!("Failed to send reply: {}", e))?;
                    }
                }
            }
        } else {
            // TODO: Implement full database mode with user lookup
            msg.channel_id
                .say(&ctx.http, "Full mode not yet implemented")
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
