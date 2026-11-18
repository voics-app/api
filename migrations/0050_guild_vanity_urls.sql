CREATE TABLE guild_vanity_urls (
    guild_id BIGINT PRIMARY KEY REFERENCES guilds(id) ON DELETE CASCADE,
    code TEXT NOT NULL UNIQUE CHECK (code ~ '^[a-z0-9-]{2,32}$'),
    uses INTEGER NOT NULL DEFAULT 0 CHECK (uses >= 0),
    created_at TIMESTAMPTZ NOT NULL
);

-- Ordinary invitations keep their existing case-sensitive semantics. This
-- index supports collision checks against the case-insensitive vanity namespace.
CREATE INDEX invites_code_lower_idx ON invites (lower(code));
