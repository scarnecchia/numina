//! Server configuration

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    /// Server bind address (e.g., "127.0.0.1:8080")
    pub bind_address: String,

    /// Database URL
    pub database_url: String,

    /// JWT secret for signing tokens
    pub jwt_secret: String,

    /// Token expiration times
    pub access_token_ttl: u64, // seconds
    pub refresh_token_ttl: u64, // seconds

    /// CORS configuration
    pub cors: CorsConfig,

    /// Rate limiting
    pub rate_limit: RateLimitConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CorsConfig {
    pub allowed_origins: Vec<String>,
    pub allowed_methods: Vec<String>,
    pub allowed_headers: Vec<String>,
    pub max_age: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RateLimitConfig {
    pub enabled: bool,
    pub requests_per_minute: u32,
    pub burst_size: u32,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            bind_address: "127.0.0.1:8080".to_string(),
            database_url: "mem://".to_string(),
            jwt_secret: "change-me-in-production".to_string(),
            access_token_ttl: 3600,    // 1 hour
            refresh_token_ttl: 604800, // 7 days
            cors: CorsConfig {
                allowed_origins: vec!["*".to_string()],
                allowed_methods: vec![
                    "GET".to_string(),
                    "POST".to_string(),
                    "PUT".to_string(),
                    "PATCH".to_string(),
                    "DELETE".to_string(),
                ],
                allowed_headers: vec!["*".to_string()],
                max_age: 3600,
            },
            rate_limit: RateLimitConfig {
                enabled: false,
                requests_per_minute: 60,
                burst_size: 10,
            },
        }
    }
}
