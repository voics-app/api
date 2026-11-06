-- PushKit (VoIP) tokens of the iPhone app, one per login session. A VoIP token is a different token
-- from the notification token (push_tokens): it is issued by PushKit, needs no notification
-- permission, and is used only to ring the phone for an incoming call while the app is closed.
-- It leaves with its session and is pruned with stale push tokens.

CREATE TABLE voip_tokens (
    session_id  UUID PRIMARY KEY REFERENCES sessions (id) ON DELETE CASCADE,
    user_id     BIGINT      NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    token       TEXT        NOT NULL,
    environment TEXT        NOT NULL CHECK (environment IN ('production', 'sandbox')),
    created_at  TIMESTAMPTZ NOT NULL,
    updated_at  TIMESTAMPTZ NOT NULL
);
-- A PushKit token belongs to one installation: registering it again moves it.
CREATE UNIQUE INDEX voip_tokens_token_idx ON voip_tokens (token);
CREATE INDEX voip_tokens_user_idx ON voip_tokens (user_id);
CREATE INDEX voip_tokens_updated_idx ON voip_tokens (updated_at);
