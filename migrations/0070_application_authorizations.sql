-- Installation identity is separate from a user's explicit consent. The private
-- generation changes on reinstall, so a stale action cannot target its replacement.
CREATE TABLE guild_application_installations (
    generation BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    guild_id BIGINT NOT NULL REFERENCES guilds(id) ON DELETE CASCADE,
    application_id BIGINT NOT NULL REFERENCES applications(id) ON DELETE CASCADE,
    bot_user_id BIGINT NOT NULL,
    installed_at TIMESTAMPTZ NOT NULL,
    UNIQUE (guild_id, application_id),
    FOREIGN KEY (guild_id, bot_user_id) REFERENCES members(guild_id, user_id) ON DELETE CASCADE
);

-- Existing bot memberships are real installations; no consenting user is invented.
INSERT INTO guild_application_installations (guild_id, application_id, bot_user_id, installed_at)
SELECT m.guild_id, a.id, a.bot_user_id, m.joined_at
FROM applications a JOIN members m ON m.user_id = a.bot_user_id;

-- Preserve the last generation after removal. Initial absence and absence after
-- an intervening install/remove must not compare equal for an in-flight request.
CREATE TABLE application_installation_versions (
    guild_id BIGINT NOT NULL REFERENCES guilds(id) ON DELETE CASCADE,
    application_id BIGINT NOT NULL REFERENCES applications(id) ON DELETE CASCADE,
    last_generation BIGINT NOT NULL,
    PRIMARY KEY (guild_id, application_id)
);
INSERT INTO application_installation_versions (guild_id, application_id, last_generation)
SELECT guild_id, application_id, generation FROM guild_application_installations;

CREATE TABLE application_authorizations (
    id BIGINT PRIMARY KEY,
    user_id BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    installation_generation BIGINT NOT NULL REFERENCES guild_application_installations(generation) ON DELETE CASCADE,
    scopes TEXT[] NOT NULL CHECK (scopes @> ARRAY['bot']::TEXT[] AND scopes <@ ARRAY['bot', 'applications.commands']::TEXT[]),
    permissions BIGINT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL,
    authorized_at TIMESTAMPTZ NOT NULL,
    UNIQUE (user_id, installation_generation)
);
CREATE INDEX application_authorizations_installation_idx ON application_authorizations(installation_generation);
