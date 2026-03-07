-- Migration: create users table
CREATE EXTENSION IF NOT EXISTS "uuid-ossp";

CREATE TABLE users (
    id                    UUID        PRIMARY KEY DEFAULT uuid_generate_v4(),
    username              TEXT        NOT NULL UNIQUE,
    password_hash         TEXT        NOT NULL,
    is_admin              BOOLEAN     NOT NULL DEFAULT FALSE,
    require_password_reset BOOLEAN    NOT NULL DEFAULT FALSE,
    created_at            TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at            TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_users_username ON users (username);
