-- Server rail order and folders, per user, synced across devices (client#58).
-- guild_folders is an ordered JSON array of {id, name, color, guild_ids}; guild_positions is the flat mirror.

ALTER TABLE user_settings
    ADD COLUMN guild_folders JSONB NOT NULL DEFAULT '[]'::jsonb,
    ADD COLUMN guild_positions BIGINT[] NOT NULL DEFAULT '{}';
