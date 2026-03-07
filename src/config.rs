use serde::Deserialize;

/// Application configuration loaded from environment variables.
///
/// All fields have defaults so the server can start with zero configuration,
/// while still being fully overridable via environment variables or a `.env` file.
#[derive(Debug, Clone, Deserialize)]
pub struct AppConfig {
    /// TCP port the HTTP server listens on.
    #[serde(default = "AppConfig::default_port")]
    pub port: u16,

    /// Host address to bind. Defaults to `127.0.0.1` for local development.
    /// Set to `0.0.0.0` in production/container environments.
    #[serde(default = "AppConfig::default_host")]
    pub host: String,

    /// Log level filter string (e.g. `info`, `debug`, `tower_http=debug`).
    #[serde(default = "AppConfig::default_log_level")]
    pub log_level: String,

    /// Timeout (in seconds) for each individual HTTP request.
    #[serde(default = "AppConfig::default_request_timeout_secs")]
    pub request_timeout_secs: u64,

    /// PostgreSQL connection URL e.g. `postgres://user:pass@localhost/dbname`
    pub database_url: String,

    /// Secret key used to sign/verify JWTs (HS256). Must be kept secret.
    pub jwt_secret: String,

    /// JWT access-token expiry in seconds (default: 24 h).
    #[serde(default = "AppConfig::default_jwt_expiry_secs")]
    pub jwt_expiry_secs: u64,

    /// Username to create on first startup when the users table is empty.
    #[serde(default = "AppConfig::default_initial_admin_username")]
    pub initial_admin_username: String,

    /// Password for the initial admin account.
    /// If not set, a random password is generated and printed to stdout on first run.
    pub initial_admin_password: Option<String>,

    /// Optional path to the Node.js executable used by yt-dlp to solve the YouTube
    /// n-signature throttle challenge. When set, `--js-runtimes node:<path>` is
    /// appended to every yt-dlp invocation so that the n-parameter is de-throttled
    /// and CDN downloads proceed at full speed.
    ///
    /// Defaults to the standard Windows installation path. Set to an empty string to
    /// disable Node.js integration entirely.
    #[serde(default = "AppConfig::default_node_path")]
    pub node_path: String,
}

impl AppConfig {
    fn default_port() -> u16 {
        3000
    }

    fn default_host() -> String {
        "127.0.0.1".to_owned()
    }

    fn default_log_level() -> String {
        "info".to_owned()
    }

    fn default_request_timeout_secs() -> u64 {
        30
    }

    fn default_jwt_expiry_secs() -> u64 {
        86_400 // 24 h
    }

    fn default_initial_admin_username() -> String {
        "admin".to_owned()
    }

    fn default_node_path() -> String {
        // Standard Node.js installation path on Windows.
        // Override via NODE_PATH env var if Node.js is installed elsewhere.
        r"C:\Program Files\nodejs\node.exe".to_owned()
    }

    /// Returns the full bind address as `host:port`.
    pub fn bind_addr(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }

    /// Minimal config suitable for unit / integration tests.
    /// Never use in production.
    #[cfg(any(test, feature = "test-helpers"))]
    pub fn test_default() -> Self {
        Self {
            port:                    0,
            host:                    "127.0.0.1".to_owned(),
            log_level:               "error".to_owned(),
            request_timeout_secs:    30,
            database_url:            "postgres://postgres:postgres@localhost/test".to_owned(),
            jwt_secret:              "test-secret-key-for-integration".to_owned(),
            jwt_expiry_secs:         3600,
            initial_admin_username:  "admin".to_owned(),
            initial_admin_password:  None,
            node_path:               String::new(),
        }
    }
}
