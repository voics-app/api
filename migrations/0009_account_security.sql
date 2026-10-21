-- Account recovery and two-factor: emailed tokens, backup codes, pending TOTP
-- enrolment, and disabled / scheduled-for-deletion state.

ALTER TABLE user_credentials ADD COLUMN pending_totp_secret TEXT;

ALTER TABLE users
    ADD COLUMN disabled_at   TIMESTAMPTZ,
    ADD COLUMN delete_after  TIMESTAMPTZ;
CREATE INDEX users_delete_after_idx ON users (delete_after) WHERE delete_after IS NOT NULL;

-- Single-use tokens (email verification, password reset, login MFA tickets).
-- Only hashes are stored.
CREATE TABLE account_tokens (
    token_hash TEXT PRIMARY KEY,
    user_id    BIGINT      NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    purpose    TEXT        NOT NULL CHECK (purpose IN ('verify_email', 'reset_password', 'mfa_login')),
    email      TEXT,
    expires_at TIMESTAMPTZ NOT NULL,
    used_at    TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX account_tokens_user_idx ON account_tokens (user_id, purpose);

-- Backup codes are argon2 hashes; a consumed row stays so the list shows what was used.
CREATE TABLE user_backup_codes (
    id         BIGSERIAL PRIMARY KEY,
    user_id    BIGINT      NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    code_hash  TEXT        NOT NULL,
    consumed_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX user_backup_codes_user_idx ON user_backup_codes (user_id, id);
