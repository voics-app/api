-- A channel-following webhook has no reusable bearer token.  Deleting either
-- channel or the webhook removes the relationship and cancels pending deliveries.
CREATE TABLE announcement_followers (
    source_channel_id BIGINT NOT NULL REFERENCES channels (id) ON DELETE CASCADE,
    target_channel_id BIGINT NOT NULL REFERENCES channels (id) ON DELETE CASCADE,
    webhook_id BIGINT NOT NULL UNIQUE REFERENCES webhooks (id) ON DELETE CASCADE,
    PRIMARY KEY (source_channel_id, target_channel_id),
    CHECK (source_channel_id <> target_channel_id)
);

CREATE INDEX announcement_followers_target_idx ON announcement_followers (target_channel_id);

-- A source channel can disappear without deleting the target channel. Remove
-- its follower webhook as well, so it cannot remain as a dangling integration.
CREATE FUNCTION delete_announcement_follower_webhook() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    UPDATE announcement_deliveries SET state = 'cancelled'
      WHERE source_channel_id = OLD.source_channel_id
        AND target_channel_id = OLD.target_channel_id AND state = 'pending';
    DELETE FROM webhooks WHERE id = OLD.webhook_id;
    RETURN OLD;
END;
$$;
CREATE TRIGGER announcement_follower_delete_webhook
    AFTER DELETE ON announcement_followers FOR EACH ROW
    EXECUTE FUNCTION delete_announcement_follower_webhook();

-- Every destination has one stable delivery identity. A claimed send whose
-- outcome is uncertain must not be retried until the destination write can be
-- checked using that identity.
CREATE TABLE announcement_publications (
    source_channel_id BIGINT NOT NULL,
    source_message_id BIGINT NOT NULL,
    payload JSONB NOT NULL,
    marked BOOLEAN NOT NULL DEFAULT false,
    marker_checked_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (source_channel_id, source_message_id)
);

CREATE TABLE announcement_deliveries (
    source_channel_id BIGINT NOT NULL,
    source_message_id BIGINT NOT NULL,
    target_channel_id BIGINT NOT NULL,
    destination_message_id BIGINT NOT NULL UNIQUE,
    webhook_id BIGINT NOT NULL,
    state TEXT NOT NULL DEFAULT 'pending' CHECK (state IN ('pending', 'claimed', 'delivered', 'unknown', 'cancelled')),
    claimed_at TIMESTAMPTZ,
    checked_at TIMESTAMPTZ,
    PRIMARY KEY (source_channel_id, source_message_id, target_channel_id),
    FOREIGN KEY (source_channel_id, source_message_id)
        REFERENCES announcement_publications (source_channel_id, source_message_id) ON DELETE CASCADE
);

CREATE INDEX announcement_deliveries_pending_idx
    ON announcement_deliveries (source_channel_id, source_message_id, target_channel_id)
    WHERE state = 'pending';
