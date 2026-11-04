-- Personal pins per user, synced across devices (client#222, client#223).
-- pinned_dm_channel_ids is ordered, oldest pin first; pinned_channel_ids maps a guild id to its ordered pinned channel ids.

ALTER TABLE user_settings
    ADD COLUMN pinned_dm_channel_ids BIGINT[] NOT NULL DEFAULT '{}',
    ADD COLUMN pinned_channel_ids JSONB NOT NULL DEFAULT '{}'::jsonb;
