use serde::{Deserialize, Serialize};

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
