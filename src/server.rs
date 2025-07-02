use crate::{agent::UserId, agents::MultiAgentSystem, db::Database};
use rmcp::{
    handler::server::tool::Parameters,
    model::{CallToolResult, Content, Implementation, InitializeResult as ServerInfo},
    Error as McpError, ServerHandler,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{future::Future, sync::Arc};
use tracing::{debug, error, info};

// Request structs for tools
#[derive(Debug, Deserialize, Serialize, schemars::JsonSchema)]
struct ChatWithAgentRequest {
    user_id: i64,
    message: String,
    agent_id: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, schemars::JsonSchema)]
struct GetAgentMemoryRequest {
    user_id: i64,
    agent_id: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, schemars::JsonSchema)]
struct UpdateAgentMemoryRequest {
    user_id: i64,
    memory_json: String,
    agent_id: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, schemars::JsonSchema)]
struct ScheduleEventRequest {
    user_id: i64,
    title: String,
    start_time: String,
    description: Option<String>,
    duration_minutes: Option<u32>,
}

#[derive(Debug, Deserialize, Serialize, schemars::JsonSchema)]
struct SendMessageRequest {
    channel_id: u64,
    message: String,
}

/// MCP server implementation for Pattern
#[derive(Clone)]
pub struct PatternMcpServer {
    letta_client: Arc<letta::LettaClient>,
    db: Arc<Database>,
    multi_agent_system: Arc<MultiAgentSystem>,
    #[cfg(feature = "discord")]
    discord_client: Option<Arc<tokio::sync::RwLock<serenity::Client>>>,
}

impl PatternMcpServer {
    pub fn new(
        letta_client: Arc<letta::LettaClient>,
        db: Arc<Database>,
        multi_agent_system: Arc<MultiAgentSystem>,
        #[cfg(feature = "discord")] discord_client: Option<
            Arc<tokio::sync::RwLock<serenity::Client>>,
        >,
    ) -> Self {
        Self {
            letta_client,
            db,
            multi_agent_system,
            #[cfg(feature = "discord")]
            discord_client,
        }
    }

    /// Run the MCP server on stdio transport
    pub async fn run_stdio(self) -> miette::Result<()> {
        info!("Starting MCP server on stdio transport");

        use tokio::io::{stdin, stdout};

        let transport = (stdin(), stdout());
        let server = rmcp::ServiceExt::serve(self, transport)
            .await
            .map_err(|e| miette::miette!("Failed to start server: {}", e))?;

        let quit_reason = server
            .waiting()
            .await
            .map_err(|e| miette::miette!("Server error: {}", e))?;

        info!("Server stopped: {:?}", quit_reason);
        Ok(())
    }

    /// Run the MCP server on SSE transport
    #[cfg(feature = "mcp-sse")]
    pub async fn run_sse(self, port: u16) -> miette::Result<()> {
        info!("Starting MCP server on SSE transport at port {}", port);

        // TODO: Implement SSE transport when available in rmcp
        Err(miette::miette!("SSE transport not yet implemented"))
    }
}

impl ServerHandler for PatternMcpServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            server_info: Implementation {
                name: "pattern".to_string(),
                version: env!("CARGO_PKG_VERSION").to_string(),
            },
            instructions: Some(
                "Pattern MCP server for multi-agent ADHD support system with Discord integration"
                    .to_string(),
            ),
            ..Default::default()
        }
    }
}

