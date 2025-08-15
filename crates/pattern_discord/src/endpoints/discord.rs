use crate::bot::DiscordBot;
use serde_json::Value;
use serenity::http::Http;
use serenity::model::id::{ChannelId, UserId as DiscordUserId};
use std::sync::Arc;
use tracing::{debug, info, warn};

use pattern_core::Result;
use pattern_core::context::message_router::{MessageEndpoint, MessageOrigin};
use pattern_core::message::{ContentPart, Message, MessageContent};

/// Discord endpoint for sending messages through the Pattern message router
#[derive(Clone)]
pub struct DiscordEndpoint {
    /// Serenity HTTP client for Discord API
    http: Arc<Http>,
    /// Reference to the Discord bot for context
    bot: Option<Arc<DiscordBot>>,
    /// Optional default channel for broadcasts
    default_channel: Option<ChannelId>,
    /// Optional default DM user for CLI mode
    default_dm_user: Option<DiscordUserId>,
}

impl DiscordEndpoint {
    /// Create a new Discord endpoint with the bot token
    pub fn new(token: String) -> Self {
        let http = Arc::new(Http::new(&token));
        Self {
            http,
            bot: None,
            default_channel: None,
            default_dm_user: None,
        }
    }

    /// Set the bot reference for context access
    pub fn with_bot(mut self, bot: Arc<DiscordBot>) -> Self {
        self.bot = Some(bot);
        self
    }

    /// Try to resolve a channel name to a channel ID
    /// Supports formats: "#channel-name", "channel-name", or numeric ID
    async fn resolve_channel_id(&self, target_id: &str) -> Option<ChannelId> {
        // Strip leading # if present
        let channel_ref = target_id.trim_start_matches('#');

        // First, try parsing as a numeric ID
        if let Ok(id) = channel_ref.parse::<u64>() {
            return Some(ChannelId::new(id));
        }

        // If we have a guild context, we could search for channel by name
        // For now, we'll just log that we couldn't resolve it
        debug!(
            "Could not resolve channel name '{}' to ID - using numeric IDs only for now",
            channel_ref
        );
        None
    }

    /// Set a default channel for messages without specific targets
    pub fn with_default_channel(mut self, channel_id: u64) -> Self {
        self.default_channel = Some(ChannelId::new(channel_id));
        self
    }

    /// Set a default DM user for messages without specific targets
    pub fn with_default_dm_user(mut self, user_id: u64) -> Self {
        self.default_dm_user = Some(DiscordUserId::new(user_id));
        self
    }

