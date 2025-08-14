//! Data source setup and configuration
//!
//! This module handles credential management for data sources like Bluesky/ATProto.

use pattern_core::{
    config::PatternConfig,
    db::{
        client::DB,
        ops::atproto::get_user_atproto_identities,
    },
};

/// Get Bluesky credentials from configuration
pub async fn get_bluesky_credentials(
    config: &PatternConfig,
) -> Option<(
    pattern_core::atproto_identity::AtprotoAuthCredentials,
    String,
)> {
    // Check if agent has a bluesky_handle configured
    let bluesky_handle = if let Some(handle) = &config.agent.bluesky_handle {
        handle.clone()
    } else {
        // No Bluesky handle configured for this agent
        return None;
    };
    // Look up ATProto identity for this handle
    let identities = get_user_atproto_identities(&DB, &config.user.id)
        .await
        .ok()
        .unwrap_or_default();

    // Find identity matching the handle
    let identity = identities
        .into_iter()
        .find(|i| i.handle == bluesky_handle || i.id.to_string() == bluesky_handle);

    if let Some(identity) = identity {
        // Get credentials
        if let Some(creds) = identity.get_auth_credentials() {
            return Some((creds, bluesky_handle));
        }
    }
    None
}