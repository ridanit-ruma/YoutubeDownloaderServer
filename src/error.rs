use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;

// ─── HttpError ────────────────────────────────────────────────────────────────

/// A structured HTTP error with an explicit status code.
///
/// Use this when you want to return a well-known 4xx/5xx response with a
/// descriptive message — e.g. 404 Not Found or 400 Bad Request.
#[derive(Debug)]
pub struct HttpError {
    pub status:  StatusCode,
    pub code:    &'static str,
    pub message: String,
}

#[allow(dead_code)]
impl HttpError {
    pub fn new(status: StatusCode, code: &'static str, message: impl Into<String>) -> Self {
        Self { status, code, message: message.into() }
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        Self::new(StatusCode::NOT_FOUND, "NOT_FOUND", message)
    }

    pub fn bad_request(message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, "BAD_REQUEST", message)
    }

    pub fn unprocessable(message: impl Into<String>) -> Self {
        Self::new(StatusCode::UNPROCESSABLE_ENTITY, "UNPROCESSABLE_ENTITY", message)
    }
}

impl IntoResponse for HttpError {
    fn into_response(self) -> Response {
        tracing::warn!(
            status  = self.status.as_u16(),
            code    = self.code,
            message = %self.message,
            "http error response"
        );

        let body = json!({
            "error": {
                "message": self.message,
                "code":    self.code,
            }
        });

        (self.status, Json(body)).into_response()
    }
}

// ─── AppError ─────────────────────────────────────────────────────────────────

/// Unified error type for axum handlers.
///
/// Carries either:
/// - an [`HttpError`] with an explicit status code (4xx / 5xx), or
/// - an opaque internal error that maps to 500.
///
/// Handlers return `Result<T, AppError>` and use `?` freely. `AppError`
/// implements [`IntoResponse`] so axum can convert it automatically.
#[derive(Debug)]
pub enum AppError {
    Http(HttpError),
    Internal(anyhow::Error),
}

impl AppError {
    /// Wraps an [`HttpError`] (preserves its status code in the response).
    pub fn from_http(e: HttpError) -> Self {
        Self::Http(e)
    }

    /// Wraps any message as a 500 Internal Server Error.
    pub fn internal(message: impl Into<String>) -> Self {
        Self::Internal(anyhow::anyhow!("{}", message.into()))
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        match self {
            AppError::Http(e) => e.into_response(),
            AppError::Internal(e) => {
                tracing::error!(error = ?e, "unhandled application error");
                let body = json!({
                    "error": {
                        "message": "an internal server error occurred",
                        "code":    "INTERNAL_SERVER_ERROR",
                    }
                });
                (StatusCode::INTERNAL_SERVER_ERROR, Json(body)).into_response()
            }
        }
    }
}

/// Blanket conversion: `sqlx::Error` → 500.
impl From<sqlx::Error> for AppError {
    fn from(err: sqlx::Error) -> Self {
        Self::Internal(anyhow::anyhow!("database error: {err}"))
    }
}

/// Result alias that reduces boilerplate in handler signatures.
pub type ApiResult<T> = Result<T, AppError>;

// Allow AppError to be used with `anyhow::Error` / `?` in main.rs
impl std::fmt::Display for AppError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AppError::Http(e)     => write!(f, "HTTP {}: {}", e.status, e.message),
            AppError::Internal(e) => write!(f, "internal error: {e}"),
        }
    }
}

impl std::error::Error for AppError {}