    /// Extract text content from a Pattern message
    fn extract_text(message: &Message) -> String {
        match &message.content {
            MessageContent::Text(text) => text.clone(),
            MessageContent::Parts(parts) => parts
                .iter()
                .filter_map(|part| match part {
                    ContentPart::Text(text) => Some(text.as_str()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("\n"),
            _ => "[Non-text content]".to_string(),
        }
    }

    /// Check if a string is likely a Discord reaction
    fn is_discord_reaction(text: &str) -> bool {
        let trimmed = text.trim();

        // Log what we're checking
        debug!(
            "Checking if '{}' (len {}, chars {}) is a reaction",
            trimmed,
            trimmed.len(),
            trimmed.chars().count()
        );

        // Check for standard Discord emoji format :name:
        if trimmed.starts_with(':') && trimmed.ends_with(':') && trimmed.len() > 2 {
            debug!("Detected :name: format emoji");
            return true;
        }

        // Check for custom emoji format <:name:id> or <a:name:id>
        if (trimmed.starts_with("<:") || trimmed.starts_with("<a:")) && trimmed.ends_with('>') {
            debug!("Detected custom emoji format");
            return true;
        }

        // Check for unicode emoji (single character or with variation selectors)
        // Allow up to 4 chars for variation selectors and zero-width joiners
        if trimmed.chars().count() <= 4 {
            for ch in trimmed.chars() {
                // Basic emoji ranges
                if (ch >= '\u{1F300}' && ch <= '\u{1F9FF}') || // Misc symbols & pictographs
                   (ch >= '\u{2600}' && ch <= '\u{26FF}') ||   // Misc symbols
                   (ch >= '\u{2700}' && ch <= '\u{27BF}') ||   // Dingbats
                   (ch >= '\u{1F600}' && ch <= '\u{1F64F}') ||  // Emoticons
                   (ch >= '\u{1F900}' && ch <= '\u{1F9FF}') ||  // Supplemental symbols
                   (ch >= '\u{2000}' && ch <= '\u{206F}')
                // General punctuation (includes some emoji)
                {
                    debug!("Detected unicode emoji");
                    return true;
                }
            }
        }

        debug!("Not detected as emoji");
        false
    }

    /// Parse a Discord emoji string into a ReactionType
    fn parse_discord_emoji(emoji_str: &str) -> serenity::model::channel::ReactionType {
        let trimmed = emoji_str.trim();

        // Check for custom emoji format :name:id or <:name:id>
        if trimmed.starts_with("<:") && trimmed.ends_with('>') {
            // Parse custom emoji <:name:id>
            let inner = &trimmed[2..trimmed.len() - 1];
            if let Some(colon_pos) = inner.rfind(':') {
                if let Ok(id) = inner[colon_pos + 1..].parse::<u64>() {
                    let name = inner[..colon_pos].to_string();
                    return serenity::model::channel::ReactionType::Custom {
                        animated: false,
                        id: serenity::model::id::EmojiId::new(id),
                        name: Some(name),
                    };
                }
            }
        }

        // Check for animated custom emoji <a:name:id>
        if trimmed.starts_with("<a:") && trimmed.ends_with('>') {
            let inner = &trimmed[3..trimmed.len() - 1];
            if let Some(colon_pos) = inner.rfind(':') {
                if let Ok(id) = inner[colon_pos + 1..].parse::<u64>() {
                    let name = inner[..colon_pos].to_string();
                    return serenity::model::channel::ReactionType::Custom {
                        animated: true,
                        id: serenity::model::id::EmojiId::new(id),
                        name: Some(name),
                    };
                }
            }
        }

        // Check for simple :name: format (needs special handling)
        if trimmed.starts_with(':') && trimmed.ends_with(':') && trimmed.len() > 2 {
            // Keep the colons for Discord to interpret
            // Discord will handle converting :thumbsup: to 👍 or finding the custom emoji
            return serenity::model::channel::ReactionType::Unicode(trimmed.to_string());
        }

        // Otherwise treat as unicode emoji
        serenity::model::channel::ReactionType::Unicode(trimmed.to_string())
    }

    /// Send a message to a specific Discord channel
    async fn send_to_channel(&self, channel_id: ChannelId, content: String) -> Result<()> {
        info!(
            "send_to_channel called with content: '{}', is_reaction: {}",
            content,
            Self::is_discord_reaction(&content)
        );

        // Check if this is just an emoji - if so, try to add it as a reaction to the last message
        if Self::is_discord_reaction(&content) {
            // Try to get the last message in the channel (excluding our own)
            match channel_id
                .messages(&self.http, serenity::builder::GetMessages::new().limit(10))
                .await
            {
                Ok(messages) => {
                    // Find the first message that's not from us
                    if let Ok(current_user) = self.http.get_current_user().await {
                        for msg in messages {
                            if msg.author.id != current_user.id {
                                // For :name: format, try to find the custom emoji in the guild
                                let mut reaction_type = Self::parse_discord_emoji(&content);

                                // If it's :name: format, try to resolve it to a custom emoji
                                if content.trim().starts_with(':') && content.trim().ends_with(':')
                                {
                                    let emoji_name = content
                                        .trim()
                                        .trim_start_matches(':')
                                        .trim_end_matches(':');

                                    // Get guild ID from the message
                                    if let Some(guild_id) = msg.guild_id {
                                        // Try to get guild emojis
                                        if let Ok(emojis) = self.http.get_emojis(guild_id).await {
                                            // Find emoji by name
                                            if let Some(emoji) =
                                                emojis.iter().find(|e| e.name == emoji_name)
                                            {
                                                info!(
                                                    "Found custom emoji {} with ID {}",
                                                    emoji_name, emoji.id
                                                );
                                                reaction_type = serenity::model::channel::ReactionType::Custom {
                                                    animated: emoji.animated,
                                                    id: emoji.id,
                                                    name: Some(emoji.name.clone()),
                                                };
                                            } else {
                                                debug!(
                                                    "Custom emoji '{}' not found in guild",
                                                    emoji_name
                                                );
                                            }
                                        }
                                    }
                                }

                                info!(
                                    "Adding reaction {:?} to message {} in channel {}",
                                    reaction_type, msg.id, channel_id
                                );

                                // Try to add the reaction
                                match msg.react(&self.http, reaction_type).await {
                                    Ok(_) => {
                                        info!("Successfully added reaction");
                                        return Ok(());
                                    }
                                    Err(e) => {
                                        warn!("Failed to add reaction: {}", e);
                                        // Continue to try as regular message
                                        break;
                                    }
                                }
                            }
                        }
                    }
                }
                Err(e) => {
                    debug!("Couldn't fetch messages to react to: {}", e);
                }
            }
        }

        // Fall back to sending as regular message
        channel_id.say(&self.http, &content).await.map_err(|e| {
            pattern_core::CoreError::ToolExecutionFailed {
                tool_name: "discord_endpoint".to_string(),
                cause: format!("Failed to send message to channel: {}", e),
                parameters: serde_json::json!({
                    "channel_id": channel_id.get(),
                    "content_length": content.len()
                }),
            }
        })?;

        info!("Sent message to Discord channel {}", channel_id);
        Ok(())
    }

    /// Send a DM to a specific Discord user
    async fn send_dm(&self, user_id: DiscordUserId, content: String) -> Result<()> {
        // Create a DM channel with the user
        let dm_channel = user_id.create_dm_channel(&self.http).await.map_err(|e| {
            pattern_core::CoreError::ToolExecutionFailed {
                tool_name: "discord_endpoint".to_string(),
                cause: format!("Failed to create DM channel: {}", e),
                parameters: serde_json::json!({
                    "user_id": user_id.get(),
                }),
            }
        })?;

        // Send the message
        dm_channel.say(&self.http, &content).await.map_err(|e| {
            pattern_core::CoreError::ToolExecutionFailed {
                tool_name: "discord_endpoint".to_string(),
                cause: format!("Failed to send DM: {}", e),
                parameters: serde_json::json!({
                    "user_id": user_id.get(),
                    "content_length": content.len()
                }),
            }
        })?;

        info!("Sent DM to Discord user {}", user_id);
        Ok(())
    }
}

#[async_trait::async_trait]
impl MessageEndpoint for DiscordEndpoint {
    async fn send(
        &self,
        message: Message,
        metadata: Option<Value>,
        origin: Option<&MessageOrigin>,
    ) -> Result<Option<String>> {
        let content = Self::extract_text(&message);

        // Check metadata for routing information
        if let Some(ref meta) = metadata {
            // Check if we should reply to a specific message (for delayed responses)
            let reply_to_id = meta
                .get("discord_message_id")
                .or_else(|| meta.get("custom").and_then(|v| v.get("discord_message_id")))
                .and_then(|v| v.as_u64());

            // First check for explicit channel_id (highest priority)
            if let Some(channel_id) = meta.get("discord_channel_id").and_then(|v| v.as_u64()) {
                let channel = ChannelId::new(channel_id);

                // If we have a message to reply to and response is delayed, use reply
                if let Some(msg_id) = reply_to_id {
                    // Check if this is a delayed response
                    // First check metadata (for batched messages)
                    let mut should_reply = meta
                        .get("response_delay_ms")
                        .and_then(|v| v.as_u64())
                        .map(|delay| delay > 30000)
                        .unwrap_or(false);

                    // If not in metadata, check bot's current processing time
                    if !should_reply {
                        if let Some(ref bot) = self.bot {
                            if let Some(duration) = bot.get_current_processing_time().await {
                                let elapsed = duration.as_millis() as u64;
                                should_reply = elapsed > 30000;
                                if should_reply {
                                    debug!(
                                        "Using reply threading: message processing took {}ms",
                                        elapsed
                                    );
                                }
                            }
                        }
                    }

                    if should_reply {
                        // Use reply for delayed responses
                        if let Ok(original_msg) = self
                            .http
                            .get_message(channel, serenity::model::id::MessageId::new(msg_id))
                            .await
                        {
                            if let Err(e) = original_msg.reply(&self.http, &content).await {
                                warn!(
                                    "Failed to reply to message: {}, falling back to channel send",
                                    e
                                );
                                self.send_to_channel(channel, content).await?;
                            } else {
                                info!("Replied to message {} in channel {}", msg_id, channel_id);
                                return Ok(Some(format!("reply:{}:{}", channel_id, msg_id)));
                            }
                        } else {
                            // Can't find original message, just send to channel
                            self.send_to_channel(channel, content).await?;
                        }
                    } else {
                        self.send_to_channel(channel, content).await?;
                    }
                } else {
                    self.send_to_channel(channel, content).await?;
                }
                return Ok(Some(format!("channel:{}", channel_id)));
            }

            // Check if target_id contains a channel name to resolve
            if let Some(target_id) = meta.get("target_id").and_then(|v| v.as_str()) {
                if let Some(channel_id) = self.resolve_channel_id(target_id).await {
                    self.send_to_channel(channel_id, content).await?;
                    return Ok(Some(format!("channel:{}", channel_id)));
                }
            }

            // Then check custom metadata (from incoming discord message)
            if let Some(custom) = meta.get("custom").and_then(|v| v.as_object()) {
                if let Some(channel_id) = custom.get("discord_channel_id").and_then(|v| v.as_u64())
                {
                    self.send_to_channel(ChannelId::new(channel_id), content)
                        .await?;
                    return Ok(Some(format!("channel:{}", channel_id)));
                }
            }

            // Finally check for user_id to send DM (lowest priority)
            if let Some(user_id) = meta.get("discord_user_id").and_then(|v| v.as_u64()) {
                self.send_dm(DiscordUserId::new(user_id), content).await?;
                return Ok(Some(format!("dm:{}", user_id)));
            }

            // Check for reply context
            if let Some(reply_to) = meta.get("reply_to_message_id").and_then(|v| v.as_u64()) {
                debug!(
                    "Reply context present but not implemented yet: {}",
                    reply_to
                );
            }
        }

        // Check if origin provides Discord context
        if let Some(MessageOrigin::Discord {
            channel_id,
            user_id,
            ..
        }) = origin
        {
            // Prefer channel if both are present (came from a channel message)
            if let Ok(chan_id) = channel_id.parse::<u64>() {
                self.send_to_channel(ChannelId::new(chan_id), content)
                    .await?;
                return Ok(Some(format!("channel:{}", chan_id)));
            } else if let Ok(usr_id) = user_id.parse::<u64>() {
                self.send_dm(DiscordUserId::new(usr_id), content).await?;
                return Ok(Some(format!("dm:{}", usr_id)));
            }
        }

        // Fall back to default DM user if configured
        if let Some(user) = self.default_dm_user {
            self.send_dm(user, content).await?;
            return Ok(Some(format!("default_dm:{}", user)));
        }

        // Fall back to default channel if configured
        if let Some(channel) = self.default_channel {
            self.send_to_channel(channel, content).await?;
            return Ok(Some(format!("default_channel:{}", channel)));
        }

        warn!("No Discord destination found in metadata or origin");
        Err(pattern_core::CoreError::ToolExecutionFailed {
            tool_name: "discord_endpoint".to_string(),
            cause: "No Discord destination specified".to_string(),
            parameters: serde_json::json!({
                "has_metadata": metadata.is_some(),
                "has_origin": origin.is_some(),
                "has_default_dm": self.default_dm_user.is_some(),
                "has_default_channel": self.default_channel.is_some(),
            }),
        })
    }

    fn endpoint_type(&self) -> &'static str {
        "discord"
    }
}
