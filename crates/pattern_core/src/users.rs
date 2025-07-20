use crate::UserId;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// User model
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct User {
    /// Unique identifier for this user
    pub id: UserId,

    /// When this user was created
    pub created_at: chrono::DateTime<chrono::Utc>,

    /// When this user was last updated
    pub updated_at: chrono::DateTime<chrono::Utc>,

    /// User-specific settings (e.g., preferences, notification settings)
    #[serde(default)]
    pub settings: HashMap<String, serde_json::Value>,

    /// Additional metadata about the user (e.g., source, tags)
    #[serde(default)]
    pub metadata: HashMap<String, serde_json::Value>,
}
