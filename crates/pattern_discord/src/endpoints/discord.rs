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
            default_channel: None,
            default_dm_user: None,
        }
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

    /// Send a message to a specific Discord channel
    async fn send_to_channel(&self, channel_id: ChannelId, content: String) -> Result<()> {
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
            // Check for user_id to send DM
            if let Some(user_id) = meta.get("discord_user_id").and_then(|v| v.as_u64()) {
                self.send_dm(DiscordUserId::new(user_id), content).await?;
                return Ok(Some(format!("dm:{}", user_id)));
            }

            // Check for channel_id to send to specific channel
            if let Some(channel_id) = meta.get("discord_channel_id").and_then(|v| v.as_u64()) {
                self.send_to_channel(ChannelId::new(channel_id), content)
                    .await?;
                return Ok(Some(format!("channel:{}", channel_id)));
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
