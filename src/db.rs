//! Database access layer — user repository.

use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

use crate::error::AppError;

// ── Model ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct User {
    pub id:                    Uuid,
    pub username:              String,
    pub password_hash:         String,
    pub is_admin:              bool,
    pub require_password_reset: bool,
    pub created_at:            DateTime<Utc>,
    pub updated_at:            DateTime<Utc>,
}

// ── Repository ────────────────────────────────────────────────────────────────

/// Count the total number of users in the database.
pub async fn count_users(pool: &PgPool) -> Result<i64, AppError> {
    let row = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM users")
        .fetch_one(pool)
        .await?;
    Ok(row)
}

/// Insert a new user and return the created record.
pub async fn create_user(
    pool:                  &PgPool,
    username:              &str,
    password_hash:         &str,
    is_admin:              bool,
    require_password_reset: bool,
) -> Result<User, AppError> {
    let user = sqlx::query_as::<_, User>(
        r#"
        INSERT INTO users (username, password_hash, is_admin, require_password_reset)
        VALUES ($1, $2, $3, $4)
        RETURNING *
        "#,
    )
    .bind(username)
    .bind(password_hash)
    .bind(is_admin)
    .bind(require_password_reset)
    .fetch_one(pool)
    .await?;
    Ok(user)
}

/// Find a user by username.
pub async fn find_user_by_username(pool: &PgPool, username: &str) -> Result<Option<User>, AppError> {
    let user = sqlx::query_as::<_, User>("SELECT * FROM users WHERE username = $1")
        .bind(username)
        .fetch_optional(pool)
        .await?;
    Ok(user)
}

/// Find a user by UUID.
pub async fn find_user_by_id(pool: &PgPool, id: Uuid) -> Result<Option<User>, AppError> {
    let user = sqlx::query_as::<_, User>("SELECT * FROM users WHERE id = $1")
        .bind(id)
        .fetch_optional(pool)
        .await?;
    Ok(user)
}

/// Return all users (admin use).
pub async fn list_users(pool: &PgPool) -> Result<Vec<User>, AppError> {
    let users = sqlx::query_as::<_, User>(
        "SELECT * FROM users ORDER BY created_at ASC",
    )
    .fetch_all(pool)
    .await?;
    Ok(users)
}

/// Update the password hash and clear `require_password_reset`.
pub async fn update_password(
    pool:          &PgPool,
    user_id:       Uuid,
    password_hash: &str,
) -> Result<(), AppError> {
    sqlx::query(
        r#"
        UPDATE users
        SET password_hash = $1, require_password_reset = FALSE, updated_at = NOW()
        WHERE id = $2
        "#,
    )
    .bind(password_hash)
    .bind(user_id)
    .execute(pool)
    .await?;
    Ok(())
}

/// Delete a user by UUID. Returns true when a row was deleted.
pub async fn delete_user(pool: &PgPool, id: Uuid) -> Result<bool, AppError> {
    let result = sqlx::query("DELETE FROM users WHERE id = $1")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(result.rows_affected() > 0)
}

/// Set `is_admin` flag for a user.
pub async fn set_admin(pool: &PgPool, id: Uuid, is_admin: bool) -> Result<(), AppError> {
    sqlx::query("UPDATE users SET is_admin = $1, updated_at = NOW() WHERE id = $2")
        .bind(is_admin)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Set `require_password_reset` flag for a user.
pub async fn set_require_password_reset(
    pool:  &PgPool,
    id:    Uuid,
    value: bool,
) -> Result<(), AppError> {
    sqlx::query(
        "UPDATE users SET require_password_reset = $1, updated_at = NOW() WHERE id = $2",
    )
    .bind(value)
    .bind(id)
    .execute(pool)
    .await?;
    Ok(())
}
