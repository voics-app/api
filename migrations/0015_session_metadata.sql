-- Client details and last activity of login sessions, for the sessions list.

ALTER TABLE sessions
    ADD COLUMN client       TEXT        NOT NULL DEFAULT 'Unknown',
    ADD COLUMN os           TEXT        NOT NULL DEFAULT 'Unknown',
    ADD COLUMN location     TEXT,
    ADD COLUMN last_used_at TIMESTAMPTZ;

UPDATE sessions SET last_used_at = created_at;

ALTER TABLE sessions ALTER COLUMN last_used_at SET NOT NULL;
