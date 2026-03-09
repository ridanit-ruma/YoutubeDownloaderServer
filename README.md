# YoutubeDownloaderServer

A Rust backend that streams YouTube audio and video to the browser, with JWT authentication and user management.

## Features

- `GET /stream` — streams the best available audio format from YouTube
- `GET /stream-video` — streams a video-only track at a requested resolution
- Parallel range-based downloading to work around YouTube CDN throttling (chunks = CPU core count)
- JWT authentication (HS256), Argon2id password hashing
- Admin API for user management
- Automatic database migration on startup (SQLx)
- Initial admin account auto-created on first run
- Swagger UI at `/swagger-ui`

## Tech Stack

- Rust (Edition 2024)
- Axum 0.8
- SQLx (PostgreSQL)
- yt-dlp
- Argon2id, JWT (HS256)

## Environment Variables

### Required

| Variable | Description |
|---|---|
| `DATABASE_URL` | PostgreSQL connection string (e.g. `postgresql://user:pass@host:5432/dbname`) |
| `JWT_SECRET` | Secret key for signing JWTs |

### Optional

| Variable | Default | Description |
|---|---|---|
| `PORT` | `3000` | Port to listen on |
| `HOST` | `127.0.0.1` | Bind address |
| `LOG_LEVEL` | `info` | Log level (`trace`, `debug`, `info`, `warn`, `error`) |
| `REQUEST_TIMEOUT_SECS` | `30` | HTTP request timeout in seconds |
| `JWT_EXPIRY_SECS` | `86400` | JWT lifetime in seconds (default: 24 h) |
| `INITIAL_ADMIN_USERNAME` | `admin` | Username for the auto-created admin account |
| `INITIAL_ADMIN_PASSWORD` | *(auto-generated)* | Password for the auto-created admin account |
| `NODE_PATH` | *(none)* | Path to Node.js binary (used by yt-dlp for n-signature decryption) |

Copy `.env.example` to `.env` and fill in the required values for local development.

## Build

```bash
cd server
cargo build --release
# binary: target/release/server
```

## Docker / Podman

Build from the **project root** (one level above this directory):

```bash
podman build -f Dockerfile -t ytdlweb-server:latest .
```

Run:

```bash
podman run -d \
  -e DATABASE_URL=postgresql://user:pass@db-host:5432/dbname \
  -e JWT_SECRET=<random-secret> \
  -e INITIAL_ADMIN_USERNAME=admin \
  -e INITIAL_ADMIN_PASSWORD=yourpassword \
  -p 3000:3000 \
  ytdlweb-server:latest
```

## API Reference

| Method | Path | Auth | Description |
|---|---|---|---|
| `GET` | `/health` | — | Health check |
| `POST` | `/auth/login` | — | Login, returns JWT |
| `POST` | `/auth/change-password` | Bearer | Change own password |
| `GET` | `/admin/users` | Bearer + admin | List all users |
| `POST` | `/admin/users` | Bearer + admin | Create a user |
| `DELETE` | `/admin/users/{id}` | Bearer + admin | Delete a user |
| `PATCH` | `/admin/users/{id}/admin` | Bearer + admin | Toggle admin flag |
| `PATCH` | `/admin/users/{id}/require-password-reset` | Bearer + admin | Force password reset |
| `GET` | `/stream?url=` | Bearer | Stream best audio |
| `GET` | `/stream-video?url=&height=` | Bearer | Stream video at given height |
| `GET` | `/swagger-ui` | — | Interactive API docs |

Full request/response schemas are available in the Swagger UI.

## Database Schema

```sql
CREATE TABLE users (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    username    TEXT UNIQUE NOT NULL,
    password_hash TEXT NOT NULL,
    is_admin    BOOLEAN NOT NULL DEFAULT FALSE,
    require_password_reset BOOLEAN NOT NULL DEFAULT FALSE,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);
```

Migrations are applied automatically at startup from the `migrations/` directory.
