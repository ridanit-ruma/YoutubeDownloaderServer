use std::time::Duration;

use axum::routing::{delete, get, patch, post};
use tower_http::{
    cors::{Any, CorsLayer},
    request_id::{MakeRequestUuid, PropagateRequestIdLayer, SetRequestIdLayer},
    timeout::TimeoutLayer,
    trace::{DefaultMakeSpan, DefaultOnResponse, TraceLayer},
};
use tracing::Level;
use utoipa::OpenApi;
use utoipa_swagger_ui::SwaggerUi;

use crate::{config::AppConfig, routes, state::AppState};

// ── OpenAPI specification ─────────────────────────────────────────────────────

#[derive(OpenApi)]
#[openapi(
    info(
        title       = "YouTube Audio Downloader API",
        version     = env!("CARGO_PKG_VERSION"),
        description = "Stream YouTube audio with JWT-based authentication.",
    ),
    paths(
        routes::health::health,
        routes::auth::login,
        routes::auth::change_password,
        routes::admin::admin_list_users,
        routes::admin::admin_create_user,
        routes::admin::admin_delete_user,
        routes::admin::admin_set_admin,
        routes::admin::admin_set_require_password_reset,
        routes::stream::stream_audio,
    ),
    components(
        schemas(
            routes::health::HealthResponse,
            routes::auth::LoginRequest,
            routes::auth::LoginResponse,
            routes::auth::ChangePasswordRequest,
            routes::auth::MessageResponse,
            routes::admin::CreateUserRequest,
            routes::admin::CreateUserResponse,
            routes::admin::UserSummary,
            routes::admin::MessageResponse,
            routes::admin::SetAdminRequest,
            routes::admin::SetRequirePasswordResetRequest,
        )
    ),
    tags(
        (name = "system",  description = "Health & liveness probes"),
        (name = "auth",    description = "Authentication (login, password change)"),
        (name = "admin",   description = "Admin-only user management"),
        (name = "youtube", description = "YouTube audio streaming"),
    ),
    modifiers(&SecurityAddon),
)]
pub struct ApiDoc;

struct SecurityAddon;

impl utoipa::Modify for SecurityAddon {
    fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
        if let Some(components) = openapi.components.as_mut() {
            use utoipa::openapi::security::{HttpAuthScheme, HttpBuilder, SecurityScheme};
            components.add_security_scheme(
                "bearer_auth",
                SecurityScheme::Http(
                    HttpBuilder::new()
                        .scheme(HttpAuthScheme::Bearer)
                        .bearer_format("JWT")
                        .build(),
                ),
            );
        }
    }
}

// ── Router ────────────────────────────────────────────────────────────────────

/// Assembles and returns the full application router with all middleware.
pub fn create_router(config: &AppConfig, state: AppState) -> axum::Router {
    let trace_layer = TraceLayer::new_for_http()
        .make_span_with(DefaultMakeSpan::new().level(Level::INFO).include_headers(false))
        .on_response(DefaultOnResponse::new().level(Level::INFO));

    let cors_layer = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    let timeout_layer = TimeoutLayer::with_status_code(
        axum::http::StatusCode::REQUEST_TIMEOUT,
        Duration::from_secs(config.request_timeout_secs),
    );

    let set_request_id       = SetRequestIdLayer::x_request_id(MakeRequestUuid);
    let propagate_request_id = PropagateRequestIdLayer::x_request_id();

    // ── API routes ────────────────────────────────────────────────────────────
    // /stream is excluded from the timeout layer because audio streaming can
    // take several minutes depending on file size and network speed.
    let stream_router = axum::Router::new()
        .route("/stream", get(routes::stream::stream_audio))
        .with_state(state.clone());

    let api = axum::Router::new()
        // Public
        .route("/health", get(routes::health::health))
        .route("/auth/login", post(routes::auth::login))
        // Authenticated (any valid user)
        .route("/auth/change-password", post(routes::auth::change_password))
        // Admin only
        .route("/admin/users", get(routes::admin::admin_list_users))
        .route("/admin/users", post(routes::admin::admin_create_user))
        .route("/admin/users/{id}", delete(routes::admin::admin_delete_user))
        .route("/admin/users/{id}/admin", patch(routes::admin::admin_set_admin))
        .route(
            "/admin/users/{id}/require-password-reset",
            patch(routes::admin::admin_set_require_password_reset),
        )
        .layer(timeout_layer)
        .with_state(state);

    // ── Swagger UI (served at /swagger-ui) ───────────────────────────────────
    let swagger = SwaggerUi::new("/swagger-ui")
        .url("/api-docs/openapi.json", ApiDoc::openapi());

    axum::Router::new()
        .merge(stream_router)
        .merge(api)
        .merge(swagger)
        .layer(propagate_request_id)
        .layer(trace_layer)
        .layer(cors_layer)
        .layer(set_request_id)
}
