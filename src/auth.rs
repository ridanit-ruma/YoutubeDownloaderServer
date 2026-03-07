//! JWT token creation / validation and Argon2 password hashing.

use argon2::{
    Argon2,
    password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString, rand_core::OsRng},
};
use chrono::Utc;
use jsonwebtoken::{DecodingKey, EncodingKey, Header, Validation, decode, encode};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::{AppError, HttpError};

// ── JWT ──────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Claims {
    /// Subject: user UUID
    pub sub: Uuid,
    /// Username
    pub username: String,
    /// Whether this user is an admin
    pub is_admin: bool,
    /// Expiry timestamp (UNIX seconds)
    pub exp: u64,
    /// Issued-at timestamp (UNIX seconds)
    pub iat: u64,
}

/// Shared JWT configuration derived from `AppConfig`.
#[derive(Clone)]
pub struct JwtConfig {
    pub encoding_key: EncodingKey,
    pub decoding_key: DecodingKey,
    pub expiry_secs:  u64,
}

impl JwtConfig {
    pub fn new(secret: &str, expiry_secs: u64) -> Self {
        Self {
            encoding_key: EncodingKey::from_secret(secret.as_bytes()),
            decoding_key: DecodingKey::from_secret(secret.as_bytes()),
            expiry_secs,
        }
    }
}

/// Mint a new signed JWT for the given user.
pub fn create_token(
    cfg:      &JwtConfig,
    user_id:  Uuid,
    username: &str,
    is_admin: bool,
) -> Result<String, AppError> {
    let now = Utc::now().timestamp() as u64;
    let claims = Claims {
        sub:      user_id,
        username: username.to_owned(),
        is_admin,
        iat:      now,
        exp:      now + cfg.expiry_secs,
    };
    encode(&Header::default(), &claims, &cfg.encoding_key)
        .map_err(|e| AppError::internal(format!("JWT signing failed: {e}")))
}

/// Validate a JWT and return its claims.
pub fn verify_token(cfg: &JwtConfig, token: &str) -> Result<Claims, HttpError> {
    let mut validation = Validation::default();
    validation.validate_exp = true;
    decode::<Claims>(token, &cfg.decoding_key, &validation)
        .map(|td| td.claims)
        .map_err(|e| HttpError::new(
            axum::http::StatusCode::UNAUTHORIZED,
            "INVALID_TOKEN",
            format!("invalid or expired token: {e}"),
        ))
}

// ── Argon2 password hashing ───────────────────────────────────────────────────

/// Hash a plaintext password using Argon2id.
pub fn hash_password(password: &str) -> Result<String, AppError> {
    let salt = SaltString::generate(&mut OsRng);
    let argon2 = Argon2::default();
    argon2
        .hash_password(password.as_bytes(), &salt)
        .map(|h| h.to_string())
        .map_err(|e| AppError::internal(format!("password hashing failed: {e}")))
}

/// Verify a plaintext password against a stored Argon2 hash.
/// Returns `Ok(true)` when it matches, `Ok(false)` when it doesn't.
pub fn verify_password(password: &str, hash: &str) -> Result<bool, AppError> {
    let parsed = PasswordHash::new(hash)
        .map_err(|e| AppError::internal(format!("invalid password hash in DB: {e}")))?;
    Ok(Argon2::default()
        .verify_password(password.as_bytes(), &parsed)
        .is_ok())
}

/// Generate a cryptographically random password of the given length.
pub fn generate_random_password(len: usize) -> String {
    use rand::Rng as _;
    const CHARSET: &[u8] =
        b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789!@#$%^&*";
    let mut rng = rand::thread_rng();
    (0..len)
        .map(|_| {
            let idx = rng.gen_range(0..CHARSET.len());
            CHARSET[idx] as char
        })
        .collect()
}
