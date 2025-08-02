//! OAuth device authorization flow implementation
//!
//! Implements the OAuth 2.0 device authorization flow with PKCE
//! for authenticating with external services like Anthropic.

use super::{PkceChallenge, TokenRequest, TokenResponse};
use crate::error::CoreError;
use base64::Engine;
use rand::Rng;
use sha2::{Digest, Sha256};
use std::collections::HashMap;

/// OAuth client configuration
#[derive(Debug, Clone)]
pub struct OAuthConfig {
    /// Client ID for the OAuth application
    pub client_id: String,

    /// OAuth authorization endpoint
    pub auth_endpoint: String,

    /// OAuth token endpoint
    pub token_endpoint: String,

    /// Redirect URI (for device flow, typically a local callback)
    pub redirect_uri: String,

    /// Requested scopes
    pub scopes: Vec<String>,
}

impl OAuthConfig {
    /// Create config for Anthropic OAuth
    pub fn anthropic() -> Self {
        Self {
            client_id: "9d1c250a-e61b-44d9-88ed-5944d1962f5e".to_string(),
            auth_endpoint: "https://claude.ai/oauth/authorize".to_string(),
            token_endpoint: "https://console.anthropic.com/v1/oauth/token".to_string(),
            redirect_uri: "https://console.anthropic.com/oauth/code/callback".to_string(),
            scopes: vec![
                "org:create_api_key".to_string(),
                "user:profile".to_string(),
                "user:inference".to_string(),
            ],
        }
    }
}

/// Generate PKCE code verifier (64 random bytes, base64url encoded)
pub fn generate_code_verifier() -> String {
    let mut random_bytes = vec![0u8; 64];
    rand::rng().fill(&mut random_bytes[..]);
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(random_bytes)
}

/// Generate PKCE code challenge from verifier (SHA256 hash, base64url encoded)
pub fn generate_code_challenge(verifier: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(verifier.as_bytes());
    let result = hasher.finalize();
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(result)
}

/// Generate random state parameter for CSRF protection
pub fn generate_state() -> String {
    let mut random_bytes = vec![0u8; 64];
    rand::rng().fill(&mut random_bytes[..]);
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(random_bytes)
}

/// Device authorization flow manager
pub struct DeviceAuthFlow {
    config: OAuthConfig,
    http_client: reqwest::Client,
}

impl DeviceAuthFlow {
    /// Create a new device auth flow
    pub fn new(config: OAuthConfig) -> Self {
        Self {
            config,
            http_client: reqwest::Client::new(),
        }
    }

    /// Start the device authorization flow
    ///
    /// Returns the authorization URL and PKCE challenge
    pub fn start_auth(&self) -> (String, PkceChallenge) {
        let verifier = generate_code_verifier();
        let challenge = generate_code_challenge(&verifier);
        let state = generate_state();

        let pkce_challenge = PkceChallenge::new(verifier, challenge.clone(), state.clone());

        // Build authorization URL
        let scope = self.config.scopes.join(" ");
        let auth_params = vec![
            ("code", "true"),
            ("client_id", &self.config.client_id),
            ("response_type", "code"),
            ("redirect_uri", &self.config.redirect_uri),
            ("scope", &scope),
            ("code_challenge", &challenge),
            ("code_challenge_method", "S256"),
            ("state", &state),
        ];

        let auth_url = format!(
            "{}?{}",
            self.config.auth_endpoint,
            serde_urlencoded::to_string(auth_params).unwrap()
        );

        (auth_url, pkce_challenge)
    }

    /// Exchange authorization code for tokens
    pub async fn exchange_code(
        &self,
        code: String,
        pkce_challenge: &PkceChallenge,
    ) -> Result<TokenResponse, CoreError> {
        let token_request = TokenRequest::AuthorizationCode {
            client_id: self.config.client_id.clone(),
            code,
            redirect_uri: self.config.redirect_uri.clone(),
            code_verifier: pkce_challenge.verifier.clone(),
            state: Some(pkce_challenge.state.clone()),
        };

        self.exchange_tokens(token_request).await
    }

    /// Refresh an access token
    pub async fn refresh_token(&self, refresh_token: String) -> Result<TokenResponse, CoreError> {
        let token_request = TokenRequest::RefreshToken {
            client_id: self.config.client_id.clone(),
            refresh_token,
        };

        self.exchange_tokens(token_request).await
    }

