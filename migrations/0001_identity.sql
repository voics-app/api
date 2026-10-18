-- Accounts, credentials, settings, sessions, devices and relationships.

CREATE TABLE users (
    id                BIGINT PRIMARY KEY,
    username          TEXT        NOT NULL,
    email             TEXT,
    global_name       TEXT,
    avatar            TEXT,
    bot               BOOLEAN     NOT NULL DEFAULT FALSE,
    system            BOOLEAN     NOT NULL DEFAULT FALSE,
    verified          BOOLEAN     NOT NULL DEFAULT FALSE,
    mfa_enabled       BOOLEAN     NOT NULL DEFAULT FALSE,
    flags             BIGINT      NOT NULL DEFAULT 0,
    platform_admin    BOOLEAN     NOT NULL DEFAULT FALSE,
    suspended         BOOLEAN     NOT NULL DEFAULT FALSE,
    suspension_reason TEXT,
    suspended_until   TIMESTAMPTZ,
    created_at        TIMESTAMPTZ NOT NULL
);
CREATE UNIQUE INDEX users_username_key ON users (lower(username));
CREATE UNIQUE INDEX users_email_key ON users (lower(email)) WHERE email IS NOT NULL;
CREATE INDEX users_created_at_idx ON users (created_at);

CREATE TABLE user_credentials (
    user_id       BIGINT PRIMARY KEY REFERENCES users (id) ON DELETE CASCADE,
    password_hash TEXT NOT NULL,
    totp_secret   TEXT
);

CREATE TABLE user_settings (
    user_id               BIGINT PRIMARY KEY REFERENCES users (id) ON DELETE CASCADE,
    default_delivery_mode TEXT    NOT NULL DEFAULT 'standard'
        CHECK (default_delivery_mode IN ('standard', 'ephemeral', 'p2p')),
    theme                 TEXT    NOT NULL DEFAULT 'dark',
    locale                TEXT    NOT NULL DEFAULT 'en-US',
    status                TEXT    NOT NULL DEFAULT 'online',
    custom_status         JSONB,
    developer_mode        BOOLEAN NOT NULL DEFAULT FALSE,
    accent_color          INTEGER
);

CREATE TABLE devices (
    id           BIGINT PRIMARY KEY,
    user_id      BIGINT      NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    name         TEXT        NOT NULL,
    identity_key BYTEA       NOT NULL,
    created_at   TIMESTAMPTZ NOT NULL
);
CREATE INDEX devices_user_idx ON devices (user_id);

CREATE TABLE key_packages (
    id         BIGSERIAL PRIMARY KEY,
    device_id  BIGINT      NOT NULL REFERENCES devices (id) ON DELETE CASCADE,
    package    BYTEA       NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX key_packages_device_idx ON key_packages (device_id, id);

CREATE TABLE sessions (
    id         UUID PRIMARY KEY,
    user_id    BIGINT      NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    device_id  BIGINT REFERENCES devices (id) ON DELETE SET NULL,
    created_at TIMESTAMPTZ NOT NULL,
    revoked_at TIMESTAMPTZ
);
CREATE INDEX sessions_user_idx ON sessions (user_id);

-- Only hashes are stored. A rotated row stays so that replaying it is detected.
CREATE TABLE refresh_tokens (
    token_hash TEXT PRIMARY KEY,
    session_id UUID        NOT NULL REFERENCES sessions (id) ON DELETE CASCADE,
    expires_at TIMESTAMPTZ NOT NULL,
    rotated_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX refresh_tokens_session_idx ON refresh_tokens (session_id);

CREATE TABLE relationships (
    user_id   BIGINT      NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    target_id BIGINT      NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    kind      SMALLINT    NOT NULL CHECK (kind BETWEEN 1 AND 4),
    since     TIMESTAMPTZ NOT NULL,
    PRIMARY KEY (user_id, target_id)
);
