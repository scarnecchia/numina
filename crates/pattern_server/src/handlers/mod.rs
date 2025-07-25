//! HTTP request handlers

use axum::{
    Router,
    routing::{get, post},
};

pub mod auth;
pub mod health;

use crate::state::AppState;

/// Build all API routes
pub fn routes() -> Router<AppState> {
    use pattern_api::{ApiEndpoint, requests::*};

    Router::new()
        // Health check
        .route(HealthCheckRequest::PATH, get(health::health_check))
        // Auth endpoints
        .route(AuthRequest::PATH, post(auth::login))
        .route(RefreshTokenRequest::PATH, post(auth::refresh_token))

    // TODO: Add remaining endpoints
}
