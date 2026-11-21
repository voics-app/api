ALTER TABLE channels
    ADD COLUMN forum_settings JSONB NOT NULL DEFAULT '{}'::jsonb,
    ADD COLUMN applied_tags TEXT[] NOT NULL DEFAULT '{}';

CREATE UNIQUE INDEX forum_post_request_unique ON channels (parent_id, owner_id, (forum_settings->>'request_key')) WHERE kind = 11 AND forum_settings ? 'request_key';
