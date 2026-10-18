-- Transactional outbox (P6): written in the same transaction as the state
-- change and relayed to Redpanda by the outbox relay.

CREATE TABLE outbox (
    id         BIGSERIAL PRIMARY KEY,
    event_id   UUID        NOT NULL UNIQUE,
    topic      TEXT        NOT NULL,
    key        TEXT        NOT NULL,
    payload    JSONB       NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    sent_at    TIMESTAMPTZ
);
CREATE INDEX outbox_unsent_idx ON outbox (id) WHERE sent_at IS NULL;
