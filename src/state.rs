use std::sync::Arc;

use sqlx::PgPool;
use yt_dlp::Downloader;

use crate::auth::JwtConfig;

/// Shared application state injected into every axum handler via `State<AppState>`.
///
/// All fields are cheap to clone because they are `Arc`-backed internally.
#[derive(Clone)]
pub struct AppState {
    /// The yt-dlp downloader client.
    pub downloader: Arc<Downloader>,

    /// Shared reqwest HTTP client for proxying audio streams.
    pub http_client: reqwest::Client,

    /// PostgreSQL connection pool.
    pub db: PgPool,

    /// JWT signing / verification configuration.
    pub jwt: JwtConfig,
}

impl AppState {
    pub fn new(
        downloader:  Downloader,
        http_client: reqwest::Client,
        db:          PgPool,
        jwt:         JwtConfig,
    ) -> Self {
        Self {
            downloader: Arc::new(downloader),
            http_client,
            db,
            jwt,
        }
    }

    /// Create state for integration tests (initialises yt-dlp from the local libs/ dir).
    #[cfg(any(test, feature = "test-helpers"))]
    pub async fn for_testing(db: PgPool, jwt: JwtConfig) -> Self {
        use std::path::PathBuf;
        let downloader = Downloader::with_new_binaries(
            PathBuf::from("libs"),
            PathBuf::from("output"),
        )
        .await
        .expect("test: downloader init")
        .build()
        .await
        .expect("test: downloader build");

        Self {
            downloader: Arc::new(downloader),
            http_client: reqwest::Client::new(),
            db,
            jwt,
        }
    }
}

