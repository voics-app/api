-- Voice channels carry a text chat; admins may switch it off per channel (api#23).

ALTER TABLE channels
    ADD COLUMN text_chat_enabled BOOLEAN NOT NULL DEFAULT TRUE;
