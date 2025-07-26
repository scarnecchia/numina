//! Authentication utilities

use argon2::{
    Argon2,
    password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
};
use jsonwebtoken::{DecodingKey, EncodingKey, Header, Validation, decode, encode};
use pattern_api::{AccessTokenClaims, RefreshTokenClaims};
use pattern_core::id::UserId;

use crate::error::ServerResult;

//pub mod atproto;

/// Hash a plaintext password
pub fn hash_password(password: &str) -> ServerResult<String> {
    let salt = SaltString::generate(&mut rand::thread_rng());
    let argon2 = Argon2::default();

    let password_hash = argon2
        .hash_password(password.as_bytes(), &salt)?
        .to_string();

    Ok(password_hash)
}

/// Verify a password against a hash
pub fn verify_password(password: &str, hash: &str) -> ServerResult<bool> {
    let parsed_hash = PasswordHash::new(hash)?;
    let argon2 = Argon2::default();

    Ok(argon2
        .verify_password(password.as_bytes(), &parsed_hash)
        .is_ok())
}

/// Generate an access token
pub fn generate_access_token(
    user_id: UserId,
    encoding_key: &EncodingKey,
    ttl_seconds: u64,
) -> ServerResult<String> {
    let now = chrono::Utc::now().timestamp();

    let claims = AccessTokenClaims {
        sub: user_id,
        iat: now,
        exp: now + ttl_seconds as i64,
        jti: uuid::Uuid::new_v4(),
        token_type: "access".to_string(),
        permissions: None,
    };

    Ok(encode(&Header::default(), &claims, encoding_key)?)
}

/// Generate a refresh token
pub fn generate_refresh_token(
    user_id: UserId,
    family: uuid::Uuid,
    encoding_key: &EncodingKey,
    ttl_seconds: u64,
) -> ServerResult<String> {
    let now = chrono::Utc::now().timestamp();

    let claims = RefreshTokenClaims {
        sub: user_id,
        iat: now,
        exp: now + ttl_seconds as i64,
        jti: uuid::Uuid::new_v4(),
        token_type: "refresh".to_string(),
        family,
    };

    Ok(encode(&Header::default(), &claims, encoding_key)?)
}

/// Validate an access token
pub fn validate_access_token(
    token: &str,
    decoding_key: &DecodingKey,
) -> ServerResult<AccessTokenClaims> {
    let token_data = decode::<AccessTokenClaims>(token, decoding_key, &Validation::default())?;
    Ok(token_data.claims)
}

/// Validate a refresh token
pub fn validate_refresh_token(
    token: &str,
    decoding_key: &DecodingKey,
) -> ServerResult<RefreshTokenClaims> {
    let token_data = decode::<RefreshTokenClaims>(token, decoding_key, &Validation::default())?;
    Ok(token_data.claims)
}
