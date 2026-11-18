-- PRECONDITION: stop all API/messages ID issuers and capture their live MAX(end_ms)
-- before restoring the DB. Pass that captured integer as psql -v previous_end_ms=...
-- If current UTC rolled back and the previous watermark is unavailable, do not restart.
-- No default is supplied: missing/invalid previous_end_ms aborts the transaction.
BEGIN;
-- Zero is not accepted: an unused allocator still needs an explicit trusted bound.
CREATE TEMP TABLE restore_id_input (
    previous_end_ms bigint NOT NULL
        CHECK (previous_end_ms BETWEEN 1767225600000 AND 3966248855551)
) ON COMMIT DROP;
INSERT INTO restore_id_input VALUES (:'previous_end_ms'::bigint);
DO $$
BEGIN
    PERFORM singleton FROM snowflake_allocation_guard FOR UPDATE;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'ID allocation guard is missing';
    END IF;
END $$;
UPDATE snowflake_allocation_guard
SET not_before_ms = GREATEST(
    not_before_ms,
    (SELECT previous_end_ms FROM restore_id_input),
    (SELECT COALESCE(MAX(end_ms),0) FROM snowflake_reservations),
    floor(extract(epoch FROM clock_timestamp()) * 1000)::bigint + 30000
);
UPDATE snowflake_reservations
SET owner_id=NULL, start_ms=0,
    end_ms=GREATEST(end_ms,(SELECT not_before_ms FROM snowflake_allocation_guard));
COMMIT;
SELECT not_before_ms, to_timestamp(not_before_ms/1000.0) AS wait_until_utc
FROM snowflake_allocation_guard;
