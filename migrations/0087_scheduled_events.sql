-- Server events and interested members. Membership deletion also removes interest.
CREATE TABLE guild_scheduled_events (
    id BIGINT PRIMARY KEY,
    guild_id BIGINT NOT NULL REFERENCES guilds (id) ON DELETE CASCADE,
    creator_id BIGINT REFERENCES users (id) ON DELETE SET NULL,
    channel_id BIGINT REFERENCES channels (id) ON DELETE CASCADE,
    name TEXT NOT NULL CHECK (char_length(btrim(name)) BETWEEN 1 AND 100),
    description TEXT CHECK (char_length(description) <= 1000),
    scheduled_start_time TIMESTAMPTZ NOT NULL,
    scheduled_end_time TIMESTAMPTZ,
    status SMALLINT NOT NULL CHECK (status IN (1, 2, 3, 4)),
    entity_type SMALLINT NOT NULL CHECK (entity_type IN (1, 2, 3)),
    location TEXT,
    image TEXT,
    CHECK (scheduled_end_time IS NULL OR scheduled_end_time > scheduled_start_time),
    CHECK ((entity_type IN (1, 2) AND channel_id IS NOT NULL AND location IS NULL)
        OR (entity_type = 3 AND channel_id IS NULL AND scheduled_end_time IS NOT NULL AND location IS NOT NULL
            AND char_length(btrim(location)) BETWEEN 1 AND 100)),
    UNIQUE (guild_id, id)
);
CREATE INDEX guild_scheduled_events_guild_start_idx
    ON guild_scheduled_events (guild_id, scheduled_start_time, id);

CREATE TABLE guild_scheduled_event_users (
    guild_id BIGINT NOT NULL,
    event_id BIGINT NOT NULL,
    user_id BIGINT NOT NULL,
    PRIMARY KEY (event_id, user_id),
    FOREIGN KEY (guild_id, event_id) REFERENCES guild_scheduled_events (guild_id, id) ON DELETE CASCADE,
    FOREIGN KEY (guild_id, user_id) REFERENCES members (guild_id, user_id) ON DELETE CASCADE
);

-- One durable notification claim per event and member. The worker must enqueue
-- notification delivery and mark this row in the same transaction.
CREATE TABLE guild_scheduled_event_reminders (
    event_id BIGINT NOT NULL,
    user_id BIGINT NOT NULL,
    enqueued_at TIMESTAMPTZ NOT NULL,
    PRIMARY KEY (event_id, user_id),
    FOREIGN KEY (event_id, user_id) REFERENCES guild_scheduled_event_users (event_id, user_id) ON DELETE CASCADE
);
