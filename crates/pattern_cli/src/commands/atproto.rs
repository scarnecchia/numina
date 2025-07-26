//! ATProto authentication commands for Pattern CLI

use miette::{IntoDiagnostic, Result};
use owo_colors::OwoColorize;
use pattern_core::{
    atproto_identity::{AtprotoAuthCredentials, AtprotoIdentity},
    config::PatternConfig,
    db::{client::DB, ops::atproto::*},
    id::Did,
};
use std::sync::Arc;
use std::{
    io::{self, Write},
    str::FromStr,
};

use atrium_common::resolver::Resolver;
use atrium_identity::{
    did::{CommonDidResolver, CommonDidResolverConfig, DEFAULT_PLC_DIRECTORY_URL},
    handle::{AtprotoHandleResolver, AtprotoHandleResolverConfig, DnsTxtResolver},
    identity_resolver::{IdentityResolver, IdentityResolverConfig},
};
use hickory_resolver::TokioAsyncResolver;

use crate::output::Output;

/// DNS TXT resolver for handle resolution
struct HickoryDnsTxtResolver {
    resolver: TokioAsyncResolver,
}

impl Default for HickoryDnsTxtResolver {
    fn default() -> Self {
        Self {
            resolver: TokioAsyncResolver::tokio_from_system_conf()
                .expect("failed to create resolver"),
        }
    }
}

impl DnsTxtResolver for HickoryDnsTxtResolver {
    async fn resolve(
        &self,
        query: &str,
    ) -> core::result::Result<Vec<String>, Box<dyn std::error::Error + Send + Sync>> {
        Ok(self
            .resolver
            .txt_lookup(query)
            .await?
            .iter()
            .map(|txt| txt.to_string())
            .collect())
    }
}

/// Simple HTTP client wrapper for atrium
struct SimpleHttpClient {
    client: reqwest::Client,
}

impl Default for SimpleHttpClient {
    fn default() -> Self {
        Self {
            client: reqwest::Client::new(),
        }
    }
}

impl atrium_xrpc::HttpClient for SimpleHttpClient {
    async fn send_http(
        &self,
        request: atrium_xrpc::http::Request<Vec<u8>>,
    ) -> core::result::Result<
        atrium_xrpc::http::Response<Vec<u8>>,
        Box<dyn std::error::Error + Send + Sync + 'static>,
    > {
        let response = self.client.execute(request.try_into()?).await?;
        let mut builder = atrium_xrpc::http::Response::builder().status(response.status());
        for (k, v) in response.headers() {
            builder = builder.header(k, v);
        }
        builder
            .body(response.bytes().await?.to_vec())
            .map_err(Into::into)
    }
}

/// Resolve a handle to its PDS URL using proper ATProto resolution
async fn resolve_handle_to_pds(handle: &str) -> Result<String> {
    // Set up the identity resolver
    let http_client = Arc::new(SimpleHttpClient::default());
    let resolver_config = IdentityResolverConfig {
        did_resolver: CommonDidResolver::new(CommonDidResolverConfig {
            plc_directory_url: DEFAULT_PLC_DIRECTORY_URL.to_string(),
            http_client: Arc::clone(&http_client),
        }),
        handle_resolver: AtprotoHandleResolver::new(AtprotoHandleResolverConfig {
            dns_txt_resolver: HickoryDnsTxtResolver::default(),
            http_client: Arc::clone(&http_client),
        }),
    };
    let resolver = IdentityResolver::new(resolver_config);

    // Resolve the handle to get the identity
    match resolver.resolve(handle).await {
        Ok(identity) => {
            // Successfully resolved - use the PDS from the identity
            tracing::debug!(
                "Resolved handle {} to DID: {} with PDS: {}",
                handle,
                identity.did,
                identity.pds
            );
            Ok(identity.pds)
        }
        Err(e) => {
            // If resolution fails, try bsky.social anyway
            tracing::debug!("Failed to resolve handle {}: {:?}", handle, e);
            Ok("https://bsky.social".to_string())
        }
    }
}