// Tool implementations for agent operations
#[rmcp::tool_router(router = agent_tool_router)]
impl PatternMcpServer {
    #[rmcp::tool(description = "Send a message to a Letta agent and get a response")]
    async fn chat_with_agent(
        &self,
        params: Parameters<ChatWithAgentRequest>,
    ) -> Result<CallToolResult, McpError> {
        let params = params.0;
        info!(user_id = params.user_id, agent_id = ?params.agent_id, "Sending message to agent");
        debug!(message = %params.message, "Message content");

        match self
            .chat_with_agent_internal(
                UserId(params.user_id),
                &params.message,
                params.agent_id.as_deref(),
            )
            .await
        {
            Ok(response) => {
                info!("Agent responded successfully");
                Ok(CallToolResult::success(vec![Content::text(response)]))
            }
            Err(e) => {
                error!("Agent error: {}", e);
                Err(McpError::internal_error(
                    "Error communicating with agent",
                    Some(json!({
                        "agent_id": params.agent_id,
                        "user_id": params.user_id,
                        "error_type": "agent_communication_failure",
                        "details": e.to_string()
                    })),
                ))
            }
        }
    }

    #[rmcp::tool(description = "Get the current memory state of an agent")]
    async fn get_agent_memory(
        &self,
        params: Parameters<GetAgentMemoryRequest>,
    ) -> Result<CallToolResult, McpError> {
        let params = params.0;
        info!(user_id = params.user_id, agent_id = ?params.agent_id, "Getting agent memory");

        match self
            .get_agent_memory_internal(UserId(params.user_id), params.agent_id.as_deref())
            .await
        {
            Ok(memory) => {
                info!("Retrieved agent memory successfully");
                Ok(CallToolResult::success(vec![Content::text(
                    serde_json::to_string_pretty(&memory).unwrap_or_else(|_| "Error".to_string()),
                )]))
            }
            Err(e) => {
                error!("Error getting agent memory: {}", e);
                Err(McpError::internal_error(
                    "Error retrieving agent memory",
                    Some(json!({
                        "agent_id": params.agent_id,
                        "user_id": params.user_id,
                        "error_type": "memory_retrieval_failure",
                        "details": e.to_string()
                    })),
                ))
            }
        }
    }

    #[rmcp::tool(description = "Update agent memory blocks")]
    async fn update_agent_memory(
        &self,
        params: Parameters<UpdateAgentMemoryRequest>,
    ) -> Result<CallToolResult, McpError> {
        let params = params.0;
        info!(user_id = params.user_id, agent_id = ?params.agent_id, "Updating agent memory");

        let updates: serde_json::Value = match serde_json::from_str(&params.memory_json) {
            Ok(json) => json,
            Err(e) => {
                error!("Invalid JSON in memory update: {}", e);
                return Err(McpError::invalid_params(
                    "Invalid JSON in memory_json parameter",
                    Some(json!({
                        "parameter": "memory_json",
                        "error_type": "json_parse_error",
                        "details": e.to_string(),
                        "line": e.line(),
                        "column": e.column()
                    })),
                ));
            }
        };

        match self
            .update_agent_memory_internal(
                UserId(params.user_id),
                params.agent_id.as_deref(),
                &updates,
            )
            .await
        {
            Ok(()) => {
                info!("Agent memory updated successfully");
                Ok(CallToolResult::success(vec![Content::text(
                    "Memory updated successfully",
                )]))
            }
            Err(e) => {
                error!("Error updating agent memory: {}", e);
                Err(McpError::internal_error(
                    "Error updating agent memory",
                    Some(json!({
                        "agent_id": params.agent_id,
                        "user_id": params.user_id,
                        "error_type": "memory_update_failure",
                        "details": e.to_string()
                    })),
                ))
            }
        }
    }

    #[rmcp::tool(description = "Schedule an event with smart time estimation")]
    async fn schedule_event(
        &self,
        params: Parameters<ScheduleEventRequest>,
    ) -> Result<CallToolResult, McpError> {
        let params = params.0;
        info!(
            user_id = params.user_id,
            title = params.title,
            "Scheduling event"
        );

        // TODO: Implement event scheduling
        Ok(CallToolResult::success(vec![Content::text(
            "Event scheduling not yet implemented",
        )]))
    }

