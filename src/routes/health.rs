use axum::Json;
use serde::Serialize;
use utoipa::ToSchema;

use crate::error::ApiResult;

/// Response body for the health-check endpoint.
#[derive(Debug, Serialize, ToSchema)]
pub struct HealthResponse {
    pub status:  &'static str,
    pub version: &'static str,
}

/// `GET /health`
///
/// Returns a lightweight liveness probe that load-balancers and container
/// orchestrators (e.g. Kubernetes) can poll to confirm the server is running.
#[utoipa::path(
    get,
    path = "/health",
    tag  = "system",
    responses(
        (status = 200, description = "Service is healthy", body = HealthResponse),
    )
)]
#[tracing::instrument]
pub async fn health() -> ApiResult<Json<HealthResponse>> {
    Ok(Json(HealthResponse {
        status:  "ok",
        version: env!("CARGO_PKG_VERSION"),
    }))
}
