//! Pattern API Server library
//!
//! Core server implementation for Pattern's HTTP/WebSocket APIs

pub mod auth;
pub mod config;
pub mod error;
pub mod handlers;
pub mod middleware;
pub mod models;
pub mod state;

pub use config::ServerConfig;
pub use error::{ServerError, ServerResult};
pub use state::AppState;

/// Start the Pattern API server
pub async fn start_server(config: ServerConfig) -> ServerResult<()> {
    use axum::Router;
    use std::net::SocketAddr;
    use tower_http::cors::CorsLayer;
    use tower_http::trace::TraceLayer;

    tracing::info!("Starting Pattern API Server on {}", config.bind_address);

    // Create app state
    let state = AppState::new(config.clone()).await?;

    // Build router
    let app = Router::new()
        .nest("/api/v1", handlers::routes())
        .layer(CorsLayer::permissive()) // TODO: Configure properly
        .layer(TraceLayer::new_for_http())
        .with_state(state);

    // Parse address
    let addr: SocketAddr = config.bind_address.parse()?;

    // Start server
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}
