//! Discord OAuth2 implementation
//!
//! Implements Discord's OAuth2 flow for user authentication
//! and linking Discord accounts to Pattern users.

use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tracing::{debug, info, warn};

use pattern_core::{
    discord_identity::DiscordIdentity,
    db::client::DB,
    db::ops,
    id::UserId,
    Result, CoreError,
};

/// Discord OAuth2 configuration
#[derive(Debug, Clone)]
pub struct DiscordOAuthConfig {
    /// Client ID (same as APP_ID)
    pub client_id: String,
    /// Client Secret (from Discord developer portal)
    pub client_secret: String,
    /// Redirect URI (must match Discord app settings)
    pub redirect_uri: String,
    /// Discord OAuth2 endpoints
    pub auth_endpoint: String,
    pub token_endpoint: String,
    pub api_endpoint: String,
}

impl DiscordOAuthConfig {
    /// Create config from environment variables
    pub fn from_env() -> Result<Self> {
        let client_id = std::env::var("APP_ID")
            .or_else(|_| std::env::var("DISCORD_CLIENT_ID"))
            .map_err(|_| CoreError::config_missing("APP_ID or DISCORD_CLIENT_ID"))?;
            
        let client_secret = std::env::var("DISCORD_CLIENT_SECRET")
            .map_err(|_| CoreError::config_missing("DISCORD_CLIENT_SECRET"))?;
            
        let redirect_uri = std::env::var("DISCORD_REDIRECT_URI")
            .unwrap_or_else(|_| "http://localhost:3000/auth/discord/callback".to_string());
            
        Ok(Self {
            client_id,
            client_secret,
            redirect_uri,
            auth_endpoint: "https://discord.com/api/oauth2/authorize".to_string(),
            token_endpoint: "https://discord.com/api/oauth2/token".to_string(),
            api_endpoint: "https://discord.com/api/v10".to_string(),
        })
    }
    
    /// Get authorization URL for user to visit
    pub fn get_auth_url(&self, state: &str) -> String {
        let scopes = "identify"; // Can add more scopes like "guilds", "email"
        format!(
            "{}?client_id={}&redirect_uri={}&response_type=code&scope={}&state={}",
            self.auth_endpoint,
            self.client_id,
            urlencoding::encode(&self.redirect_uri),
            scopes,
            state
        )
    }
}

/// Discord OAuth2 token response
#[derive(Debug, Deserialize)]
pub struct DiscordTokenResponse {
    pub access_token: String,
    pub token_type: String,
    pub expires_in: u64,
    pub refresh_token: String,
    pub scope: String,
}

/// Discord user information
#[derive(Debug, Deserialize)]
pub struct DiscordUser {
    pub id: String,
    pub username: String,
    pub discriminator: String,
    pub global_name: Option<String>,
    pub avatar: Option<String>,
    pub bot: Option<bool>,
    pub system: Option<bool>,
    pub mfa_enabled: Option<bool>,
    pub locale: Option<String>,
    pub verified: Option<bool>,
    pub email: Option<String>,
    pub flags: Option<u64>,
    pub premium_type: Option<u8>,
    pub public_flags: Option<u64>,
}

/// Discord OAuth2 client
pub struct DiscordOAuthClient {
    config: DiscordOAuthConfig,
    http_client: Client,
}

impl DiscordOAuthClient {
    /// Create new OAuth client
    pub fn new(config: DiscordOAuthConfig) -> Self {
        Self {
            config,
            http_client: Client::new(),
        }
    }
    
    /// Exchange authorization code for access token
    pub async fn exchange_code(&self, code: &str) -> Result<DiscordTokenResponse> {
        let mut params = HashMap::new();
        params.insert("client_id", self.config.client_id.as_str());
        params.insert("client_secret", self.config.client_secret.as_str());
        params.insert("grant_type", "authorization_code");
        params.insert("code", code);
        params.insert("redirect_uri", self.config.redirect_uri.as_str());
        
        let response = self.http_client
            .post(&self.config.token_endpoint)
            .form(&params)
            .send()
            .await
            .map_err(|e| CoreError::ExternalApiError {
                service: "Discord OAuth".to_string(),
                message: format!("Failed to exchange code: {}", e),
            })?;
            
        if !response.status().is_success() {
            let error_text = response.text().await.unwrap_or_default();
            return Err(CoreError::ExternalApiError {
                service: "Discord OAuth".to_string(),
                message: format!("Token exchange failed: {}", error_text),
            });
        }
        
        response
            .json::<DiscordTokenResponse>()
            .await
            .map_err(|e| CoreError::ExternalApiError {
                service: "Discord OAuth".to_string(),
                message: format!("Failed to parse token response: {}", e),
            })
    }
    
