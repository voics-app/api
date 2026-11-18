-- Hash-only evidence for replaying one committed rotation after response loss.
-- The successor is derived from the old bearer token and the request UUID, so
-- neither bearer tokens nor the UUID need to be stored in plaintext.
ALTER TABLE refresh_tokens
    ADD COLUMN rotation_request_hash TEXT,
    ADD COLUMN successor_hash TEXT,
    ADD CONSTRAINT refresh_rotation_recovery_complete CHECK (
        (rotation_request_hash IS NULL AND successor_hash IS NULL)
        OR (rotation_request_hash IS NOT NULL AND successor_hash IS NOT NULL AND rotated_at IS NOT NULL)
    );
