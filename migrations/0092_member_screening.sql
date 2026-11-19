CREATE TABLE guild_member_screening (
    guild_id BIGINT PRIMARY KEY REFERENCES guilds(id) ON DELETE CASCADE,
    enabled BOOLEAN NOT NULL DEFAULT FALSE,
    rules TEXT[] NOT NULL DEFAULT '{}',
    revision BIGINT NOT NULL DEFAULT 0
);

INSERT INTO guild_member_screening (guild_id) SELECT id FROM guilds;

CREATE FUNCTION create_guild_member_screening() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    INSERT INTO guild_member_screening (guild_id) VALUES (NEW.id);
    RETURN NEW;
END;
$$;
CREATE TRIGGER guild_member_screening_create AFTER INSERT ON guilds
    FOR EACH ROW EXECUTE FUNCTION create_guild_member_screening();
