-- Active stage instances. The unique channel key makes concurrent starts exclusive.
CREATE TABLE stage_instances (
    id BIGINT PRIMARY KEY,
    guild_id BIGINT NOT NULL REFERENCES guilds(id) ON DELETE CASCADE,
    channel_id BIGINT NOT NULL UNIQUE REFERENCES channels(id) ON DELETE CASCADE,
    topic TEXT NOT NULL CHECK (char_length(topic) BETWEEN 1 AND 120)
);
