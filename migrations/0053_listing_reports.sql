-- Retain public listing evidence after a guild is changed or deleted.
ALTER TABLE reports DROP CONSTRAINT reports_kind_check,
    ADD CONSTRAINT reports_kind_check CHECK (kind IN ('message', 'user', 'guild')),
    ADD CONSTRAINT reports_guild_target_check CHECK (kind <> 'guild' OR (guild_id IS NOT NULL AND content_source IS NOT NULL AND content_source = 'server' AND content IS NOT NULL));
CREATE UNIQUE INDEX reports_open_guild_uniq ON reports (reporter_id, guild_id)
    WHERE status = 'open' AND kind = 'guild';
