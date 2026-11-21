-- Existing accounts retain the previous policy of accepting requests from everyone.
ALTER TABLE user_settings
    ADD COLUMN friend_source_all BOOLEAN NOT NULL DEFAULT TRUE,
    ADD COLUMN friend_source_mutual_friends BOOLEAN NOT NULL DEFAULT FALSE,
    ADD COLUMN friend_source_mutual_guilds BOOLEAN NOT NULL DEFAULT FALSE;
