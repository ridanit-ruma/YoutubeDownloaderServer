mod auth;
mod config;
mod db;
mod error;
mod middleware;
mod router;
mod routes;
mod state;
mod youtube;

use std::path::PathBuf;

use anyhow::Context as _;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt as _;
use sqlx::postgres::PgPoolOptions;
use tracing::info;
use tracing_subscriber::{EnvFilter, fmt, prelude::*};
use yt_dlp::Downloader;

use crate::{
    auth::{JwtConfig, generate_random_password, hash_password},
    state::AppState,
};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // ── 1. Load .env ─────────────────────────────────────────────────────────
    dotenvy::dotenv().ok();

    // ── 2. Deserialise environment variables into AppConfig ──────────────────
    let config: config::AppConfig = envy::from_env()
        .context("failed to load configuration from environment variables")?;

    // ── 3. Initialise tracing subscriber ────────────────────────────────────
    init_tracing(&config.log_level);

    info!(
        host    = %config.host,
        port    = config.port,
        version = env!("CARGO_PKG_VERSION"),
        "starting server"
    );

    // ── 4. Ensure the target database exists, then connect ───────────────────
    ensure_database_exists(&config.database_url).await?;

    info!("connecting to PostgreSQL…");
    let db_pool = PgPoolOptions::new()
        .max_connections(10)
        .connect(&config.database_url)
        .await
        .context("failed to connect to PostgreSQL")?;

    sqlx::migrate!("./migrations")
        .run(&db_pool)
        .await
        .context("failed to run database migrations")?;

    info!("database migrations applied");

    // ── 5. Seed initial admin user if no users exist ─────────────────────────
    let user_count = db::count_users(&db_pool).await?;
    if user_count == 0 {
        let password = match &config.initial_admin_password {
            Some(p) => p.clone(),
            None => {
                let p = generate_random_password(20);
                println!(
                    "\n\
                     ╔══════════════════════════════════════════════╗\n\
                     ║  Initial admin account created               ║\n\
                     ║  Username : {:<34}║\n\
                     ║  Password : {:<34}║\n\
                     ║  (Change this immediately via /auth/change-password)\n\
                     ╚══════════════════════════════════════════════╝\n",
                    config.initial_admin_username,
                    p
                );
                p
            }
        };

        let hash = hash_password(&password)
            .map_err(|_| anyhow::anyhow!("failed to hash initial admin password"))?;

        db::create_user(&db_pool, &config.initial_admin_username, &hash, true, false).await?;
        info!(username = %config.initial_admin_username, "initial admin user created");
    }

    // ── 6. Initialise yt-dlp Downloader ─────────────────────────────────────
    let libs_dir   = PathBuf::from("libs");
    let output_dir = PathBuf::from("output");

    info!("initialising yt-dlp (may download binaries on first run)…");

    let mut downloader_builder = Downloader::with_new_binaries(libs_dir.clone(), output_dir)
        .await
        .context("failed to install yt-dlp / ffmpeg binaries")?;

    // The yt-dlp crate downloads the binary but does not set the executable
    // bit on Linux. Ensure both binaries are executable before we proceed.
    #[cfg(unix)]
    {
        for name in &["yt-dlp", "ffmpeg"] {
            let path = libs_dir.join(name);
            if path.exists() {
                let mut perms = std::fs::metadata(&path)
                    .with_context(|| format!("failed to stat {}", path.display()))?
                    .permissions();
                // Add owner+group+other execute bits (0o111).
                perms.set_mode(perms.mode() | 0o111);
                std::fs::set_permissions(&path, perms)
                    .with_context(|| format!("failed to chmod +x {}", path.display()))?;
                info!(path = %path.display(), "ensured executable bit on binary");
            }
        }
    }

    // If a Node.js executable is configured, tell yt-dlp to use it so that the
    // YouTube n-signature throttle challenge can be solved and CDN downloads run
    // at full speed instead of being throttled to ~32 KB/s.
    if !config.node_path.is_empty() {
        let node_exe = std::path::Path::new(&config.node_path);
        if node_exe.exists() {
            let arg = format!("--js-runtimes=node:{}", config.node_path);
            info!(node_path = %config.node_path, "enabling yt-dlp Node.js JS runtime for n-signature solving");
            downloader_builder = downloader_builder.add_arg(arg);
        } else {
            tracing::warn!(
                node_path = %config.node_path,
                "NODE_PATH is set but the file does not exist — n-signature throttle will not be resolved"
            );
        }
    }

    let downloader = downloader_builder
        .build()
        .await
        .context("failed to build yt-dlp Downloader")?;

    // ── 7. Build shared HTTP client ───────────────────────────────────────────
    let http_client = reqwest::Client::builder()
        // Only limit how long we wait to *connect* to the CDN, not the total
        // transfer time. Audio streams can take several minutes to complete,
        // so a per-request completion timeout would kill long downloads.
        .connect_timeout(std::time::Duration::from_secs(config.request_timeout_secs))
        .user_agent(concat!(env!("CARGO_PKG_NAME"), "/", env!("CARGO_PKG_VERSION")))
        .build()
        .context("failed to build HTTP client")?;

    // ── 8. Build JWT config ───────────────────────────────────────────────────
    let jwt = JwtConfig::new(&config.jwt_secret, config.jwt_expiry_secs);

    let app_state = AppState::new(downloader, http_client, db_pool, jwt);

    // ── 9. Build the router ───────────────────────────────────────────────────
    let app = router::create_router(&config, app_state);

    // ── 10. Bind and serve ────────────────────────────────────────────────────
    let bind_addr = config.bind_addr();
    let listener = tokio::net::TcpListener::bind(&bind_addr)
        .await
        .with_context(|| format!("failed to bind to {bind_addr}"))?;

    info!(
        addr        = %listener.local_addr().context("failed to read local addr")?,
        swagger_ui  = %format!("http://{bind_addr}/swagger-ui"),
        "listening"
    );

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .context("server error")?;

    info!("server shut down gracefully");
    Ok(())
}

