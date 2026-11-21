-- A trigger advances the revision for every writer, including older API binaries.
-- Older binaries must still be fenced before enabling privacy settings: they do not compare it.
ALTER TABLE user_settings ADD COLUMN settings_revision BIGINT NOT NULL DEFAULT 0;
CREATE FUNCTION bump_user_settings_revision() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    NEW.settings_revision := OLD.settings_revision + 1;
    RETURN NEW;
END;
$$;
CREATE TRIGGER user_settings_revision_before_update
    BEFORE UPDATE ON user_settings FOR EACH ROW EXECUTE FUNCTION bump_user_settings_revision();
