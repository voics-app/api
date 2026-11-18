-- Channel reports capture server-owned metadata without loading message content.
ALTER TABLE reports DROP CONSTRAINT reports_kind_check,
    ADD CONSTRAINT reports_kind_check CHECK (kind IN ('message', 'user', 'guild', 'channel')),
    ADD CONSTRAINT reports_channel_target_check CHECK (
        kind <> 'channel' OR (
            channel_id IS NOT NULL AND guild_id IS NOT NULL AND message_id IS NULL
            AND delivery_mode IS NULL AND content IS NOT NULL
            AND content_source IS NOT NULL AND content_source = 'server'
        )
    );
CREATE UNIQUE INDEX reports_open_channel_uniq ON reports (reporter_id, channel_id)
    WHERE status = 'open' AND kind = 'channel';
