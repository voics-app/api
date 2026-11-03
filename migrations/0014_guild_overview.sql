-- Server overview settings (api#37): system message flags, banner and invite splash.

ALTER TABLE guilds
    ADD COLUMN system_channel_flags INTEGER NOT NULL DEFAULT 0,
    ADD COLUMN banner               TEXT,
    ADD COLUMN splash               TEXT;