/// Parses `database_url`, connects to the `postgres` maintenance DB on the
/// same host, and issues `CREATE DATABASE` if the target DB does not exist.
///
/// This lets the server start cleanly even when the operator has only
/// configured a PostgreSQL *server* but not yet created the application DB.
async fn ensure_database_exists(database_url: &str) -> anyhow::Result<()> {
    // ── Parse the target database name from the URL ───────────────────────────
    // Expected format: postgres://user:pass@host:port/dbname[?params]
    let db_name = database_url
        .rsplit_once('/')
        .map(|(_, tail)| tail.split('?').next().unwrap_or(tail).trim())
        .filter(|s| !s.is_empty())
        .with_context(|| {
            format!("cannot determine database name from DATABASE_URL: {database_url}")
        })?
        .to_owned();

    // ── Build a maintenance URL pointing at the `postgres` system database ───
    // Replace the last path segment (the db name) with `postgres`.
    let maintenance_url = database_url
        .rsplit_once('/')
        .map(|(prefix, tail)| {
            // Preserve any query string that was on the original URL.
            let query = tail.find('?').map(|i| &tail[i..]).unwrap_or("");
            format!("{prefix}/postgres{query}")
        })
        .unwrap(); // safe: we already validated the URL above

    // ── Connect to the maintenance DB ────────────────────────────────────────
    let maintenance_pool = PgPoolOptions::new()
        .max_connections(1)
        .connect(&maintenance_url)
        .await
        .with_context(|| {
            format!("failed to connect to PostgreSQL server (maintenance DB) at {maintenance_url}")
        })?;

    // ── Check whether the target database already exists ─────────────────────
    let exists: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM pg_database WHERE datname = $1)",
    )
    .bind(&db_name)
    .fetch_one(&maintenance_pool)
    .await
    .with_context(|| format!("failed to query pg_database for '{db_name}'"))?;

    if exists {
        info!(db = %db_name, "database already exists");
    } else {
        // `CREATE DATABASE` cannot be parameterised, but db_name comes from
        // our own config (not user input), so we validate it first.
        anyhow::ensure!(
            db_name.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '-'),
            "database name '{db_name}' contains unsafe characters"
        );

        info!(db = %db_name, "database not found — creating…");

        // Use a raw string; identifier quoting with double-quotes is safe here.
        sqlx::query(&format!("CREATE DATABASE \"{db_name}\""))
            .execute(&maintenance_pool)
            .await
            .with_context(|| format!("failed to create database '{db_name}'"))?;

        info!(db = %db_name, "database created successfully");
    }

    maintenance_pool.close().await;
    Ok(())
}

fn init_tracing(default_level: &str) {    let filter = EnvFilter::try_from_env("LOG_LEVEL")
        .or_else(|_| EnvFilter::try_from_env("RUST_LOG"))
        .unwrap_or_else(|_| EnvFilter::new(default_level));

    tracing_subscriber::registry()
        .with(filter)
        .with(fmt::layer().with_target(true).with_thread_ids(false))
        .init();
}

async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    {
        let terminate = async {
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                .expect("failed to install SIGTERM handler")
                .recv()
                .await;
        };

        tokio::select! {
            _ = ctrl_c    => {},
            _ = terminate => {},
        }
    }

    #[cfg(not(unix))]
    ctrl_c.await;

    info!("shutdown signal received, draining connections…");
}
