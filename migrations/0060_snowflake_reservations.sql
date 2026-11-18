-- Permanent watermarks; never delete these rows while issuers can run.
-- Workers0 and64 remain reserved for legacy static-ID binaries.
CREATE TABLE snowflake_reservations (
    worker_id smallint PRIMARY KEY CHECK (worker_id BETWEEN 0 AND 127),
    owner_id uuid UNIQUE,
    start_ms bigint NOT NULL DEFAULT 0 CHECK (start_ms >= 0),
    end_ms bigint NOT NULL DEFAULT 0 CHECK (end_ms >= start_ms),
    issued_at_ms bigint NOT NULL DEFAULT 0 CHECK (issued_at_ms >= 0)
);
INSERT INTO snowflake_reservations(worker_id) SELECT generate_series(0,127);
CREATE TABLE snowflake_allocation_guard (
    singleton boolean PRIMARY KEY DEFAULT true CHECK (singleton),
    not_before_ms bigint NOT NULL DEFAULT 0 CHECK (not_before_ms >= 0)
);
INSERT INTO snowflake_allocation_guard(singleton) VALUES (true);
