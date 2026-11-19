-- Soundboard assets are limited and validated in the application before insertion.
CREATE TABLE soundboard_sounds (
    id BIGINT PRIMARY KEY,
    guild_id BIGINT NOT NULL REFERENCES guilds(id) ON DELETE CASCADE,
    creator_id BIGINT REFERENCES users(id) ON DELETE SET NULL,
    name TEXT NOT NULL CHECK (char_length(name) BETWEEN 2 AND 32),
    volume REAL NOT NULL CHECK (volume >= 0 AND volume <= 1),
    audio BYTEA NOT NULL CHECK (octet_length(audio) BETWEEN 46 AND 480044)
);
CREATE INDEX soundboard_sounds_guild_id_id ON soundboard_sounds(guild_id, id);
