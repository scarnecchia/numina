//! Application state

use crate::{config::ServerConfig, error::ServerResult};
use surrealdb::{Surreal, engine::any::Any};

#[derive(Clone)]
pub struct AppState {
    pub config: ServerConfig,
    pub db: Surreal<Any>,
    pub jwt_encoding_key: jsonwebtoken::EncodingKey,
    pub jwt_decoding_key: jsonwebtoken::DecodingKey,
}

impl AppState {
    pub async fn new(config: ServerConfig) -> ServerResult<Self> {
        // Connect to database
        let db = surrealdb::engine::any::connect(&config.database_url)
            .await
            .map_err(|e| {
                crate::error::ServerError::Database(
                    pattern_core::db::DatabaseError::ConnectionFailed(e),
                )
            })?;

        // Use default namespace and database
        db.use_ns("pattern").use_db("main").await.map_err(|e| {
            crate::error::ServerError::Database(pattern_core::db::DatabaseError::ConnectionFailed(
                e,
            ))
        })?;

        // Create JWT keys
        let jwt_encoding_key = jsonwebtoken::EncodingKey::from_secret(config.jwt_secret.as_bytes());
        let jwt_decoding_key = jsonwebtoken::DecodingKey::from_secret(config.jwt_secret.as_bytes());

        Ok(Self {
            config,
            db,
            jwt_encoding_key,
            jwt_decoding_key,
        })
    }
}
