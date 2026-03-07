//! Integration tests for auth and admin API endpoints.
//!
//! Spins up a real PostgreSQL container via testcontainers, runs sqlx
//! migrations, and exercises the axum router with `tower::ServiceExt::oneshot`.
//!
//! Requirements:
//! - Docker (or compatible) daemon must be running on the host.
//! - Run: `cargo test --test integration`

use axum::{
    body::Body,
    http::{Request, StatusCode, header},
};
use serde_json::{Value, json};
use sqlx::postgres::PgPoolOptions;
use testcontainers::ContainerAsync;
use testcontainers_modules::{postgres::Postgres, testcontainers::runners::AsyncRunner};
use tower::ServiceExt as _;

// ── DB setup ──────────────────────────────────────────────────────────────────

async fn setup_db() -> (sqlx::PgPool, ContainerAsync<Postgres>) {
    let container = Postgres::default()
        .start()
        .await
        .expect("failed to start Postgres container");

    let host = container.get_host().await.expect("get host");
    let port = container.get_host_port_ipv4(5432).await.expect("get port");
    let url  = format!("postgres://postgres:postgres@{host}:{port}/postgres");

    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&url)
        .await
        .expect("failed to connect to test DB");

    sqlx::migrate!("./migrations")
        .run(&pool)
        .await
        .expect("migrations failed");

    (pool, container)
}

async fn make_app(pool: sqlx::PgPool) -> axum::Router {
    use server::{auth::JwtConfig, config::AppConfig, router::create_router, state::AppState};

    let jwt    = JwtConfig::new("test-secret-key-for-integration", 3600);
    let state  = AppState::for_testing(pool, jwt).await;
    let config = AppConfig::test_default();
    create_router(&config, state)
}

// ── Request helpers ───────────────────────────────────────────────────────────

async fn req_json(
    app:    &axum::Router,
    method: &str,
    path:   &str,
    token:  Option<&str>,
    body:   Option<Value>,
) -> (StatusCode, Value) {
    let mut builder = Request::builder()
        .method(method)
        .uri(path);

    if let Some(t) = token {
        builder = builder.header(header::AUTHORIZATION, format!("Bearer {t}"));
    }

    let req_body = match body {
        Some(v) => {
            builder = builder.header(header::CONTENT_TYPE, "application/json");
            Body::from(serde_json::to_vec(&v).unwrap())
        }
        None => Body::empty(),
    };

    let res = app.clone().oneshot(builder.body(req_body).unwrap()).await.unwrap();
    let status = res.status();
    let bytes  = axum::body::to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let json   = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, json)
}

// ── Seed helpers ──────────────────────────────────────────────────────────────

async fn seed_admin(pool: &sqlx::PgPool, username: &str, password: &str) {
    use server::{auth::hash_password, db::create_user};
    let hash = hash_password(password).unwrap();
    create_user(pool, username, &hash, true, false).await.unwrap();
}

async fn seed_regular(pool: &sqlx::PgPool, username: &str, password: &str) {
    use server::{auth::hash_password, db::create_user};
    let hash = hash_password(password).unwrap();
    create_user(pool, username, &hash, false, false).await.unwrap();
}

