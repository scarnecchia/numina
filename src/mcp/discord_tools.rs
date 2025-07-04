use rmcp::{
    handler::server::tool::Parameters,
    model::{CallToolResult, Content},
    Error as McpError,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::sync::Arc;
use tracing::{error, info};

// Discord-specific request structs
#[derive(Debug, Deserialize, Serialize, schemars::JsonSchema)]
pub struct SendDiscordMessageRequest {
    /// Channel ID or "guild/channel" format (e.g. "MyServer/general")
    pub channel: String,
    pub message: String,
}

#[derive(Debug, Deserialize, Serialize, schemars::JsonSchema)]
pub struct SendDiscordEmbedRequest {
    /// Channel ID or "guild/channel" format (e.g. "MyServer/general")
    pub channel: String,
    pub title: String,
    pub description: String,
    pub color: Option<u32>,
    pub fields: Option<Vec<EmbedField>>,
}

#[derive(Debug, Deserialize, Serialize, schemars::JsonSchema)]
pub struct EmbedField {
    pub name: String,
    pub value: String,
    pub inline: Option<bool>,
}

#[derive(Debug, Deserialize, Serialize, schemars::JsonSchema)]
pub struct GetDiscordChannelInfoRequest {
    /// Channel ID or "guild/channel" format (e.g. "MyServer/general")
    pub channel: String,
}

#[derive(Debug, Deserialize, Serialize, schemars::JsonSchema)]
pub struct SendDiscordDmRequest {
    pub user_id: u64,
    pub message: String,
}

/// Core MCP tools handler
#[derive(Clone)]
pub struct DiscordTools {
    #[allow(dead_code)] // Used by Discord MCP tools
    pub discord_client: Option<Arc<tokio::sync::RwLock<serenity::Client>>>,
}

// Core MCP tool implementations
impl DiscordTools {
    pub async fn send_discord_embed(
        &self,
        params: Parameters<SendDiscordEmbedRequest>,
    ) -> Result<CallToolResult, McpError> {
        use serenity::all::{CreateEmbed, CreateMessage};

        let params = params.0;
        info!(channel = %params.channel, title = %params.title, "Sending Discord embed via MCP");

        // Resolve channel
        let channel_id = self.resolve_channel(&params.channel).await?;

        let client = self.discord_client.clone().ok_or_else(|| {
            McpError::internal_error(
                "Discord client not available",
                Some(json!({
                    "error_type": "discord_not_configured",
                    "details": "Discord bot is not running or not configured"
                })),
            )
        })?;

        let ctx = client.read().await.http.clone();

        let mut embed = CreateEmbed::new()
            .title(&params.title)
            .description(&params.description);

        if let Some(color) = params.color {
            embed = embed.color(color);
        }

        if let Some(fields) = params.fields {
            for field in fields {
                embed = embed.field(&field.name, &field.value, field.inline.unwrap_or(false));
            }
        }

        let message = CreateMessage::new().embed(embed);

        match channel_id.send_message(&ctx, message).await {
            Ok(message) => {
                info!("Discord embed sent successfully");
                Ok(CallToolResult::success(vec![Content::text(
                    serde_json::to_string_pretty(&json!({
                        "success": true,
                        "message_id": message.id.to_string(),
                        "channel_id": channel_id.to_string(),
                        "channel": params.channel,
                        "timestamp": message.timestamp.to_string()
                    }))
                    .unwrap(),
                )]))
            }
            Err(e) => {
                error!("Failed to send Discord embed: {}", e);
                Err(McpError::internal_error(
                    "Failed to send Discord embed",
                    Some(json!({
                        "channel": params.channel,
                        "channel_id": channel_id.to_string(),
                        "error_type": "discord_embed_failed",
                        "details": e.to_string()
                    })),
                ))
            }
        }
    }

    pub async fn get_discord_channel_info(
        &self,
        params: Parameters<GetDiscordChannelInfoRequest>,
    ) -> Result<CallToolResult, McpError> {
        let params = params.0;
        info!(channel = %params.channel, "Getting Discord channel info via MCP");

        // Resolve channel
        let channel_id = self.resolve_channel(&params.channel).await?;

        let client = self.discord_client.clone().ok_or_else(|| {
            McpError::internal_error(
                "Discord client not available",
                Some(json!({
                    "error_type": "discord_not_configured",
                    "details": "Discord bot is not running or not configured"
                })),
            )
        })?;

        let ctx = client.read().await.http.clone();

        match channel_id.to_channel(&ctx).await {
            Ok(channel) => {
                let info = match channel {
                    serenity::all::Channel::Guild(guild_channel) => {
                        json!({
                            "type": "guild",
                            "id": guild_channel.id.to_string(),
                            "name": guild_channel.name,
                            "guild_id": guild_channel.guild_id.to_string(),
                            "kind": format!("{:?}", guild_channel.kind),
                            "topic": guild_channel.topic,
                            "nsfw": guild_channel.nsfw,
                            "position": guild_channel.position,
                        })
                    }
                    serenity::all::Channel::Private(private_channel) => {
                        json!({
                            "type": "private",
                            "id": private_channel.id.to_string(),
                            "recipient": private_channel.recipient.name,
                            "recipient_id": private_channel.recipient.id.to_string(),
                        })
                    }
                    _ => json!({ "type": "unknown" }),
                };

                Ok(CallToolResult::success(vec![Content::text(
                    serde_json::to_string_pretty(&info).unwrap(),
                )]))
            }
            Err(e) => {
                error!("Failed to get Discord channel info: {}", e);
                Err(McpError::internal_error(
                    "Failed to get Discord channel info",
                    Some(json!({
                        "channel": params.channel,
                        "channel_id": channel_id.to_string(),
                        "error_type": "channel_not_found",
                        "details": e.to_string()
                    })),
                ))
            }
        }
    }
}

impl DiscordTools {
    /// Resolve a channel string to a ChannelId
    /// Accepts either:
    /// - A numeric channel ID (e.g. "123456789")
    /// - A guild/channel format (e.g. "MyServer/general")
    async fn resolve_channel(&self, channel: &str) -> Result<serenity::all::ChannelId, McpError> {
        use serenity::all::{ChannelId, GuildId};

        // First, try parsing as a direct channel ID
        if let Ok(id) = channel.parse::<u64>() {
            return Ok(ChannelId::new(id));
        }

        // Otherwise, try guild/channel format
        let parts: Vec<&str> = channel.split('/').collect();
        if parts.len() != 2 {
            return Err(McpError::invalid_params(
                "Channel must be either a numeric ID or 'guild/channel' format",
                Some(json!({
                    "channel": channel,
                    "expected": "123456789 or MyServer/general"
                })),
            ));
        }

        let guild_name = parts[0];
        let channel_name = parts[1];

        let client = self.discord_client.clone().ok_or_else(|| {
            McpError::internal_error(
                "Discord client not available",
                Some(json!({
                    "error_type": "discord_not_configured"
                })),
            )
        })?;

        let ctx = client.read().await.http.clone();

        // Get all guilds the bot is in
        let guilds = ctx.get_guilds(None, None).await.map_err(|e| {
            McpError::internal_error(
                "Failed to fetch guilds",
                Some(json!({
                    "error": e.to_string()
                })),
            )
        })?;

        // Find the matching guild
        let guild_id = if let Ok(id) = guild_name.parse::<u64>() {
            GuildId::new(id)
        } else {
            // Search by name
            guilds
                .iter()
                .find(|g| g.name.eq_ignore_ascii_case(guild_name))
                .map(|g| g.id)
                .ok_or_else(|| {
                    McpError::invalid_params(
                        "Guild not found",
                        Some(json!({
                            "guild": guild_name,
                            "available_guilds": guilds.iter().map(|g| &g.name).collect::<Vec<_>>()
                        })),
                    )
                })?
        };

        // Get guild channels
        let channels = guild_id.channels(&ctx).await.map_err(|e| {
            McpError::internal_error(
                "Failed to fetch channels",
                Some(json!({
                    "guild_id": guild_id.to_string(),
                    "error": e.to_string()
                })),
            )
        })?;

        // Find the matching channel
        let channel_id = channels
            .values()
            .find(|ch| ch.name.eq_ignore_ascii_case(channel_name))
            .map(|ch| ch.id)
            .ok_or_else(|| {
                McpError::invalid_params(
                    "Channel not found in guild",
                    Some(json!({
                        "guild": guild_name,
                        "channel": channel_name,
                        "available_channels": channels.values().map(|ch| &ch.name).collect::<Vec<_>>()
                    })),
                )
            })?;

        Ok(channel_id)
    }

    pub async fn send_discord_dm(
        &self,
        params: SendDiscordDmRequest,
    ) -> std::result::Result<CallToolResult, McpError> {
        use serenity::all::UserId;

        info!(user_id = params.user_id, "Sending Discord DM via MCP");

        let client = self.discord_client.clone().ok_or_else(|| {
            McpError::internal_error(
                "Discord client not available",
                Some(json!({
                    "error_type": "discord_not_configured",
                    "details": "Discord bot is not running or not configured"
                })),
            )
        })?;

        let ctx = client.read().await.http.clone();
        let user_id = UserId::new(params.user_id as u64);

        match user_id.create_dm_channel(&ctx).await {
            Ok(channel) => match channel.say(&ctx, &params.message).await {
                Ok(message) => {
                    info!("Discord DM sent successfully");
                    Ok(CallToolResult::success(vec![Content::text(
                        serde_json::to_string_pretty(&json!({
                            "success": true,
                            "message_id": message.id.to_string(),
                            "channel_id": channel.id.to_string(),
                            "user_id": params.user_id,
                            "timestamp": message.timestamp.to_string()
                        }))
                        .unwrap(),
                    )]))
                }
                Err(e) => {
                    error!("Failed to send Discord DM: {}", e);
                    Err(McpError::internal_error(
                        "Failed to send Discord DM",
                        Some(json!({
                            "user_id": params.user_id,
                            "error_type": "discord_dm_failed",
                            "details": e.to_string()
                        })),
                    ))
                }
            },
            Err(e) => {
                error!("Failed to create DM channel: {}", e);
                Err(McpError::internal_error(
                    "Failed to create DM channel",
                    Some(json!({
                        "user_id": params.user_id,
                        "error_type": "dm_channel_creation_failed",
                        "details": e.to_string()
                    })),
                ))
            }
        }
    }

    pub async fn send_discord_channel_message(
        &self,
        params: SendDiscordMessageRequest,
    ) -> std::result::Result<CallToolResult, McpError> {
        info!(channel = %params.channel, "Sending Discord message via MCP");

        // Resolve channel
        let channel_id = self.resolve_channel(&params.channel).await?;

        let client = self.discord_client.clone().ok_or_else(|| {
            McpError::internal_error(
                "Discord client not available",
                Some(json!({
                    "error_type": "discord_not_configured",
                    "details": "Discord bot is not running or not configured"
                })),
            )
        })?;

        let ctx = client.read().await.http.clone();

        match channel_id.say(&ctx, &params.message).await {
            Ok(message) => {
                info!("Discord message sent successfully");
                Ok(CallToolResult::success(vec![Content::text(
                    serde_json::to_string_pretty(&json!({
                        "success": true,
                        "message_id": message.id.to_string(),
                        "channel_id": channel_id.to_string(),
                        "channel": params.channel,
                        "timestamp": message.timestamp.to_string()
                    }))
                    .unwrap(),
                )]))
            }
            Err(e) => {
                error!("Failed to send Discord message: {}", e);
                Err(McpError::internal_error(
                    "Failed to send Discord message",
                    Some(json!({
                        "channel": params.channel,
                        "channel_id": channel_id.to_string(),
                        "error_type": "discord_send_failed",
                        "details": e.to_string()
                    })),
                ))
            }
        }
    }
}
