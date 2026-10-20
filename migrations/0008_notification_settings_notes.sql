-- Per-user notification settings per guild and for DMs (api#28) and private user notes (api#51).

-- `guild_id = 0` holds the settings of the user's direct messages (Discord's `@me`), so it has
-- no foreign key to `guilds`; rows of a deleted guild are removed by the delete itself.
CREATE TABLE user_guild_settings (
    user_id             BIGINT   NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    guild_id            BIGINT   NOT NULL,
    -- NULL inherits the guild's default_message_notifications.
    message_notifications SMALLINT CHECK (message_notifications BETWEEN 0 AND 2),
    suppress_everyone   BOOLEAN  NOT NULL DEFAULT FALSE,
    suppress_roles      BOOLEAN  NOT NULL DEFAULT FALSE,
    muted               BOOLEAN  NOT NULL DEFAULT FALSE,
    -- NULL while muted means until turned off.
    mute_end_time       TIMESTAMPTZ,
    hide_muted_channels BOOLEAN  NOT NULL DEFAULT FALSE,
    -- [{channel_id, message_notifications, muted, mute_end_time}]
    channel_overrides   JSONB    NOT NULL DEFAULT '[]'::jsonb,
    updated_at          TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (user_id, guild_id)
);
CREATE INDEX user_guild_settings_guild_idx ON user_guild_settings (guild_id);

CREATE TABLE user_notes (
    owner_id  BIGINT NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    target_id BIGINT NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    note      TEXT   NOT NULL CHECK (char_length(note) BETWEEN 1 AND 256),
    PRIMARY KEY (owner_id, target_id)
);
