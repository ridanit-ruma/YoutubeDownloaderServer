//! Admin-only routes for user management.

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::{
    auth::{generate_random_password, hash_password},
    db,
    error::{AppError, HttpError},
    middleware::AdminUser,
    state::AppState,
};

// ── Request / Response types ──────────────────────────────────────────────────

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateUserRequest {
    pub username:               String,
    /// If omitted a random password is generated.
    pub password:               Option<String>,
    #[serde(default)]
    pub is_admin:               bool,
    /// Whether the user must change their password on first login.
    #[serde(default = "default_true")]
    pub require_password_reset: bool,
}

fn default_true() -> bool { true }

#[derive(Debug, Serialize, ToSchema)]
pub struct CreateUserResponse {
    pub id:                    Uuid,
    pub username:              String,
    pub is_admin:              bool,
    pub require_password_reset: bool,
    /// Only present when the server generated the password.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub generated_password:    Option<String>,
    pub created_at:            DateTime<Utc>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct UserSummary {
    pub id:                    Uuid,
    pub username:              String,
    pub is_admin:              bool,
    pub require_password_reset: bool,
    pub created_at:            DateTime<Utc>,
    pub updated_at:            DateTime<Utc>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct MessageResponse {
    pub message: String,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct SetAdminRequest {
    pub is_admin: bool,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct SetRequirePasswordResetRequest {
    pub require_password_reset: bool,
}

// ── Handlers ──────────────────────────────────────────────────────────────────

/// List all users. Admin only.
#[utoipa::path(
    get,
    path = "/admin/users",
    tag  = "admin",
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "List of users", body = Vec<UserSummary>),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
    )
)]
pub async fn admin_list_users(
    State(state): State<AppState>,
    admin:        AdminUser,
) -> Result<Json<Vec<UserSummary>>, AppError> {
    let _ = admin;
    let users = db::list_users(&state.db).await?;
    let summaries = users
        .into_iter()
        .map(|u| UserSummary {
            id:                    u.id,
            username:              u.username,
            is_admin:              u.is_admin,
            require_password_reset: u.require_password_reset,
            created_at:            u.created_at,
            updated_at:            u.updated_at,
        })
        .collect();
    Ok(Json(summaries))
}

/// Create a new user. Admin only.
#[utoipa::path(
    post,
    path = "/admin/users",
    tag  = "admin",
    security(("bearer_auth" = [])),
    request_body = CreateUserRequest,
    responses(
        (status = 201, description = "User created",  body = CreateUserResponse),
        (status = 409, description = "Username taken"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
    )
)]
pub async fn admin_create_user(
    State(state): State<AppState>,
    admin:        AdminUser,
    Json(body):   Json<CreateUserRequest>,
) -> Result<(StatusCode, Json<CreateUserResponse>), AppError> {
    let _ = admin;

    if body.username.trim().is_empty() {
        return Err(AppError::from_http(HttpError::bad_request("username cannot be empty")));
    }

    if db::find_user_by_username(&state.db, &body.username).await?.is_some() {
        return Err(AppError::from_http(HttpError::new(
            StatusCode::CONFLICT,
            "USERNAME_TAKEN",
            format!("username '{}' is already taken", body.username),
        )));
    }

    let (plain_password, generated_password) = match body.password {
        Some(p) => {
            if p.len() < 8 {
                return Err(AppError::from_http(HttpError::bad_request(
                    "password must be at least 8 characters",
                )));
            }
            (p, None)
        }
        None => {
            let p = generate_random_password(20);
            (p.clone(), Some(p))
        }
    };

    let hash = hash_password(&plain_password)?;
    let user = db::create_user(
        &state.db,
        &body.username,
        &hash,
        body.is_admin,
        body.require_password_reset,
    )
    .await?;

    let resp = CreateUserResponse {
        id:                    user.id,
        username:              user.username,
        is_admin:              user.is_admin,
        require_password_reset: user.require_password_reset,
        generated_password,
        created_at:            user.created_at,
    };
    Ok((StatusCode::CREATED, Json(resp)))
}

/// Delete a user by ID. Admin only. Cannot delete yourself.
#[utoipa::path(
    delete,
    path = "/admin/users/{id}",
    tag  = "admin",
    security(("bearer_auth" = [])),
    params(("id" = Uuid, Path, description = "User UUID")),
    responses(
        (status = 200, description = "User deleted",  body = MessageResponse),
        (status = 404, description = "User not found"),
        (status = 400, description = "Cannot delete yourself"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
    )
)]
pub async fn admin_delete_user(
    State(state): State<AppState>,
    admin:        AdminUser,
    Path(id):     Path<Uuid>,
) -> Result<Json<MessageResponse>, AppError> {
    if id == admin.0.sub {
        return Err(AppError::from_http(HttpError::bad_request(
            "you cannot delete your own account",
        )));
    }

    let deleted = db::delete_user(&state.db, id).await?;
    if !deleted {
        return Err(AppError::from_http(HttpError::not_found(
            format!("user {id} not found"),
        )));
    }

    Ok(Json(MessageResponse { message: format!("user {id} deleted") }))
}

/// Set admin flag for a user. Admin only.
#[utoipa::path(
    patch,
    path = "/admin/users/{id}/admin",
    tag  = "admin",
    security(("bearer_auth" = [])),
    params(("id" = Uuid, Path, description = "User UUID")),
    request_body = SetAdminRequest,
    responses(
        (status = 200, description = "Updated",       body = MessageResponse),
        (status = 404, description = "User not found"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
    )
)]
pub async fn admin_set_admin(
    State(state): State<AppState>,
    admin:        AdminUser,
    Path(id):     Path<Uuid>,
    Json(body):   Json<SetAdminRequest>,
) -> Result<Json<MessageResponse>, AppError> {
    let _ = admin;
    if db::find_user_by_id(&state.db, id).await?.is_none() {
        return Err(AppError::from_http(HttpError::not_found(format!("user {id} not found"))));
    }
    db::set_admin(&state.db, id, body.is_admin).await?;
    Ok(Json(MessageResponse { message: "updated".to_owned() }))
}

/// Set require_password_reset flag for a user. Admin only.
#[utoipa::path(
    patch,
    path = "/admin/users/{id}/require-password-reset",
    tag  = "admin",
    security(("bearer_auth" = [])),
    params(("id" = Uuid, Path, description = "User UUID")),
    request_body = SetRequirePasswordResetRequest,
    responses(
        (status = 200, description = "Updated",       body = MessageResponse),
        (status = 404, description = "User not found"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
    )
)]
pub async fn admin_set_require_password_reset(
    State(state): State<AppState>,
    admin:        AdminUser,
    Path(id):     Path<Uuid>,
    Json(body):   Json<SetRequirePasswordResetRequest>,
) -> Result<Json<MessageResponse>, AppError> {
    let _ = admin;
    if db::find_user_by_id(&state.db, id).await?.is_none() {
        return Err(AppError::from_http(HttpError::not_found(format!("user {id} not found"))));
    }
    db::set_require_password_reset(&state.db, id, body.require_password_reset).await?;
    Ok(Json(MessageResponse { message: "updated".to_owned() }))
}
