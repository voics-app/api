-- Push notification tokens of the mobile app (APNs and FCM), one per login session.
-- A token leaves with its session (the row is deleted when the session is revoked or removed)
-- and with its E2EE device, and stale rows are pruned together with stale devices.

CREATE TABLE push_tokens (
    session_id  UUID PRIMARY KEY REFERENCES sessions (id) ON DELETE CASCADE,
    user_id     BIGINT      NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    device_id   BIGINT REFERENCES devices (id) ON DELETE CASCADE,
    platform    TEXT        NOT NULL CHECK (platform IN ('apns', 'fcm')),
    token       TEXT        NOT NULL,
    environment TEXT        NOT NULL CHECK (environment IN ('production', 'sandbox')),
    previews    BOOLEAN     NOT NULL DEFAULT TRUE,
    -- The device-only "only mentions" preference of the app: pushes for this token need a mention.
    only_mentions BOOLEAN   NOT NULL DEFAULT FALSE,
    created_at  TIMESTAMPTZ NOT NULL,
    updated_at  TIMESTAMPTZ NOT NULL
);
-- A provider token belongs to one installation: registering it again moves it.
CREATE UNIQUE INDEX push_tokens_provider_token_idx ON push_tokens (platform, token);
CREATE INDEX push_tokens_user_idx ON push_tokens (user_id);
CREATE INDEX push_tokens_updated_idx ON push_tokens (updated_at);
