//! OAuth integration that combines middleware with genai client
//!
//! This module provides the glue between Pattern's OAuth tokens,
//! the request transformation middleware, and genai's client.

use crate::error::CoreError;
use crate::id::UserId;
use crate::oauth::{OAuthToken, auth_flow::DeviceAuthFlow, resolver::OAuthClientBuilder};
use chrono::Utc;
use std::sync::Arc;
use surrealdb::{Connection, Surreal};

/// OAuth-enabled model provider that integrates with genai
pub struct OAuthModelProvider<C: Connection> {
    db: Arc<Surreal<C>>,
    user_id: UserId,
}

impl<C: Connection + 'static> OAuthModelProvider<C> {
    /// Create a new OAuth-enabled model provider
    pub fn new(db: Arc<Surreal<C>>, user_id: UserId) -> Self {
        Self { db, user_id }
    }

    /// Get or refresh OAuth token for a provider
    pub async fn get_token(&self, provider: &str) -> Result<Option<Arc<OAuthToken>>, CoreError> {
        // Try to get existing token
        let token = crate::db::ops::get_user_oauth_token(&self.db, &self.user_id, provider).await?;

        if let Some(mut token) = token {
            tracing::debug!(
                "Found OAuth token for provider '{}', expires at: {}, needs refresh: {}",
                provider,
                token.expires_at,
                token.needs_refresh()
            );

            // Check if token needs refresh
            if token.needs_refresh() && token.refresh_token.is_some() {
                tracing::info!(
                    "OAuth token for {} needs refresh (expires: {}), attempting refresh...",
                    provider,
                    token.expires_at
                );

                // Refresh the token
                let config = match provider {
                    "anthropic" => crate::oauth::auth_flow::OAuthConfig::anthropic(),
                    _ => {
                        return Err(CoreError::OAuthError {
                            provider: provider.to_string(),
                            operation: "get_config".to_string(),
                            details: format!("Unknown OAuth provider: {}", provider),
                        });
                    }
                };

                let flow = DeviceAuthFlow::new(config);

                tracing::debug!(
                    "Attempting token refresh with refresh_token: {}",
                    if token.refresh_token.is_some() {
                        "[PRESENT]"
                    } else {
                        "[MISSING]"
                    }
                );

                match flow
                    .refresh_token(token.refresh_token.clone().unwrap())
                    .await
                {
                    Ok(token_response) => {
                        // Calculate new expiry
                        let new_expires_at = Utc::now()
                            + chrono::Duration::seconds(token_response.expires_in as i64);

                        tracing::info!(
                            "OAuth token refresh successful! New token expires at: {} ({} seconds from now)",
                            new_expires_at,
                            token_response.expires_in
                        );

                        // Update the token in database
                        token = crate::db::ops::update_oauth_token(
                            &self.db,
                            &token.id,
                            token_response.access_token,
                            token_response.refresh_token,
                            new_expires_at,
                        )
                        .await?;
                    }
                    Err(e) => {
                        tracing::error!("OAuth token refresh failed: {}", e);
                        return Err(e);
                    }
                }
            } else if token.needs_refresh() && token.refresh_token.is_none() {
                tracing::warn!(
                    "OAuth token for {} needs refresh but no refresh token available! Token expires: {}",
                    provider,
                    token.expires_at
                );
            }

            // Mark token as used
            crate::db::ops::mark_oauth_token_used(&self.db, &token.id).await?;

            Ok(Some(Arc::new(token)))
        } else {
            tracing::debug!("No OAuth token found for provider '{}'", provider);
            Ok(None)
        }
    }

    /// Create a genai client with OAuth support
    pub async fn create_client(&self) -> Result<genai::Client, CoreError> {
        // Use the OAuth client builder
        OAuthClientBuilder::new(self.db.clone(), self.user_id.clone()).build()
    }

    /// Start OAuth flow for a provider
    pub async fn start_oauth_flow(
        &self,
        provider: &str,
    ) -> Result<(String, crate::oauth::PkceChallenge), CoreError> {
        let config = match provider {
            "anthropic" => crate::oauth::auth_flow::OAuthConfig::anthropic(),
            _ => {
                return Err(CoreError::OAuthError {
                    provider: provider.to_string(),
                    operation: "start_flow".to_string(),
                    details: format!("Unknown OAuth provider: {}", provider),
                });
            }
        };

        let flow = DeviceAuthFlow::new(config);
        Ok(flow.start_auth())
    }

    /// Complete OAuth flow with authorization code
    pub async fn complete_oauth_flow(
        &self,
        provider: &str,
        code: String,
        pkce_challenge: &crate::oauth::PkceChallenge,
    ) -> Result<OAuthToken, CoreError> {
        let config = match provider {
            "anthropic" => crate::oauth::auth_flow::OAuthConfig::anthropic(),
            _ => {
                return Err(CoreError::OAuthError {
                    provider: provider.to_string(),
                    operation: "complete_flow".to_string(),
                    details: format!("Unknown OAuth provider: {}", provider),
                });
            }
        };

        let flow = DeviceAuthFlow::new(config);
        let token_response = flow.exchange_code(code, pkce_challenge).await?;

        // Calculate expiry
        let expires_at = Utc::now() + chrono::Duration::seconds(token_response.expires_in as i64);

        // Store the token
        let token = crate::db::ops::create_oauth_token(
            &self.db,
            provider.to_string(),
            token_response.access_token,
            token_response.refresh_token,
            expires_at,
            self.user_id.clone(),
        )
        .await?;

        Ok(token)
    }

    /// Revoke OAuth tokens for a provider
    pub async fn revoke_oauth(&self, provider: &str) -> Result<usize, CoreError> {
        let count =
            crate::db::ops::delete_user_oauth_tokens(&self.db, &self.user_id, provider).await?;
        Ok(count)
    }
}
