//! ATProto OAuth authentication flow for Pattern API server

use crate::{auth::Claims, state::AppState};
use axum::{
    Json,
    extract::{Query, State},
    response::{IntoResponse, Redirect},
};
use pattern_api::error::ApiError;
use pattern_core::{
    atproto_identity::{AtprotoAuthState, AtprotoIdentity},
    db::ops::atproto::*,
    id::UserId,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tracing::{debug, error, info};

/// OAuth authorization request
#[derive(Debug, Deserialize)]
pub struct AuthorizeRequest {
    /// The PDS or handle to authenticate with
    pub identifier: String,
    /// Optional user ID to link to existing account
    pub user_id: Option<UserId>,
    /// Optional redirect URL after authentication
    pub redirect_url: Option<String>,
}

/// OAuth callback parameters from ATProto
#[derive(Debug, Deserialize)]
pub struct CallbackParams {
    pub code: String,
    pub state: String,
    pub iss: Option<String>,
}

/// ATProto profile response
#[derive(Debug, Serialize)]
pub struct AtprotoProfileResponse {
    pub did: String,
    pub handle: String,
    pub display_name: Option<String>,
    pub avatar_url: Option<String>,
}

/// Initiate ATProto OAuth flow
pub async fn authorize(
    State(state): State<Arc<AppState>>,
    Json(req): Json<AuthorizeRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    info!(
        "Initiating ATProto OAuth for identifier: {}",
        req.identifier
    );

    // Generate PKCE challenge
    let verifier = generate_code_verifier();
    let challenge = generate_code_challenge(&verifier);

    // Generate state for CSRF protection
    let state_param = generate_random_string(32);

    // Create auth state
    let auth_state = AtprotoAuthState {
        state: state_param.clone(),
        code_verifier: verifier,
        code_challenge: challenge.clone(),
        user_id: req.user_id,
        created_at: chrono::Utc::now(),
        redirect_url: req.redirect_url,
    };

    // Store auth state
    store_auth_state(&state.db, &state_param, auth_state).await?;

    // Get the OAuth client
    let oauth_client = &state.atproto_oauth_client;

    // Build authorization URL
    let auth_url = oauth_client
        .authorize(
            req.identifier,
            atrium_oauth::AuthorizeOptions {
                state: Some(state_param),
                // Add other options as needed
                ..Default::default()
            },
        )
        .await
        .map_err(|e| {
            error!("Failed to build auth URL: {:?}", e);
            ApiError::BadRequest(format!("Failed to initiate OAuth: {}", e))
        })?;

    debug!("Generated auth URL: {}", auth_url);

    Ok(Json(serde_json::json!({
        "auth_url": auth_url,
        "state": state_param
    })))
}

/// Handle OAuth callback from ATProto
pub async fn callback(
    State(state): State<Arc<AppState>>,
    Query(params): Query<CallbackParams>,
) -> Result<impl IntoResponse, ApiError> {
    info!(
        "Handling ATProto OAuth callback with state: {}",
        params.state
    );

    // Retrieve and validate auth state
    let auth_state = consume_auth_state(&state.db, &params.state)
        .await?
        .ok_or_else(|| ApiError::BadRequest("Invalid or expired state parameter".to_string()))?;

    if !auth_state.is_valid() {
        return Err(ApiError::BadRequest(
            "Authentication state expired".to_string(),
        ));
    }

    // Exchange code for tokens
    let oauth_client = &state.atproto_oauth_client;
    let callback_params = atrium_oauth::CallbackParams {
        code: params.code,
        state: Some(params.state),
        iss: params.iss,
    };

    let (session, _state) = oauth_client.callback(callback_params).await.map_err(|e| {
        error!("OAuth callback failed: {:?}", e);
        ApiError::BadRequest(format!("OAuth callback failed: {}", e))
    })?;

    // Get the DID from the session
    let did = session.did.to_string();
    let pds_url = session.pds_url.to_string();

    // Get profile information
    let profile = fetch_atproto_profile(&session).await?;

    // Determine the user ID
    let user_id = if let Some(existing_user_id) = auth_state.user_id {
        // Linking to existing user
        existing_user_id
    } else {
        // Check if this DID is already linked
        if let Some(existing_identity) = get_atproto_identity_by_did(&state.db, &did).await? {
            existing_identity.user_id
        } else {
            // Create new user
            create_user_from_atproto(&state.db, &profile).await?
        }
    };

    // Create or update the ATProto identity
    let mut identity = AtprotoIdentity::new(
        did.clone(),
        profile.handle,
        pds_url,
        session.access_token,
        session.refresh_token,
        session
            .expires_at
            .unwrap_or(chrono::Utc::now() + chrono::Duration::hours(1)),
        user_id.clone(),
    );

    identity.display_name = profile.display_name;
    identity.avatar_url = profile.avatar_url;

    let identity = upsert_atproto_identity(&state.db, identity).await?;

    // Generate JWT for the user
    let claims = Claims {
        sub: user_id.to_string(),
        did: Some(did),
        exp: (chrono::Utc::now() + chrono::Duration::hours(24)).timestamp() as usize,
    };

    let token = crate::auth::encode_jwt(&claims, &state.jwt_secret)?;

    // Redirect to the requested URL or default
    let redirect_url = auth_state.redirect_url.unwrap_or_else(|| "/".to_string());

    // Add token to redirect URL
    let redirect_with_token = if redirect_url.contains('?') {
        format!("{}&token={}", redirect_url, token)
    } else {
        format!("{}?token={}", redirect_url, token)
    };

    info!("ATProto OAuth successful for user: {}", user_id);
    Ok(Redirect::to(&redirect_with_token))
}

/// Get current user's ATProto identities
pub async fn get_identities(
    State(state): State<Arc<AppState>>,
    claims: Claims,
) -> Result<Json<Vec<AtprotoProfileResponse>>, ApiError> {
    let user_id = UserId::from_string(claims.sub);

    let identities = get_user_atproto_identities(&state.db, &user_id).await?;

    let profiles: Vec<AtprotoProfileResponse> = identities
        .into_iter()
        .map(|identity| AtprotoProfileResponse {
            did: identity.did,
            handle: identity.handle,
            display_name: identity.display_name,
            avatar_url: identity.avatar_url,
        })
        .collect();

    Ok(Json(profiles))
}

/// Unlink an ATProto identity
pub async fn unlink_identity(
    State(state): State<Arc<AppState>>,
    claims: Claims,
    Json(did): Json<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let user_id = UserId::from_string(claims.sub);

    delete_atproto_identity(&state.db, &did, &user_id).await?;

    Ok(Json(serde_json::json!({
        "success": true,
        "message": "ATProto identity unlinked"
    })))
}

/// App password authentication request
#[derive(Debug, Deserialize)]
pub struct AppPasswordAuthRequest {
    /// The handle or DID to authenticate as
    pub identifier: String,
    /// The app password
    pub app_password: String,
    /// Optional user ID to link to existing account
    pub user_id: Option<UserId>,
}

/// Authenticate with app password
pub async fn authenticate_app_password(
    State(state): State<Arc<AppState>>,
    Json(req): Json<AppPasswordAuthRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    info!(
        "App password authentication for identifier: {}",
        req.identifier
    );

    // Create a basic session to fetch profile
    // In production, you'd use atrium-api to create a proper session
    let client =
        atrium_api::client::AtpServiceClient::new(atrium_api::client::AtpServiceWrapper::new(
            atrium_xrpc::client::reqwest::ReqwestClient::new("https://bsky.social"),
        ));

    // Authenticate with app password
    let session = client
        .com
        .atproto
        .server
        .create_session(
            atrium_api::com::atproto::server::create_session::InputData {
                identifier: req.identifier.clone(),
                password: req.app_password.clone(),
                auth_factor_token: None,
            }
            .into(),
        )
        .await
        .map_err(|e| {
            error!("App password authentication failed: {:?}", e);
            ApiError::Unauthorized("Invalid identifier or app password".to_string())
        })?;

    let did = session.data.did.to_string();
    let handle = session.data.handle.to_string();
    let pds_url = session
        .data
        .did_doc
        .as_ref()
        .and_then(|doc| doc.service.as_ref())
        .and_then(|services| services.first())
        .and_then(|service| service.service_endpoint.as_ref())
        .map(|endpoint| endpoint.to_string())
        .unwrap_or_else(|| "https://bsky.social".to_string());

    // Determine the user ID
    let user_id = if let Some(existing_user_id) = req.user_id {
        // Linking to existing user
        existing_user_id
    } else {
        // Check if this DID is already linked
        if let Some(existing_identity) = get_atproto_identity_by_did(&state.db, &did).await? {
            existing_identity.user_id
        } else {
            // Create new user
            let profile = pattern_core::atproto_identity::AtprotoProfile {
                did: did.clone(),
                handle: handle.clone(),
                display_name: session.data.display_name,
                description: None,
                avatar_url: session.data.avatar,
                indexed_at: None,
            };
            create_user_from_atproto(&state.db, &profile).await?
        }
    };

    // Create or update the ATProto identity with app password
    let identity = AtprotoIdentity::new_app_password(
        did.clone(),
        handle,
        pds_url,
        req.app_password,
        user_id.clone(),
    );

    let identity = upsert_atproto_identity(&state.db, identity).await?;

    // Generate JWT for the user
    let claims = Claims {
        sub: user_id.to_string(),
        did: Some(did),
        exp: (chrono::Utc::now() + chrono::Duration::hours(24)).timestamp() as usize,
    };

    let token = crate::auth::encode_jwt(&claims, &state.jwt_secret)?;

    info!(
        "App password authentication successful for user: {}",
        user_id
    );

    Ok(Json(serde_json::json!({
        "success": true,
        "token": token,
        "user_id": user_id.to_string(),
        "did": identity.did,
        "handle": identity.handle,
    })))
}

// Helper functions

fn generate_code_verifier() -> String {
    use rand::Rng;
    // let verifier_bytes: Vec<u8> = (0..32)
    //     .map(|_| rand::thread_rng().gen::<u8>())
    //     .collect();
    base64_url_encode(&verifier_bytes)
}

fn generate_code_challenge(verifier: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(verifier.as_bytes());
    let result = hasher.finalize();
    base64_url_encode(&result)
}

fn generate_random_string(len: usize) -> String {
    use rand::Rng;
    let chars: Vec<char> = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789"
        .chars()
        .collect();
    let mut rng = rand::thread_rng();
    (0..len)
        .map(|_| chars[rng.gen_range(0..chars.len())])
        .collect()
}

fn base64_url_encode(data: &[u8]) -> String {
    base64::encode_config(data, base64::URL_SAFE_NO_PAD)
}

async fn fetch_atproto_profile(
    session: &atrium_oauth::OAuthSession<
        impl atrium_oauth::HttpClient,
        impl atrium_oauth::DidResolver,
        impl atrium_oauth::HandleResolver,
        impl atrium_oauth::SessionStore,
    >,
) -> Result<pattern_core::atproto_identity::AtprotoProfile, ApiError> {
    // TODO: Use the session to fetch profile from PDS
    // For now, return basic info from session
    Ok(pattern_core::atproto_identity::AtprotoProfile {
        did: session.did.to_string(),
        handle: session
            .handle
            .as_ref()
            .map(|h| h.to_string())
            .unwrap_or_default(),
        display_name: None,
        description: None,
        avatar_url: None,
        indexed_at: None,
    })
}

async fn create_user_from_atproto(
    db: &pattern_core::db::Db,
    profile: &pattern_core::atproto_identity::AtprotoProfile,
) -> Result<UserId, ApiError> {
    // Create a new Pattern user from ATProto profile
    let user = pattern_core::users::User {
        id: UserId::generate(),
        username: profile.handle.clone(),
        display_name: profile.display_name.clone(),
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    };

    let created = pattern_core::db::ops::create_entity(db, user).await?;
    Ok(created.id)
}
