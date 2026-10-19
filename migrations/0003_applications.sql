-- Applications, bots, commands, interactions and webhooks.

CREATE TABLE applications (
    id                        BIGINT PRIMARY KEY,
    name                      TEXT        NOT NULL,
    description               TEXT        NOT NULL DEFAULT '',
    icon                      TEXT,
    owner_id                  BIGINT      NOT NULL REFERENCES users (id),
    bot_user_id               BIGINT      NOT NULL UNIQUE REFERENCES users (id),
    bot_public                BOOLEAN     NOT NULL DEFAULT TRUE,
    bot_require_code_grant    BOOLEAN     NOT NULL DEFAULT FALSE,
    verify_key                TEXT        NOT NULL,
    flags                     BIGINT      NOT NULL DEFAULT 0,
    interactions_endpoint_url TEXT,
    created_at                TIMESTAMPTZ NOT NULL
);
CREATE INDEX applications_owner_idx ON applications (owner_id);

CREATE TABLE bot_tokens (
    application_id BIGINT PRIMARY KEY REFERENCES applications (id) ON DELETE CASCADE,
    bot_user_id    BIGINT      NOT NULL,
    token_hash     TEXT        NOT NULL UNIQUE,
    created_at     TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE application_commands (
    id                         BIGINT PRIMARY KEY,
    application_id             BIGINT   NOT NULL REFERENCES applications (id) ON DELETE CASCADE,
    guild_id                   BIGINT REFERENCES guilds (id) ON DELETE CASCADE,
    name                       TEXT     NOT NULL,
    description                TEXT     NOT NULL DEFAULT '',
    kind                       SMALLINT NOT NULL DEFAULT 1,
    options                    JSONB    NOT NULL DEFAULT '[]',
    default_member_permissions BIGINT,
    dm_permission              BOOLEAN  NOT NULL DEFAULT TRUE,
    nsfw                       BOOLEAN  NOT NULL DEFAULT FALSE,
    version                    BIGINT   NOT NULL
);
CREATE UNIQUE INDEX application_commands_scope_name_key
    ON application_commands (application_id, COALESCE(guild_id, 0), kind, name);

CREATE TABLE interactions (
    id                  BIGINT PRIMARY KEY,
    application_id      BIGINT      NOT NULL REFERENCES applications (id) ON DELETE CASCADE,
    kind                SMALLINT    NOT NULL,
    guild_id            BIGINT,
    channel_id          BIGINT      NOT NULL,
    user_id             BIGINT      NOT NULL,
    token_hash          TEXT        NOT NULL,
    data                JSONB       NOT NULL DEFAULT '{}',
    state               TEXT        NOT NULL DEFAULT 'pending'
        CHECK (state IN ('pending', 'deferred', 'responded')),
    original_message_id BIGINT,
    source_message_id   BIGINT,
    created_at          TIMESTAMPTZ NOT NULL
);
CREATE INDEX interactions_token_idx ON interactions (application_id, token_hash);
CREATE INDEX interactions_created_idx ON interactions (created_at);

CREATE TABLE webhooks (
    id             BIGINT PRIMARY KEY,
    kind           SMALLINT NOT NULL,
    guild_id       BIGINT   NOT NULL REFERENCES guilds (id) ON DELETE CASCADE,
    channel_id     BIGINT   NOT NULL REFERENCES channels (id) ON DELETE CASCADE,
    creator_id     BIGINT,
    name           TEXT     NOT NULL,
    avatar         TEXT,
    application_id BIGINT,
    token_hash     TEXT     NOT NULL
);
CREATE INDEX webhooks_guild_idx ON webhooks (guild_id);
CREATE INDEX webhooks_channel_idx ON webhooks (channel_id);
