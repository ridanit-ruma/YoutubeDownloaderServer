//! Authentication routes: login and password change.

use axum::{Json, extract::State};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::{
    auth::{create_token, hash_password, verify_password},
    db,
    error::{AppError, HttpError},
    middleware::AuthUser,
    state::AppState,
};

// ── Request / Response types ──────────────────────────────────────────────────

#[derive(Debug, Deserialize, ToSchema)]
pub struct LoginRequest {
    pub username: String,
    pub password: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct LoginResponse {
    pub token:                  String,
    pub require_password_reset: bool,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct ChangePasswordRequest {
    pub current_password: String,
    pub new_password:     String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct MessageResponse {
    pub message: String,
}

// ── Handlers ──────────────────────────────────────────────────────────────────

/// Login with username and password.
/// Returns a JWT access token.
#[utoipa::path(
    post,
    path = "/auth/login",
    tag  = "auth",
    request_body = LoginRequest,
    responses(
        (status = 200, description = "Login successful",            body = LoginResponse),
        (status = 401, description = "Invalid credentials"),
        (status = 400, description = "Bad request"),
    )
)]
pub async fn login(
    State(state): State<AppState>,
    Json(body):   Json<LoginRequest>,
) -> Result<Json<LoginResponse>, AppError> {
    let user = db::find_user_by_username(&state.db, &body.username)
        .await?
        .ok_or_else(|| AppError::from_http(HttpError::new(
            axum::http::StatusCode::UNAUTHORIZED,
            "INVALID_CREDENTIALS",
            "invalid username or password",
        )))?;

    if !verify_password(&body.password, &user.password_hash)? {
        return Err(AppError::from_http(HttpError::new(
            axum::http::StatusCode::UNAUTHORIZED,
            "INVALID_CREDENTIALS",
            "invalid username or password",
        )));
    }

    let token = create_token(&state.jwt, user.id, &user.username, user.is_admin)?;

    Ok(Json(LoginResponse {
        token,
        require_password_reset: user.require_password_reset,
    }))
}

/// Change the currently authenticated user's password.
#[utoipa::path(
    post,
    path = "/auth/change-password",
    tag  = "auth",
    security(("bearer_auth" = [])),
    request_body = ChangePasswordRequest,
    responses(
        (status = 200, description = "Password changed successfully", body = MessageResponse),
        (status = 401, description = "Unauthorized"),
        (status = 400, description = "Current password is incorrect"),
    )
)]
pub async fn change_password(
    State(state):     State<AppState>,
    AuthUser(claims): AuthUser,
    Json(body):       Json<ChangePasswordRequest>,
) -> Result<Json<MessageResponse>, AppError> {
    if body.new_password.len() < 8 {
        return Err(AppError::from_http(HttpError::bad_request(
            "new password must be at least 8 characters",
        )));
    }

    let user = db::find_user_by_id(&state.db, claims.sub)
        .await?
        .ok_or_else(|| AppError::internal("authenticated user not found in DB"))?;

    if !verify_password(&body.current_password, &user.password_hash)? {
        return Err(AppError::from_http(HttpError::bad_request(
            "current password is incorrect",
        )));
    }

    let new_hash = hash_password(&body.new_password)?;
    db::update_password(&state.db, user.id, &new_hash).await?;

    Ok(Json(MessageResponse {
        message: "password changed successfully".to_owned(),
    }))
}
