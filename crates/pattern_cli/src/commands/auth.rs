//! OAuth authentication commands

use miette::{IntoDiagnostic, Result};
use owo_colors::OwoColorize;
use pattern_core::{
    config::PatternConfig,
    db::{client::DB, ops},
    oauth::{OAuthClient, OAuthProvider, OAuthToken, auth_flow::split_callback_code},
};
use std::io::{self, Write};

use crate::output::Output;

/// Login with OAuth
pub async fn login(provider: &str, config: &PatternConfig) -> Result<()> {
    let output = Output::new();

    // Parse provider
    let provider = match provider {
        "anthropic" => OAuthProvider::Anthropic,
        _ => {
            output.error(&format!("Unknown provider: {}", provider));
            return Ok(());
        }
    };

    output.info("Provider:", &provider.to_string().bright_cyan().to_string());

    // Create OAuth client
    let oauth_client = OAuthClient::new(provider.clone());

    // Start device flow
    output.success("Starting OAuth device flow...");
    let device_response = oauth_client.start_device_flow().into_diagnostic()?;

    // Display instructions
    output.success("─────────────────────────────────────────────");
    output.info(
        "Authorization URL:",
        &device_response.verification_uri.bright_yellow().to_string(),
    );
    output.success("─────────────────────────────────────────────");
    output.success("Please visit the URL above and authorize the application.");
    output.info("After authorization, copy the code shown on the page.", "");
    output.info("", "");

    // Prompt for the code
    print!("Enter the authorization code: ");
    io::stdout().flush().unwrap();

    let mut code = String::new();
    io::stdin().read_line(&mut code).into_diagnostic()?;
    let (code, state) = split_callback_code(code.trim())?;

    // Verify state matches
    if let Some(pkce) = &device_response.pkce_challenge {
        if state != pkce.state {
            output.error("State mismatch - authorization may have been tampered with");
            return Ok(());
        }

        // Exchange code for token
        let token_response = oauth_client
            .exchange_code(code, pkce)
            .await
            .into_diagnostic()?;

        output.success("Authentication successful!");

        // Log token details for debugging
        tracing::info!(
            "Received OAuth token response - has refresh_token: {}, expires_in: {} seconds",
            token_response.refresh_token.is_some(),
            token_response.expires_in
        );

        if token_response.refresh_token.is_none() {
            tracing::warn!(
                "Anthropic OAuth response did not include a refresh token! Token will expire in {} seconds.",
                token_response.expires_in
            );
            output.warning("Note: No refresh token received. You'll need to re-authenticate when the token expires.");
        }

        // Store token in database
        tracing::info!("Storing OAuth token for user: {}", config.user.id);
        let expires_at =
            chrono::Utc::now() + chrono::Duration::seconds(token_response.expires_in as i64);
        let mut token = OAuthToken::new(
            provider.to_string(),
            token_response.access_token,
            token_response.refresh_token,
            expires_at,
            config.user.id.clone(),
        );

        // Set optional fields
        if let Some(scope) = token_response.scope {
            token.scope = Some(scope);
        }

        ops::create_oauth_token(
            &DB,
            token.provider,
            token.access_token,
            token.refresh_token,
            token.expires_at,
            token.owner_id,
        )
        .await
        .into_diagnostic()?;

        output.success(&format!("Token stored for provider: {}", provider));
        output.info(
            "You can now use OAuth authentication with this provider.",
            "",
        );
    } else {
        output.error("No PKCE challenge found - this shouldn't happen");
    }

    Ok(())
}

/// Show authentication status
pub async fn status(config: &PatternConfig) -> Result<()> {
    let output = Output::new();

    tracing::info!("Checking OAuth status for user: {}", config.user.id);

    output.success("OAuth Authentication Status");
    output.success("─────────────────────────────────────────────");

    // Check each provider
    for provider in &[OAuthProvider::Anthropic] {
        let token = ops::get_user_oauth_token(&DB, &config.user.id, &provider.to_string())
            .await
            .into_diagnostic()?;

        match token {
            Some(token) => {
                output.info(
                    &format!("{}:", provider),
                    &"authenticated".bright_green().to_string(),
                );

                if token.needs_refresh() {
                    output.warning("  Token needs refresh");
                }

                if token.expires_at != chrono::DateTime::<chrono::Utc>::default() {
                    let expires_at = token.expires_at;
                    let remaining = expires_at.signed_duration_since(chrono::Utc::now());
                    if remaining.num_seconds() > 0 {
                        output.info(
                            "  Expires in:",
                            &format!("{} minutes", remaining.num_minutes()),
                        );
                    } else {
                        output.error("  Token expired");
                    }
                }
            }
            None => {
                output.info(
                    &format!("{}:", provider),
                    &"not authenticated".bright_red().to_string(),
                );
            }
        }
    }

    Ok(())
}

/// Logout (remove stored tokens)
pub async fn logout(provider: &str, config: &PatternConfig) -> Result<()> {
    let output = Output::new();

    // Parse provider
    let provider = match provider {
        "anthropic" => OAuthProvider::Anthropic,
        _ => {
            output.error(&format!("Unknown provider: {}", provider));
            return Ok(());
        }
    };

    // Delete token from database
    let count = ops::delete_user_oauth_tokens(&DB, &config.user.id, &provider.to_string())
        .await
        .into_diagnostic()?;

    if count > 0 {
        output.success(&format!("Logged out from {}", provider));
    } else {
        output.warning(&format!("No token found for {}", provider));
    }

    Ok(())
}
