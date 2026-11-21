CREATE TABLE user_connections (
    user_id BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    provider TEXT NOT NULL,
    account_id TEXT NOT NULL,
    name TEXT NOT NULL,
    visible BOOLEAN NOT NULL DEFAULT FALSE,
    PRIMARY KEY (user_id, provider),
    UNIQUE (provider, account_id)
);
CREATE TABLE connection_oauth_states (
    state_hash TEXT PRIMARY KEY,
    user_id BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    session_id UUID NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    verifier TEXT NOT NULL,
    expires_at TIMESTAMPTZ NOT NULL
);
