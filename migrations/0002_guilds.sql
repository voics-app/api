-- Guilds, channels, roles, members, invites, bans, emojis and the audit log.

CREATE TABLE guilds (
    id                            BIGINT PRIMARY KEY,
    name                          TEXT        NOT NULL,
    icon                          TEXT,
    description                   TEXT,
    owner_id                      BIGINT      NOT NULL REFERENCES users (id),
    voice_region                  TEXT,
    verification_level            SMALLINT    NOT NULL DEFAULT 0,
    default_message_notifications SMALLINT    NOT NULL DEFAULT 0,
    explicit_content_filter       SMALLINT    NOT NULL DEFAULT 0,
    afk_channel_id                BIGINT,
    afk_timeout                   INTEGER     NOT NULL DEFAULT 300,
    system_channel_id             BIGINT,
    created_at                    TIMESTAMPTZ NOT NULL
);
CREATE INDEX guilds_owner_idx ON guilds (owner_id);

CREATE TABLE channels (
    id                  BIGINT PRIMARY KEY,
    kind                SMALLINT    NOT NULL,
    guild_id            BIGINT REFERENCES guilds (id) ON DELETE CASCADE,
    position            INTEGER     NOT NULL DEFAULT 0,
    name                TEXT,
    topic               TEXT,
    nsfw                BOOLEAN     NOT NULL DEFAULT FALSE,
    bitrate             INTEGER,
    user_limit          INTEGER,
    rate_limit_per_user INTEGER     NOT NULL DEFAULT 0,
    parent_id           BIGINT,
    owner_id            BIGINT,
    icon                TEXT,
    delivery_mode       TEXT        NOT NULL DEFAULT 'standard'
        CHECK (delivery_mode IN ('standard', 'ephemeral', 'p2p')),
    voice_region        TEXT,
    thread_metadata     JSONB,
    created_at          TIMESTAMPTZ NOT NULL
);
CREATE INDEX channels_guild_idx ON channels (guild_id) WHERE guild_id IS NOT NULL;
CREATE INDEX channels_parent_idx ON channels (parent_id) WHERE parent_id IS NOT NULL;

CREATE TABLE channel_recipients (
    channel_id BIGINT NOT NULL REFERENCES channels (id) ON DELETE CASCADE,
    user_id    BIGINT NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    PRIMARY KEY (channel_id, user_id)
);
CREATE INDEX channel_recipients_user_idx ON channel_recipients (user_id);

CREATE TABLE permission_overwrites (
    channel_id BIGINT   NOT NULL REFERENCES channels (id) ON DELETE CASCADE,
    target_id  BIGINT   NOT NULL,
    kind       SMALLINT NOT NULL CHECK (kind IN (0, 1)),
    allow_bits BIGINT   NOT NULL,
    deny_bits  BIGINT   NOT NULL,
    PRIMARY KEY (channel_id, target_id)
);

CREATE TABLE channel_delivery_overrides (
    user_id    BIGINT NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    channel_id BIGINT NOT NULL REFERENCES channels (id) ON DELETE CASCADE,
    mode       TEXT   NOT NULL CHECK (mode IN ('standard', 'ephemeral', 'p2p')),
    PRIMARY KEY (user_id, channel_id)
);

CREATE TABLE roles (
    id            BIGINT PRIMARY KEY,
    guild_id      BIGINT   NOT NULL REFERENCES guilds (id) ON DELETE CASCADE,
    name          TEXT     NOT NULL,
    color         INTEGER  NOT NULL DEFAULT 0,
    hoist         BOOLEAN  NOT NULL DEFAULT FALSE,
    icon          TEXT,
    unicode_emoji TEXT,
    position      INTEGER  NOT NULL DEFAULT 0,
    permissions   BIGINT   NOT NULL DEFAULT 0,
    managed       BOOLEAN  NOT NULL DEFAULT FALSE,
    mentionable   BOOLEAN  NOT NULL DEFAULT FALSE,
    bot_id        BIGINT
);
CREATE INDEX roles_guild_idx ON roles (guild_id);

CREATE TABLE members (
    guild_id                     BIGINT      NOT NULL REFERENCES guilds (id) ON DELETE CASCADE,
    user_id                      BIGINT      NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    nick                         TEXT,
    avatar                       TEXT,
    joined_at                    TIMESTAMPTZ NOT NULL,
    deaf                         BOOLEAN     NOT NULL DEFAULT FALSE,
    mute                         BOOLEAN     NOT NULL DEFAULT FALSE,
    communication_disabled_until TIMESTAMPTZ,
    pending                      BOOLEAN     NOT NULL DEFAULT FALSE,
    PRIMARY KEY (guild_id, user_id)
);
CREATE INDEX members_user_idx ON members (user_id);

CREATE TABLE member_roles (
    guild_id BIGINT NOT NULL,
    user_id  BIGINT NOT NULL,
    role_id  BIGINT NOT NULL REFERENCES roles (id) ON DELETE CASCADE,
    PRIMARY KEY (guild_id, user_id, role_id),
    FOREIGN KEY (guild_id, user_id) REFERENCES members (guild_id, user_id) ON DELETE CASCADE
);
CREATE INDEX member_roles_role_idx ON member_roles (role_id);

CREATE TABLE bans (
    guild_id   BIGINT      NOT NULL REFERENCES guilds (id) ON DELETE CASCADE,
    user_id    BIGINT      NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    reason     TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (guild_id, user_id)
);

CREATE TABLE invites (
    code       TEXT PRIMARY KEY,
    guild_id   BIGINT      NOT NULL REFERENCES guilds (id) ON DELETE CASCADE,
    channel_id BIGINT      NOT NULL REFERENCES channels (id) ON DELETE CASCADE,
    inviter_id BIGINT      REFERENCES users (id) ON DELETE SET NULL,
    max_age    INTEGER     NOT NULL DEFAULT 0,
    max_uses   INTEGER     NOT NULL DEFAULT 0,
    uses       INTEGER     NOT NULL DEFAULT 0,
    temporary  BOOLEAN     NOT NULL DEFAULT FALSE,
    created_at TIMESTAMPTZ NOT NULL
);
CREATE INDEX invites_guild_idx ON invites (guild_id);

CREATE TABLE emojis (
    id             BIGINT PRIMARY KEY,
    guild_id       BIGINT  NOT NULL REFERENCES guilds (id) ON DELETE CASCADE,
    name           TEXT    NOT NULL,
    creator_id     BIGINT,
    animated       BOOLEAN NOT NULL DEFAULT FALSE,
    available      BOOLEAN NOT NULL DEFAULT TRUE,
    require_colons BOOLEAN NOT NULL DEFAULT TRUE,
    managed        BOOLEAN NOT NULL DEFAULT FALSE
);
CREATE INDEX emojis_guild_idx ON emojis (guild_id);

CREATE TABLE emoji_roles (
    emoji_id BIGINT NOT NULL REFERENCES emojis (id) ON DELETE CASCADE,
    role_id  BIGINT NOT NULL REFERENCES roles (id) ON DELETE CASCADE,
    PRIMARY KEY (emoji_id, role_id)
);

CREATE TABLE audit_log (
    id         BIGINT PRIMARY KEY,
    guild_id   BIGINT      NOT NULL REFERENCES guilds (id) ON DELETE CASCADE,
    user_id    BIGINT,
    target_id  BIGINT,
    action     SMALLINT    NOT NULL,
    changes    JSONB       NOT NULL DEFAULT '[]',
    reason     TEXT,
    created_at TIMESTAMPTZ NOT NULL
);
CREATE INDEX audit_log_guild_idx ON audit_log (guild_id, id DESC);
