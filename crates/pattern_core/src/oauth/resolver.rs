//! Custom resolvers for genai integration with OAuth
//!
//! Provides AuthResolver and ServiceTargetResolver implementations that
//! integrate with Pattern's OAuth token storage.

use crate::db::DbEntity;
use crate::error::CoreError;
use crate::id::UserId;
use crate::oauth::OAuthToken;
use genai::ModelIden;
use genai::ServiceTarget;
use genai::adapter::AdapterKind;
use genai::resolver::{AuthData, AuthResolver, Result as ResolverResult, ServiceTargetResolver};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use surrealdb::RecordId;
use surrealdb::{Connection, Surreal};

/// Create an OAuth-aware auth resolver for Pattern
pub fn create_oauth_auth_resolver<C: Connection + 'static>(
    db: Arc<Surreal<C>>,
    user_id: UserId,
) -> AuthResolver {
    let resolver_fn = move |model_iden: ModelIden| -> Pin<
        Box<dyn Future<Output = ResolverResult<Option<AuthData>>> + Send>,
    > {
        let db = db.clone();
        let user_id = user_id.clone();

        Box::pin(async move {
            // Extract adapter kind from model identifier
            let adapter_kind = model_iden.adapter_kind;

            // Only handle Anthropic OAuth for now
            if adapter_kind == AdapterKind::Anthropic {
                tracing::debug!("Checking OAuth token for Anthropic, user: {}", user_id);
                // Use OAuthModelProvider to handle token refresh automatically
                let provider =
                    crate::oauth::integration::OAuthModelProvider::new(db.clone(), user_id.clone());

                match provider.get_token("anthropic").await {
                    Ok(Some(token)) => {
                        // Token is automatically refreshed if needed by get_token()
                        tracing::debug!(
                            "Using OAuth token for Anthropic (expires: {})",
                            token.expires_at
                        );

                        // Return bearer token with "Bearer " prefix so genai detects OAuth
                        return Ok(Some(AuthData::Key(format!(
                            "Bearer {}",
                            token.access_token
                        ))));
                    }
                    Ok(None) => {
                        // No OAuth token found
                        // Check if API key is available as fallback
                        if std::env::var("ANTHROPIC_API_KEY").is_ok() {
                            tracing::debug!(
                                "No OAuth token found for Anthropic, falling back to API key"
                            );
                            // Return None to use default auth (API key)
                            return Ok(None);
                        } else {
                            // No API key either, return OAuth required error
                            tracing::warn!(
                                "Neither OAuth token nor API key available for Anthropic"
                            );
                            return Err(genai::resolver::Error::Custom(
                                "Authentication required for Anthropic. Please either:\n1. Run 'pattern-cli auth login anthropic' to use OAuth, or\n2. Set ANTHROPIC_API_KEY environment variable".to_string()
                            ));
                        }
                    }
                    Err(e) => {
                        // Log error but don't fail - try API key as fallback
                        tracing::error!("Error loading OAuth token: {}", e);
                    }
                }
            }

            // Fall back to None to let genai use its default resolution
            Ok(None)
        })
    };

    AuthResolver::from_resolver_async_fn(resolver_fn)
}

/// Create a default service target resolver
pub fn create_service_target_resolver() -> ServiceTargetResolver {
    let resolver_fn = move |service_target: ServiceTarget| -> Pin<
        Box<dyn Future<Output = ResolverResult<ServiceTarget>> + Send>,
    > {
        Box::pin(async move {
            // For now, just return the service target as-is
            // In the future, we might want to use different endpoints for OAuth
            Ok(service_target)
        })
    };

    ServiceTargetResolver::from_resolver_async_fn(resolver_fn)
}

/// Load OAuth token for a provider from the database
#[allow(dead_code)]
async fn get_oauth_token<C: Connection>(
    db: &Surreal<C>,
    user_id: &UserId,
    provider: &str,
) -> Result<Option<OAuthToken>, CoreError> {
    // Query for OAuth token belonging to this user and provider
    let query = r#"
        SELECT * FROM oauth_token
        WHERE owner_id = $user_id
        AND provider = $provider
        ORDER BY last_used_at DESC
        LIMIT 1
    "#;

    // Clone to avoid borrow issues
    let user_id = user_id.clone();
    let provider = provider.to_string();

    let mut result = db
        .query(query)
        .bind(("user_id", RecordId::from(user_id.clone())))
        .bind(("provider", provider.clone()))
        .await
        .map_err(|e| CoreError::DatabaseQueryFailed {
            query: query.to_string(),
            table: "oauth_token".to_string(),
            cause: e,
        })?;

    let tokens: Vec<<OAuthToken as DbEntity>::DbModel> =
        result.take(0).map_err(|e| CoreError::DatabaseQueryFailed {
            query: query.to_string(),
            table: "oauth_token".to_string(),
            cause: e,
        })?;

    let mut tokens = tokens
        .into_iter()
        .map(|tok| OAuthToken::from_db_model(tok).expect("should be the db model"));

    let token = tokens.next();

    if token.is_none() {
        tracing::info!(
            "No OAuth token found for user {} provider {}",
            user_id,
            provider
        );
    } else {
        tracing::info!(
            "Found OAuth token for user {} provider {}",
            user_id,
            provider
        );
    }

    Ok(token)
}

/// Builder for creating a genai client with OAuth support
pub struct OAuthClientBuilder<C: Connection> {
    db: Arc<Surreal<C>>,
    user_id: UserId,
    base_url: Option<String>,
}

impl<C: Connection + 'static> OAuthClientBuilder<C> {
    /// Create a new builder
    pub fn new(db: Arc<Surreal<C>>, user_id: UserId) -> Self {
        Self {
            db,
            user_id,
            base_url: None,
        }
    }

    /// Set a custom base URL for the API
    pub fn with_base_url(mut self, url: String) -> Self {
        self.base_url = Some(url);
        self
    }

    /// Build a genai client with OAuth support
    pub fn build(self) -> Result<genai::Client, CoreError> {
        // Create our OAuth-aware auth resolver
        let auth_resolver = create_oauth_auth_resolver(self.db.clone(), self.user_id.clone());

        // Create service target resolver
        let target_resolver = create_service_target_resolver();

        // Build the client
        let client = genai::Client::builder()
            .with_auth_resolver(auth_resolver)
            .with_service_target_resolver(target_resolver)
            .build();

        Ok(client)
    }
}
