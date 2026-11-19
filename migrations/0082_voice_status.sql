-- Kept separate from channel metadata writes so unrelated edits cannot reset it.
ALTER TABLE channels ADD COLUMN status TEXT
    CHECK (status IS NULL OR char_length(status) <= 500);
ALTER TABLE guilds ADD COLUMN widget_enabled BOOLEAN NOT NULL DEFAULT FALSE;
ALTER TABLE guilds ADD COLUMN widget_channel_id BIGINT REFERENCES channels(id) ON DELETE SET NULL;

-- Counts only actual membership inserts/deletes in the transaction that
-- changes membership. Other feeds must be committed before insights is exposed.
CREATE TABLE guild_daily_activity (
    guild_id BIGINT NOT NULL REFERENCES guilds(id) ON DELETE CASCADE,
    day DATE NOT NULL,
    joins BIGINT NOT NULL DEFAULT 0 CHECK (joins >= 0),
    leaves BIGINT NOT NULL DEFAULT 0 CHECK (leaves >= 0),
    messages BIGINT NOT NULL DEFAULT 0 CHECK (messages >= 0),
    voice_seconds BIGINT NOT NULL DEFAULT 0 CHECK (voice_seconds >= 0),
    PRIMARY KEY (guild_id, day)
);

CREATE TABLE guild_activity_collection (
    singleton BOOLEAN PRIMARY KEY DEFAULT TRUE CHECK (singleton),
    started_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
INSERT INTO guild_activity_collection DEFAULT VALUES;

CREATE FUNCTION count_guild_membership_change() RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE
    affected_guild BIGINT;
BEGIN
    affected_guild := CASE WHEN TG_OP = 'INSERT' THEN NEW.guild_id ELSE OLD.guild_id END;
    -- A guild cascade is deletion of the aggregate itself, not member churn.
    IF NOT EXISTS (SELECT 1 FROM guilds WHERE id = affected_guild) THEN
        RETURN NULL;
    END IF;
    INSERT INTO guild_daily_activity (guild_id, day, joins, leaves)
    VALUES (affected_guild, (transaction_timestamp() AT TIME ZONE 'UTC')::date,
            CASE WHEN TG_OP = 'INSERT' THEN 1 ELSE 0 END,
            CASE WHEN TG_OP = 'DELETE' THEN 1 ELSE 0 END)
    ON CONFLICT (guild_id, day) DO UPDATE
       SET joins = guild_daily_activity.joins + EXCLUDED.joins,
           leaves = guild_daily_activity.leaves + EXCLUDED.leaves;
    RETURN NULL;
END;
$$;

CREATE TRIGGER count_guild_member_insert AFTER INSERT ON members
    FOR EACH ROW EXECUTE FUNCTION count_guild_membership_change();
CREATE TRIGGER count_guild_member_delete AFTER DELETE ON members
    FOR EACH ROW EXECUTE FUNCTION count_guild_membership_change();
