//! Health check endpoint

use axum::Json;
use pattern_api::responses::{ComponentStatus, HealthResponse, HealthStatus};

pub async fn health_check() -> Json<HealthResponse> {
    Json(HealthResponse {
        status: HealthStatus::Healthy,
        version: pattern_api::API_VERSION.to_string(),
        uptime_seconds: 0, // TODO: Track actual uptime
        database_status: ComponentStatus::Ok,
        services: vec![],
    })
}
