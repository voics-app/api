-- Reports of messages and users, reviewed in the admin portal (api#126).
-- content is the message text copied from the messages service at report time (content_source
-- 'server'), or what the reporter submitted for ephemeral and P2P chats the server never stores
-- (content_source 'reporter'). The reporter is never exposed to the reported user: no
-- user-facing route reads this table.

CREATE TABLE reports (
    id               BIGINT      PRIMARY KEY,
    reporter_id      BIGINT      NOT NULL REFERENCES users (id),
    reported_user_id BIGINT      NOT NULL REFERENCES users (id),
    kind             TEXT        NOT NULL CHECK (kind IN ('message', 'user')),
    category         TEXT        NOT NULL,
    note             TEXT,
    channel_id       BIGINT,
    guild_id         BIGINT,
    message_id       BIGINT,
    delivery_mode    TEXT,
    content          TEXT,
    content_source   TEXT        CHECK (content_source IN ('server', 'reporter')),
    status           TEXT        NOT NULL DEFAULT 'open' CHECK (status IN ('open', 'resolved', 'dismissed')),
    created_at       TIMESTAMPTZ NOT NULL,
    resolved_at      TIMESTAMPTZ,
    resolved_by      TEXT,
    resolution_note  TEXT
);

CREATE INDEX reports_status_idx ON reports (status, id DESC);
CREATE INDEX reports_reported_user_idx ON reports (reported_user_id);

-- One open report per reporter and target: repeating a report is a silent no-op.
CREATE UNIQUE INDEX reports_open_message_uniq ON reports (reporter_id, message_id)
    WHERE status = 'open' AND kind = 'message';
CREATE UNIQUE INDEX reports_open_user_uniq ON reports (reporter_id, reported_user_id)
    WHERE status = 'open' AND kind = 'user';
