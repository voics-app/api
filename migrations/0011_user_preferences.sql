-- Synced client preferences and compact message display, per user (client#123).
-- preferences is a sparse JSON object validated by the application (domain::user_preferences).

ALTER TABLE user_settings
    ADD COLUMN message_display_compact BOOLEAN NOT NULL DEFAULT FALSE,
    ADD COLUMN preferences JSONB NOT NULL DEFAULT '{}'::jsonb;
