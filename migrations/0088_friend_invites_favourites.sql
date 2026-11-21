CREATE TABLE friend_favourites (
    user_id BIGINT NOT NULL,
    target_id BIGINT NOT NULL,
    PRIMARY KEY (user_id, target_id),
    FOREIGN KEY (user_id, target_id) REFERENCES relationships (user_id, target_id) ON DELETE CASCADE
);
CREATE TABLE friend_invites (
    token_hash TEXT PRIMARY KEY,
    owner_id BIGINT NOT NULL UNIQUE REFERENCES users(id) ON DELETE CASCADE,
    expires_at TIMESTAMPTZ NOT NULL
);
CREATE INDEX friend_invites_owner_idx ON friend_invites (owner_id);