    /// Get user information using access token
    pub async fn get_user(&self, access_token: &str) -> Result<DiscordUser> {
        let response = self.http_client
            .get(&format!("{}/users/@me", self.config.api_endpoint))
            .header("Authorization", format!("Bearer {}", access_token))
            .send()
            .await
            .map_err(|e| CoreError::ExternalApiError {
                service: "Discord API".to_string(),
                message: format!("Failed to get user info: {}", e),
            })?;
            
        if !response.status().is_success() {
            let error_text = response.text().await.unwrap_or_default();
            return Err(CoreError::ExternalApiError {
                service: "Discord API".to_string(),
                message: format!("Get user failed: {}", error_text),
            });
        }
        
        response
            .json::<DiscordUser>()
            .await
            .map_err(|e| CoreError::ExternalApiError {
                service: "Discord API".to_string(),
                message: format!("Failed to parse user response: {}", e),
            })
    }
    
    /// Complete OAuth flow and link Discord account to Pattern user
    pub async fn link_discord_account(
        &self,
        code: &str,
        pattern_user_id: UserId,
    ) -> Result<DiscordIdentity> {
        // Exchange code for token
        let token_response = self.exchange_code(code).await?;
        info!("Successfully exchanged Discord OAuth code for token");
        
        // Get user information
        let discord_user = self.get_user(&token_response.access_token).await?;
        info!("Retrieved Discord user info: {} ({})", discord_user.username, discord_user.id);
        
        // Check if this Discord ID is already linked
        if let Ok(Some(existing)) = ops::get_discord_identity_by_discord_id(&DB, &discord_user.id).await {
            if existing.user_id != pattern_user_id {
                return Err(CoreError::ValidationError {
                    field: "discord_id".to_string(),
                    message: "This Discord account is already linked to another Pattern user".to_string(),
                });
            }
            // Update existing identity
            let mut updated = existing;
            updated.update_from_discord(
                discord_user.username.clone(),
                None, // Discord OAuth doesn't provide server nicknames
                discord_user.global_name.clone(),
            );
            ops::update_discord_identity(&DB, &updated).await?;
            return Ok(updated);
        }
        
        // Create new Discord identity
        let mut identity = DiscordIdentity::new(
            pattern_user_id,
            discord_user.id.clone(),
            discord_user.username.clone(),
        );
        
        identity.global_name = discord_user.global_name;
        identity.metadata = serde_json::json!({
            "linked_via": "oauth",
            "avatar": discord_user.avatar,
            "verified": discord_user.verified.unwrap_or(false),
            "locale": discord_user.locale,
        });
        
        // Save to database
        ops::create_discord_identity(&DB, &identity).await?;
        
        info!("Successfully linked Discord account {} to Pattern user {}", 
            discord_user.username, pattern_user_id);
        
        Ok(identity)
    }
}

/// Generate a secure random state parameter for CSRF protection
pub fn generate_state() -> String {
    use rand::Rng;
    use base64::Engine;
    
    let mut random_bytes = vec![0u8; 32];
    rand::rng().fill(&mut random_bytes[..]);
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(random_bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_auth_url_generation() {
        let config = DiscordOAuthConfig {
            client_id: "123456789".to_string(),
            client_secret: "secret".to_string(),
            redirect_uri: "http://localhost:3000/callback".to_string(),
            auth_endpoint: "https://discord.com/api/oauth2/authorize".to_string(),
            token_endpoint: "https://discord.com/api/oauth2/token".to_string(),
            api_endpoint: "https://discord.com/api/v10".to_string(),
        };
        
        let state = "test_state";
        let url = config.get_auth_url(state);
        
        assert!(url.contains("client_id=123456789"));
        assert!(url.contains("redirect_uri=http%3A%2F%2Flocalhost%3A3000%2Fcallback"));
        assert!(url.contains("response_type=code"));
        assert!(url.contains("scope=identify"));
        assert!(url.contains("state=test_state"));
    }
    
    #[test]
    fn test_state_generation() {
        let state1 = generate_state();
        let state2 = generate_state();
        
        // States should be different
        assert_ne!(state1, state2);
        
        // States should be long enough for security
        assert!(state1.len() >= 32);
        assert!(state2.len() >= 32);
    }
}