use chrono::{DateTime, Utc};
use pattern_macros::Entity;
use serde::{Deserialize, Serialize};

use crate::id::{DiscordIdentityId, UserId};

/// Discord identity mapping for Pattern users
#[derive(Debug, Clone, Entity, Serialize, Deserialize)]
#[entity(entity_type = "discord_identity")]
pub struct DiscordIdentity {
    /// Unique identifier for this mapping
    pub id: DiscordIdentityId,

    /// Pattern user this Discord identity belongs to
    pub user_id: UserId,

    /// Discord user ID (snowflake)
    pub discord_user_id: String,

    /// Discord username (without discriminator)
    pub username: String,

    /// Discord display name (server nickname if available)
    pub display_name: Option<String>,

    /// Discord global name (new username system)
    pub global_name: Option<String>,

    /// Whether this user can receive DMs from Pattern
    pub dm_allowed: bool,

    /// Default channel for this user (if they prefer channel over DM)
    pub default_channel_id: Option<String>,

    /// When this identity was linked
    pub linked_at: DateTime<Utc>,

    /// Last time we successfully sent a message
    pub last_message_at: Option<DateTime<Utc>>,

    /// Additional metadata
    pub metadata: serde_json::Value,
}

impl DiscordIdentity {
    /// Create a new Discord identity mapping
    pub fn new(user_id: UserId, discord_user_id: String, username: String) -> Self {
        Self {
            id: DiscordIdentityId::generate(),
            user_id,
            discord_user_id,
            username,
            display_name: None,
            global_name: None,
            dm_allowed: false, // Opt-in by default
            default_channel_id: None,
            linked_at: Utc::now(),
            last_message_at: None,
            metadata: serde_json::json!({}),
        }
    }

    /// Update display information from Discord
    pub fn update_from_discord(
        &mut self,
        username: String,
        display_name: Option<String>,
        global_name: Option<String>,
    ) {
        self.username = username;
        self.display_name = display_name;
        self.global_name = global_name;
    }

    /// Get the best available name for display
    pub fn best_name(&self) -> &str {
        self.display_name
            .as_deref()
            .or(self.global_name.as_deref())
            .unwrap_or(&self.username)
    }

    /// Check if this user can receive DMs
    pub fn can_receive_dm(&self) -> bool {
        self.dm_allowed
    }

    /// Mark that we sent a message
    pub fn mark_message_sent(&mut self) {
        self.last_message_at = Some(Utc::now());
    }
}

/// Relation between DiscordIdentity and User
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscordIdentityBelongsTo {
    pub linked_at: DateTime<Utc>,
    pub linked_by: String, // "oauth", "command", "admin"
}

impl Default for DiscordIdentityBelongsTo {
    fn default() -> Self {
        Self {
            linked_at: Utc::now(),
            linked_by: "command".to_string(),
        }
    }
}
