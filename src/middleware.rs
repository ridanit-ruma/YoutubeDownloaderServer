//! Axum extractors for authenticated users and admins.
//!
//! Usage in a handler:
//! ```rust,ignore
//! async fn my_handler(AuthUser(claims): AuthUser) -> ... { ... }
//! async fn admin_only(AdminUser(claims): AdminUser) -> ... { ... }
//! ```

use axum::{
    extract::FromRequestParts,
    http::{StatusCode, request::Parts},
};

use crate::{
    auth::{Claims, verify_token},
    error::HttpError,
    state::AppState,
};

// ── Helper: extract Bearer token from Authorization header ────────────────────

fn extract_bearer(parts: &Parts) -> Result<&str, HttpError> {
    let header = parts
        .headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| HttpError::new(
            StatusCode::UNAUTHORIZED,
            "MISSING_TOKEN",
            "Authorization header is required",
        ))?;

    header.strip_prefix("Bearer ").ok_or_else(|| HttpError::new(
        StatusCode::UNAUTHORIZED,
        "INVALID_TOKEN_FORMAT",
        "Authorization header must use 'Bearer <token>' format",
    ))
}

// ── AuthUser extractor ────────────────────────────────────────────────────────

/// Extracts and validates a JWT from the `Authorization: Bearer <token>` header.
/// Succeeds for any authenticated user.
pub struct AuthUser(pub Claims);

impl FromRequestParts<AppState> for AuthUser {
    type Rejection = HttpError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let token = extract_bearer(parts)?;
        let claims = verify_token(&state.jwt, token)?;
        Ok(AuthUser(claims))
    }
}

// ── AdminUser extractor ───────────────────────────────────────────────────────

/// Like `AuthUser` but additionally requires `claims.is_admin == true`.
pub struct AdminUser(pub Claims);

impl FromRequestParts<AppState> for AdminUser {
    type Rejection = HttpError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let token = extract_bearer(parts)?;
        let claims = verify_token(&state.jwt, token)?;
        if !claims.is_admin {
            return Err(HttpError::new(
                StatusCode::FORBIDDEN,
                "FORBIDDEN",
                "admin privileges required",
            ));
        }
        Ok(AdminUser(claims))
    }
}