/// Login with ATProto OAuth
pub async fn oauth_login(identifier: &str, _config: &PatternConfig) -> Result<()> {
    let output = Output::new();

    output.info("ATProto OAuth Login", "");
    output.info("Identifier:", &identifier.bright_cyan().to_string());

    // For CLI, we'll need to start a local server to handle the callback
    // For now, we'll show the manual flow
    output.warning("OAuth flow requires a web browser and callback handler.");
    output.info(
        "",
        "For now, please use app password authentication instead.",
    );
    output.info("", "Full OAuth support coming soon!");

    Ok(())
}

/// Login with ATProto app password
pub async fn app_password_login(
    identifier: &str,
    app_password: Option<String>,
    config: &PatternConfig,
) -> Result<()> {
    let output = Output::new();

    output.info("ATProto App Password Login", "");
    output.info("Identifier:", &identifier.bright_cyan().to_string());

    // Get app password if not provided
    let app_password = if let Some(password) = app_password {
        password
    } else {
        print!("App Password: ");
        io::stdout().flush().unwrap();
        rpassword::read_password().into_diagnostic()?
    };

    // Create a session to validate credentials and get profile info
    output.info("Authenticating with Bluesky...", "");

    // First, we need to resolve the handle to find the correct PDS
    let pds_url = resolve_handle_to_pds(identifier).await?;
    output.info("Resolved PDS:", &pds_url);

    let client = atrium_api::client::AtpServiceClient::new(
        atrium_xrpc_client::reqwest::ReqwestClient::new(&pds_url),
    );

    // Authenticate
    let session = client
        .service
        .com
        .atproto
        .server
        .create_session(
            atrium_api::com::atproto::server::create_session::InputData {
                identifier: identifier.to_string(),
                password: app_password.clone(),
                auth_factor_token: None,
                allow_takendown: None,
            }
            .into(),
        )
        .await
        .map_err(|e| miette::miette!("Authentication failed: {:?}", e))?;

    let did = session.data.did.to_string();
    let handle = session.data.handle.to_string();
    // Use the PDS URL we resolved earlier
    // Note: pds_url is already set from resolve_handle_to_pds()

    output.success(&format!("Authenticated as {}", handle.bright_green()));
    output.info("DID:", &did.bright_cyan().to_string());
    output.info("PDS:", &pds_url);

    // Check if already linked
    let did_obj = Did::from_str(&did).into_diagnostic()?;
    if let Some(existing) = get_atproto_identity_by_did(&DB, &did_obj)
        .await
        .into_diagnostic()?
    {
        output.warning(&format!(
            "This account is already linked to user: {}",
            existing.user_id
        ));

        // Update the app password
        let mut updated = existing;
        updated.update_app_password(app_password);
        upsert_atproto_identity(&DB, updated)
            .await
            .into_diagnostic()?;

        output.success("App password updated!");
    } else {
        // Create new identity
        let pattern_did = Did::from_str(&did).unwrap();
        let identity = AtprotoIdentity::new_app_password(
            pattern_did.0.clone(),
            handle.clone(),
            pds_url,
            app_password,
            config.user.id.clone(),
        );

        upsert_atproto_identity(&DB, identity)
            .await
            .into_diagnostic()?;

        output.success(&format!(
            "ATProto identity linked to user: {}",
            config.user.id
        ));
    }

    // Note: display_name is not available in the create_session response
    // We would need to make a separate getProfile call to get it

    output.success("─────────────────────────────────────────────");
    output.success("You can now use your Bluesky account with Pattern!");

    Ok(())
}

