-- Account setting: every call the user starts or joins is end-to-end encrypted (voice#8).

ALTER TABLE user_settings
    ADD COLUMN e2ee_calls_default BOOLEAN NOT NULL DEFAULT FALSE;
