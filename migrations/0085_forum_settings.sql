ALTER TABLE channels
    ADD COLUMN forum_settings JSONB NOT NULL DEFAULT '{}'::jsonb,
    ADD COLUMN applied_tags TEXT[] NOT NULL DEFAULT '{}';
