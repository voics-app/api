-- Thread membership is scoped to a current guild member and removed with either
-- the thread or the guild membership. Existing creators remain members.
ALTER TABLE channels ADD COLUMN thread_membership_version BIGINT NOT NULL DEFAULT 0;
CREATE UNIQUE INDEX channels_id_guild_unique ON channels (id, guild_id);

CREATE TABLE thread_members (
    thread_id BIGINT NOT NULL,
    guild_id BIGINT NOT NULL,
    user_id BIGINT NOT NULL,
    joined_at TIMESTAMPTZ NOT NULL,
    PRIMARY KEY (thread_id, user_id),
    FOREIGN KEY (thread_id, guild_id) REFERENCES channels (id, guild_id) ON DELETE CASCADE,
    FOREIGN KEY (guild_id, user_id) REFERENCES members (guild_id, user_id) ON DELETE CASCADE
);

CREATE INDEX thread_members_user_idx ON thread_members (user_id, thread_id);

INSERT INTO thread_members (thread_id, guild_id, user_id, joined_at)
SELECT c.id, c.guild_id, c.owner_id, c.created_at
FROM channels c
JOIN members m ON m.guild_id = c.guild_id AND m.user_id = c.owner_id
WHERE c.kind IN (10, 11, 12)
ON CONFLICT DO NOTHING;
