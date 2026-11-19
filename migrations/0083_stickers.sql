-- Guild stickers retain immutable IDs and format so existing messages can still resolve
-- their display metadata after removal from the picker. The CDN object is retained.
CREATE TABLE stickers (
    id BIGINT PRIMARY KEY,
    guild_id BIGINT NOT NULL REFERENCES guilds(id) ON DELETE CASCADE,
    creator_id BIGINT REFERENCES users(id) ON DELETE SET NULL,
    name TEXT NOT NULL CHECK (char_length(name) BETWEEN 2 AND 30),
    tags TEXT NOT NULL CHECK (char_length(tags) <= 200),
    format_type SMALLINT NOT NULL CHECK (format_type BETWEEN 1 AND 4),
    available BOOLEAN NOT NULL DEFAULT TRUE
);
CREATE INDEX stickers_available_guild ON stickers (guild_id, id) WHERE available;
