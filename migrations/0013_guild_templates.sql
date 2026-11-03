-- Server templates (api#44): a shareable snapshot of a guild's settings, roles and channels.
-- Built-in templates live in code (domain::template) and have no row.

CREATE TABLE guild_templates (
    code                    TEXT        PRIMARY KEY,
    guild_id                BIGINT      NOT NULL UNIQUE REFERENCES guilds (id) ON DELETE CASCADE,
    name                    TEXT        NOT NULL,
    description             TEXT,
    usage_count             INTEGER     NOT NULL DEFAULT 0,
    creator_id              BIGINT      REFERENCES users (id) ON DELETE SET NULL,
    serialized_source_guild JSONB       NOT NULL,
    created_at              TIMESTAMPTZ NOT NULL,
    updated_at              TIMESTAMPTZ NOT NULL
);
