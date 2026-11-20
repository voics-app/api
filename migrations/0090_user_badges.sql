-- Grants are managed by trusted operators; account owners control visibility.
CREATE TABLE user_badges (
    user_id BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    badge_id TEXT NOT NULL CHECK (badge_id ~ '^[a-z0-9][a-z0-9_-]{0,63}$'),
    visible BOOLEAN NOT NULL DEFAULT TRUE,
    PRIMARY KEY (user_id, badge_id)
);
