-- Profile "About me" text, at most 300 characters (api#224, client#224).

ALTER TABLE users
    ADD COLUMN bio TEXT CHECK (bio IS NULL OR char_length(bio) <= 300);
