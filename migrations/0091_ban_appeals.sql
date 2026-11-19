CREATE TABLE ban_appeal_forms (
    guild_id BIGINT PRIMARY KEY REFERENCES guilds(id) ON DELETE CASCADE,
    question TEXT NOT NULL CHECK (char_length(question) BETWEEN 1 AND 500)
);
CREATE TABLE ban_appeals (
    id BIGINT PRIMARY KEY,
    guild_id BIGINT NOT NULL REFERENCES guilds(id) ON DELETE CASCADE,
    user_id BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    ban_created_at TIMESTAMPTZ NOT NULL,
    question TEXT NOT NULL,
    answer TEXT NOT NULL CHECK (char_length(answer) BETWEEN 1 AND 2000),
    status TEXT NOT NULL DEFAULT 'open' CHECK (status IN ('open', 'accepted', 'denied', 'closed')),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    resolved_at TIMESTAMPTZ,
    resolved_by BIGINT REFERENCES users(id) ON DELETE SET NULL
);
CREATE UNIQUE INDEX ban_appeals_one_open ON ban_appeals(guild_id, user_id) WHERE status = 'open';
