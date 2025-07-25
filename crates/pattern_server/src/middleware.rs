//! Middleware for authentication, rate limiting, etc.

use axum::{
    extract::{Request, State},
    http::HeaderMap,
    middleware::Next,
    response::Response,
};
use pattern_api::ApiError;

use crate::state::AppState;

/// Extract and validate bearer token from Authorization header
pub fn extract_bearer_token(headers: &HeaderMap) -> Option<&str> {
    headers
        .get("Authorization")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
}

/// Authentication middleware
pub async fn require_auth(
    State(state): State<AppState>,
    headers: HeaderMap,
    mut request: Request,
    next: Next,
) -> Result<Response, ApiError> {
    let token = extract_bearer_token(&headers).ok_or_else(|| ApiError::Unauthorized {
        message: Some("Missing authorization header".to_string()),
    })?;

    // Validate token
    let claims =
        crate::auth::validate_access_token(token, &state.jwt_decoding_key).map_err(|_| {
            ApiError::Unauthorized {
                message: Some("Invalid or expired token".to_string()),
            }
        })?;

    // Insert user ID into request extensions for handlers to use
    request.extensions_mut().insert(claims.sub);

    Ok(next.run(request).await)
}