    #[rmcp::tool(description = "Send message to user via Discord")]
    async fn send_message(
        &self,
        params: Parameters<SendMessageRequest>,
    ) -> Result<CallToolResult, McpError> {
        let params = params.0;
        info!(channel_id = params.channel_id, "Sending Discord message");

        // TODO: Implement Discord message sending
        // This will need access to the Discord bot instance
        Ok(CallToolResult::success(vec![Content::text(
            "Discord messaging not yet implemented",
        )]))
    }

    #[rmcp::tool(description = "Check activity state for interruption timing")]
    async fn check_activity_state(&self) -> Result<CallToolResult, McpError> {
        info!("Checking activity state");

        // TODO: Implement activity monitoring
        Ok(CallToolResult::success(vec![Content::text(
            serde_json::to_string_pretty(&json!({
                "interruptible": true,
                "current_focus": null,
                "last_activity": "idle"
            }))
            .unwrap_or_else(|_| "Error".to_string()),
        )]))
    }

    // Discord integration tools
    #[cfg(feature = "discord")]
    #[rmcp::tool(description = "Send a message to a Discord channel")]
    async fn send_discord_message(
        &self,
        params: Parameters<crate::mcp_tools::SendDiscordMessageRequest>,
    ) -> Result<CallToolResult, McpError> {
        let params = params.0;
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

    #[cfg(feature = "discord")]
    #[rmcp::tool(description = "Send an embed message to a Discord channel")]
    async fn send_discord_embed(
        &self,
        params: Parameters<crate::mcp_tools::SendDiscordEmbedRequest>,
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

    #[cfg(feature = "discord")]
    #[rmcp::tool(description = "Get information about a Discord channel")]
    async fn get_discord_channel_info(
        &self,
        params: Parameters<crate::mcp_tools::GetDiscordChannelInfoRequest>,
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

    #[cfg(feature = "discord")]
    #[rmcp::tool(description = "Send a direct message to a Discord user")]
    async fn send_discord_dm(
        &self,
        params: Parameters<crate::mcp_tools::SendDiscordDmRequest>,
    ) -> Result<CallToolResult, McpError> {
        use serenity::all::UserId;

        let params = params.0;
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
        let user_id = UserId::new(params.user_id);

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
}

// Internal implementation methods
impl PatternMcpServer {
    /// Resolve a channel string to a ChannelId
    /// Accepts either:
    /// - A numeric channel ID (e.g. "123456789")
    /// - A guild/channel format (e.g. "MyServer/general")
    #[cfg(feature = "discord")]
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
    async fn chat_with_agent_internal(
        &self,
        user_id: UserId,
        message: &str,
        agent_id: Option<&str>,
    ) -> miette::Result<String> {
        // Initialize user if needed
        self.multi_agent_system.initialize_user(user_id).await?;

        // Update active context
        let context = format!("task: chat | message: {} | agent: {:?}", message, agent_id);
        self.multi_agent_system
            .update_shared_memory(user_id, "active_context", &context)
            .await?;

        // Route to specific agent and get response
        let response = self
            .multi_agent_system
            .send_message_to_agent(user_id, agent_id, message)
            .await?;

        Ok(response)
    }

    async fn get_agent_memory_internal(
        &self,
        user_id: UserId,
        _agent_id: Option<&str>,
    ) -> miette::Result<serde_json::Value> {
        // Get shared memory
        let memory = self
            .multi_agent_system
            .get_all_shared_memory(user_id)
            .await?;

        Ok(serde_json::json!({
            "shared_memory": memory,
            // TODO: Add agent-specific memory when we implement per-agent memory
        }))
    }

    async fn update_agent_memory_internal(
        &self,
        user_id: UserId,
        _agent_id: Option<&str>,
        updates: &serde_json::Value,
    ) -> miette::Result<()> {
        // Update shared memory blocks
        if let Some(obj) = updates.as_object() {
            for (key, value) in obj {
                if let Some(value_str) = value.as_str() {
                    self.multi_agent_system
                        .update_shared_memory(user_id, key, value_str)
                        .await?;
                }
            }
        }

        Ok(())
    }
}