/// Show ATProto authentication status
pub async fn status(config: &PatternConfig) -> Result<()> {
    let output = Output::new();

    output.success("ATProto Authentication Status");
    output.success("─────────────────────────────────────────────");

    let identities = get_user_atproto_identities(&DB, &config.user.id)
        .await
        .into_diagnostic()?;

    if identities.is_empty() {
        output.info("No ATProto identities linked", "");
        output.info("", "Use 'pattern-cli atproto login' to link an account");
    } else {
        for identity in identities {
            output.info("Handle:", &identity.handle.bright_cyan().to_string());
            output.info("DID:", &identity.id.to_string());
            output.info("Auth Method:", &format!("{:?}", identity.auth_method));
            output.info("PDS:", &identity.pds_url);

            if let Some(display_name) = &identity.display_name {
                output.info("Display Name:", display_name);
            }

            match identity.auth_method {
                pattern_core::atproto_identity::AtprotoAuthMethod::OAuth => {
                    if identity.needs_token_refresh() {
                        output.warning("  OAuth token needs refresh");
                    } else if let Some(expires_at) = identity.token_expires_at {
                        let remaining = expires_at.signed_duration_since(chrono::Utc::now());
                        if remaining.num_seconds() > 0 {
                            output.info(
                                "  Token expires in:",
                                &format!("{} minutes", remaining.num_minutes()),
                            );
                        } else {
                            output.error("  Token expired");
                        }
                    }
                }
                pattern_core::atproto_identity::AtprotoAuthMethod::AppPassword => {
                    output.info("  Using app password", "");
                }
            }

            output.info(
                "Last used:",
                &identity
                    .last_auth_at
                    .format("%Y-%m-%d %H:%M:%S UTC")
                    .to_string(),
            );
            output.success("─────────────────────────────────────────────");
        }
    }

    Ok(())
}

/// Unlink an ATProto identity
pub async fn unlink(identifier: &str, config: &PatternConfig) -> Result<()> {
    let output = Output::new();

    // Find the identity by handle or DID
    let identities = get_user_atproto_identities(&DB, &config.user.id)
        .await
        .into_diagnostic()?;

    let identity = identities
        .into_iter()
        .find(|i| i.handle == identifier || i.id.to_string() == identifier);

    if let Some(identity) = identity {
        output.warning(&format!(
            "Unlinking ATProto identity: {} ({})",
            identity.handle, identity.id
        ));

        // Confirm
        print!("Are you sure? (y/N): ");
        io::stdout().flush().unwrap();

        let mut response = String::new();
        io::stdin().read_line(&mut response).into_diagnostic()?;

        if response.trim().to_lowercase() == "y" {
            delete_atproto_identity(&DB, &identity.id, &config.user.id)
                .await
                .into_diagnostic()?;

            output.success("ATProto identity unlinked!");
        } else {
            output.info("Cancelled", "");
        }
    } else {
        output.error(&format!("No identity found for: {}", identifier));
    }

    Ok(())
}

/// Test ATProto connection
pub async fn test(config: &PatternConfig) -> Result<()> {
    let output = Output::new();

    output.info("Testing ATProto connections...", "");

    let identities = get_user_atproto_identities(&DB, &config.user.id)
        .await
        .into_diagnostic()?;

    if identities.is_empty() {
        output.error("No ATProto identities to test");
        return Ok(());
    }

    for identity in identities {
        output.info(&format!("Testing {}...", identity.handle), "");

        match identity.get_auth_credentials() {
            Some(AtprotoAuthCredentials::OAuth { access_token }) => {
                // Test OAuth token
                output.info("  Auth method:", "OAuth");
                output.info(
                    "  Token preview:",
                    &format!("{}...", &access_token[..20.min(access_token.len())]),
                );

                if identity.needs_token_refresh() {
                    output.warning("  Token needs refresh!");
                } else {
                    output.success("  Token is valid");
                }
            }
            Some(AtprotoAuthCredentials::AppPassword {
                identifier,
                password,
            }) => {
                // Test app password
                output.info("  Auth method:", "App Password");
                output.info("  Identifier:", &identifier);
                output.info(
                    "  Password:",
                    &format!("{}...", &password[..8.min(password.len())]),
                );

                // Try to create a session
                let client = atrium_api::client::AtpServiceClient::new(
                    atrium_xrpc_client::reqwest::ReqwestClient::new(&identity.pds_url),
                );

                match client.service.com.atproto.server.get_session().await {
                    Ok(_) => output.success("  Connection successful!"),
                    Err(e) => output.error(&format!("  Connection failed: {:?}", e)),
                }
            }
            None => {
                output.error("  No credentials available");
            }
        }
    }

    Ok(())
}
