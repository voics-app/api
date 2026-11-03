-- Opt-in public server discovery (api#74). A row means the guild is listed in Discover.
-- Description comes from guilds.description; member/online counts and NSFW exclusion are computed at query time.

CREATE TABLE guild_discovery (
    guild_id  BIGINT      PRIMARY KEY REFERENCES guilds (id) ON DELETE CASCADE,
    category  TEXT        NOT NULL,
    tags      TEXT[]      NOT NULL DEFAULT '{}',
    listed_at TIMESTAMPTZ NOT NULL
);
CREATE INDEX guild_discovery_category_idx ON guild_discovery (category);
CREATE INDEX guild_discovery_tags_idx ON guild_discovery USING GIN (tags);
