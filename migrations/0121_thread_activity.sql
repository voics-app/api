-- Existing threads receive a conservative grace period on upgrade: their
-- historical messages live in the messages service and are not in Postgres.
CREATE TABLE thread_activity (
    thread_id BIGINT PRIMARY KEY REFERENCES channels (id) ON DELETE CASCADE,
    last_activity_at TIMESTAMPTZ NOT NULL,
    automatically_archived BOOLEAN NOT NULL DEFAULT FALSE
);

INSERT INTO thread_activity (thread_id, last_activity_at)
SELECT id, GREATEST(created_at, CURRENT_TIMESTAMP)
FROM channels
WHERE kind IN (10, 11, 12);

CREATE FUNCTION initialize_thread_activity() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    INSERT INTO thread_activity (thread_id, last_activity_at)
    VALUES (NEW.id, NEW.created_at)
    ON CONFLICT (thread_id) DO NOTHING;
    RETURN NEW;
END;
$$;

CREATE TRIGGER initialize_thread_activity_after_insert
AFTER INSERT ON channels
FOR EACH ROW WHEN (NEW.kind IN (10, 11, 12))
EXECUTE FUNCTION initialize_thread_activity();

CREATE FUNCTION mark_thread_archive_manual() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    UPDATE thread_activity SET automatically_archived = FALSE WHERE thread_id = NEW.id;
    RETURN NEW;
END;
$$;

CREATE TRIGGER mark_thread_archive_manual_after_update
AFTER UPDATE OF thread_metadata ON channels
FOR EACH ROW WHEN ((OLD.thread_metadata->>'archived') IS DISTINCT FROM (NEW.thread_metadata->>'archived'))
EXECUTE FUNCTION mark_thread_archive_manual();