    /// Common token exchange logic
    async fn exchange_tokens(&self, request: TokenRequest) -> Result<TokenResponse, CoreError> {
        // Serialize based on the enum variant
        let form_data = match &request {
            TokenRequest::AuthorizationCode {
                client_id,
                code,
                redirect_uri,
                code_verifier,
                state,
            } => {
                let mut params = vec![
                    ("grant_type", "authorization_code"),
                    ("client_id", client_id),
                    ("code", code),
                    ("redirect_uri", redirect_uri),
                    ("code_verifier", code_verifier),
                ];
                if let Some(state) = state {
                    params.push(("state", state));
                }
                params
            }
            TokenRequest::RefreshToken {
                client_id,
                refresh_token,
            } => vec![
                ("grant_type", "refresh_token"),
                ("client_id", client_id),
                ("refresh_token", refresh_token),
            ],
        };

        let response = self
            .http_client
            .post(&self.config.token_endpoint)
            .header("Content-Type", "application/x-www-form-urlencoded")
            .form(&form_data)
            .send()
            .await
            .map_err(|e| CoreError::OAuthError {
                provider: "anthropic".to_string(),
                operation: "token_exchange".to_string(),
                details: format!("HTTP request failed: {}", e),
            })?;

        let status = response.status();
        if !status.is_success() {
            let error_text = response.text().await.unwrap_or_default();
            return Err(CoreError::OAuthError {
                provider: "anthropic".to_string(),
                operation: "token_exchange".to_string(),
                details: format!(
                    "Token exchange failed with status {}: {}",
                    status, error_text
                ),
            });
        }

        let token_response: TokenResponse =
            response.json().await.map_err(|e| CoreError::OAuthError {
                provider: "anthropic".to_string(),
                operation: "token_parse".to_string(),
                details: format!("Failed to parse token response: {}", e),
            })?;

        Ok(token_response)
    }
}

pub fn split_callback_code(code: &str) -> Result<(String, String), CoreError> {
    let mut split = code.split('#');
    let code = split.next().unwrap().to_string();
    let state = split.next().unwrap().to_string();
    Ok((code, state))
}

/// Helper to parse authorization code from callback URL
pub fn parse_callback_code(url: &str) -> Result<(String, String), CoreError> {
    let parsed = url::Url::parse(url).map_err(|e| CoreError::OAuthError {
        provider: "anthropic".to_string(),
        operation: "callback_parse".to_string(),
        details: format!("Invalid callback URL '{}': {}", url, e),
    })?;

    let params: HashMap<String, String> = parsed
        .query_pairs()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();

    let code = params
        .get("code")
        .ok_or_else(|| CoreError::OAuthError {
            provider: "anthropic".to_string(),
            operation: "callback_parse".to_string(),
            details: format!("Missing 'code' parameter in callback URL: {}", url),
        })?
        .clone();

    let state = params
        .get("state")
        .ok_or_else(|| CoreError::OAuthError {
            provider: "anthropic".to_string(),
            operation: "callback_parse".to_string(),
            details: format!("Missing 'state' parameter in callback URL: {}", url),
        })?
        .clone();

    Ok((code, state))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pkce_generation() {
        let verifier = generate_code_verifier();
        let challenge = generate_code_challenge(&verifier);

        // Verifier should be base64url encoded without padding
        assert!(!verifier.contains('='));
        assert!(!verifier.contains('+'));
        assert!(!verifier.contains('/'));

        // Challenge should also be base64url encoded
        assert!(!challenge.contains('='));
        assert!(!challenge.contains('+'));
        assert!(!challenge.contains('/'));

        // Should be deterministic
        let challenge2 = generate_code_challenge(&verifier);
        assert_eq!(challenge, challenge2);
    }

    #[test]
    fn test_callback_parsing() {
        let url = "http://localhost:4001/callback?code=test_code_123&state=test_state_456";
        let (code, state) = parse_callback_code(url).unwrap();
        assert_eq!(code, "test_code_123");
        assert_eq!(state, "test_state_456");

        // Test error cases
        let bad_url = "http://localhost:4001/callback?state=test_state";
        assert!(parse_callback_code(bad_url).is_err());
    }

    #[test]
    fn test_auth_url_generation() {
        let config = OAuthConfig::anthropic();
        let flow = DeviceAuthFlow::new(config);
        let (auth_url, pkce) = flow.start_auth();

        // URL should contain all required parameters
        assert!(auth_url.contains("response_type=code"));
        assert!(auth_url.contains("code_challenge="));
        assert!(auth_url.contains("code_challenge_method=S256"));
        assert!(auth_url.contains("state="));

        // PKCE challenge should be valid
        assert!(pkce.is_valid());
    }
}
