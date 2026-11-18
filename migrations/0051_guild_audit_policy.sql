ALTER TABLE guilds
    ADD COLUMN audit_log_enabled BOOLEAN NOT NULL DEFAULT true,
    ADD COLUMN audit_log_retention_days SMALLINT NOT NULL DEFAULT 90
        CHECK (audit_log_retention_days IN (30, 90, 365));
CREATE INDEX audit_log_retention_idx ON audit_log (guild_id, created_at);