async fn login(app: &axum::Router, username: &str, password: &str) -> String {
    let (_, body) = req_json(
        app, "POST", "/auth/login", None,
        Some(json!({ "username": username, "password": password })),
    )
    .await;
    body["token"].as_str().unwrap().to_owned()
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn health_check_returns_ok() {
    let (pool, _c) = setup_db().await;
    let app = make_app(pool).await;
    let (status, _) = req_json(&app, "GET", "/health", None, None).await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn login_success() {
    let (pool, _c) = setup_db().await;
    seed_admin(&pool, "admin", "password123").await;
    let app = make_app(pool).await;

    let (status, body) = req_json(
        &app, "POST", "/auth/login", None,
        Some(json!({ "username": "admin", "password": "password123" })),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert!(body["token"].is_string());
    assert_eq!(body["require_password_reset"], false);
}

#[tokio::test]
async fn login_wrong_password_returns_401() {
    let (pool, _c) = setup_db().await;
    seed_admin(&pool, "admin", "correct").await;
    let app = make_app(pool).await;

    let (status, _) = req_json(
        &app, "POST", "/auth/login", None,
        Some(json!({ "username": "admin", "password": "wrong" })),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn stream_requires_auth() {
    let (pool, _c) = setup_db().await;
    let app = make_app(pool).await;
    let (status, _) = req_json(&app, "GET", "/stream?url=dQw4w9WgXcQ", None, None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn admin_endpoints_require_admin_role() {
    let (pool, _c) = setup_db().await;
    seed_regular(&pool, "regular", "password123").await;
    let app = make_app(pool).await;
    let token = login(&app, "regular", "password123").await;

    let (status, _) = req_json(&app, "GET", "/admin/users", Some(&token), None).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn admin_create_and_list_users() {
    let (pool, _c) = setup_db().await;
    seed_admin(&pool, "admin", "adminpass1").await;
    let app = make_app(pool).await;
    let token = login(&app, "admin", "adminpass1").await;

    // Create a new user
    let (create_status, create_body) = req_json(
        &app, "POST", "/admin/users", Some(&token),
        Some(json!({
            "username": "newuser",
            "password": "newpassword1",
            "is_admin": false,
            "require_password_reset": true
        })),
    )
    .await;
    assert_eq!(create_status, StatusCode::CREATED);
    assert_eq!(create_body["username"], "newuser");
    assert_eq!(create_body["require_password_reset"], true);
    assert!(create_body["generated_password"].is_null());

    // List users — should now have 2
    let (list_status, list_body) = req_json(&app, "GET", "/admin/users", Some(&token), None).await;
    assert_eq!(list_status, StatusCode::OK);
    assert_eq!(list_body.as_array().unwrap().len(), 2);
}

#[tokio::test]
async fn admin_create_user_with_generated_password() {
    let (pool, _c) = setup_db().await;
    seed_admin(&pool, "admin", "adminpass1").await;
    let app = make_app(pool).await;
    let token = login(&app, "admin", "adminpass1").await;

    let (status, body) = req_json(
        &app, "POST", "/admin/users", Some(&token),
        Some(json!({ "username": "autopass_user" })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    assert!(body["generated_password"].as_str().unwrap().len() >= 8);
}

#[tokio::test]
async fn admin_create_duplicate_user_returns_409() {
    let (pool, _c) = setup_db().await;
    seed_admin(&pool, "admin", "adminpass1").await;
    let app = make_app(pool).await;
    let token = login(&app, "admin", "adminpass1").await;

    req_json(&app, "POST", "/admin/users", Some(&token),
        Some(json!({ "username": "dup", "password": "password1" }))).await;

    let (status, _) = req_json(&app, "POST", "/admin/users", Some(&token),
        Some(json!({ "username": "dup", "password": "password2" }))).await;
    assert_eq!(status, StatusCode::CONFLICT);
}

#[tokio::test]
async fn admin_delete_user() {
    let (pool, _c) = setup_db().await;
    seed_admin(&pool, "admin", "adminpass1").await;
    let app = make_app(pool).await;
    let token = login(&app, "admin", "adminpass1").await;

    let (_, create_body) = req_json(
        &app, "POST", "/admin/users", Some(&token),
        Some(json!({ "username": "todelete", "password": "password1" })),
    )
    .await;
    let user_id = create_body["id"].as_str().unwrap();

    let (del_status, _) =
        req_json(&app, "DELETE", &format!("/admin/users/{user_id}"), Some(&token), None).await;
    assert_eq!(del_status, StatusCode::OK);

    let (_, list) = req_json(&app, "GET", "/admin/users", Some(&token), None).await;
    assert_eq!(list.as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn change_password_works() {
    let (pool, _c) = setup_db().await;
    seed_regular(&pool, "user1", "oldpassword").await;
    let app = make_app(pool).await;
    let token = login(&app, "user1", "oldpassword").await;

    let (status, _) = req_json(
        &app, "POST", "/auth/change-password", Some(&token),
        Some(json!({ "current_password": "oldpassword", "new_password": "newpassword1" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // Old password should no longer work
    let (old_status, _) = req_json(
        &app, "POST", "/auth/login", None,
        Some(json!({ "username": "user1", "password": "oldpassword" })),
    )
    .await;
    assert_eq!(old_status, StatusCode::UNAUTHORIZED);

    // New password must work
    let (new_status, _) = req_json(
        &app, "POST", "/auth/login", None,
        Some(json!({ "username": "user1", "password": "newpassword1" })),
    )
    .await;
    assert_eq!(new_status, StatusCode::OK);
}
