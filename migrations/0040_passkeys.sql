CREATE TABLE user_passkeys (
    id UUID PRIMARY KEY,
    user_id BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    name TEXT NOT NULL CHECK (char_length(name) BETWEEN 1 AND 64),
    credential_id BYTEA NOT NULL UNIQUE,
    credential JSONB NOT NULL,
    signature_counter BIGINT NOT NULL CHECK (signature_counter BETWEEN 0 AND 4294967295),
    created_at TIMESTAMPTZ NOT NULL,
    last_used_at TIMESTAMPTZ
);
CREATE INDEX user_passkeys_owner ON user_passkeys(user_id);
CREATE TABLE passkey_challenges (
    id UUID PRIMARY KEY,
    user_id BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    kind TEXT NOT NULL CHECK (kind IN ('registration', 'authentication')),
    binding TEXT NOT NULL,
    name TEXT,
    state JSONB NOT NULL,
    expires_at TIMESTAMPTZ NOT NULL,
    created_at TIMESTAMPTZ NOT NULL
);
CREATE INDEX passkey_challenges_expiry ON passkey_challenges(expires_at);
CREATE INDEX passkey_challenges_owner ON passkey_challenges(user_id, created_at);
