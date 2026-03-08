# ─────────────────────────────────────────────────────────────────────────────
# Stage 1: Builder
#   - Caches dependency compilation separately from application code
# ─────────────────────────────────────────────────────────────────────────────
FROM rust:1.88-slim-bookworm AS builder

RUN apt-get update && apt-get install -y --no-install-recommends \
        libssl-dev \
        pkg-config \
        libpq-dev \
        ca-certificates \
        curl \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# ── Dependency-layer cache trick ──────────────────────────────────────────────
# Copy only manifests first so the dependency build layer is cached separately
# and not re-run on every source-code change.
COPY Cargo.toml Cargo.lock ./

# Dummy source files so Cargo can resolve the full dependency graph.
# Cargo.toml declares [[test]] name = "integration" → tests/integration.rs must exist.
RUN mkdir -p src tests && \
    echo 'fn main() {}' > src/main.rs && \
    echo '' > src/lib.rs && \
    echo '' > tests/integration.rs

RUN cargo build --release --features test-helpers && \
    rm -rf src tests

# ── Copy real source & build ──────────────────────────────────────────────────
COPY src        ./src
COPY migrations ./migrations
COPY tests      ./tests

# Touch main.rs so Cargo notices the source changed.
RUN touch src/main.rs src/lib.rs

RUN cargo build --release

# ─────────────────────────────────────────────────────────────────────────────
# Stage 2: Runtime
#   - Minimal Debian image — no Rust toolchain
# ─────────────────────────────────────────────────────────────────────────────
FROM debian:bookworm-slim AS runtime

RUN apt-get update && apt-get install -y --no-install-recommends \
        libssl3 \
        libpq5 \
        ca-certificates \
        python3 \
        ffmpeg \
        curl \
    && rm -rf /var/lib/apt/lists/*

RUN groupadd --system app && useradd --system --gid app app

WORKDIR /app

COPY --from=builder /app/target/release/server /app/server
COPY --from=builder /app/migrations            /app/migrations

# Directories written to at runtime (yt-dlp binary download + audio output).
RUN mkdir -p /app/libs /app/output && chown -R app:app /app

USER app

ENV HOST=0.0.0.0 \
    PORT=3000 \
    LOG_LEVEL=info \
    REQUEST_TIMEOUT_SECS=30 \
    JWT_EXPIRY_SECS=86400

EXPOSE 3000

# yt-dlp downloads its binary on first boot → generous start period
HEALTHCHECK --interval=30s --timeout=10s --start-period=60s --retries=3 \
    CMD curl -f http://localhost:3000/health || exit 1

ENTRYPOINT ["/app/server"]
