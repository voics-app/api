-- Server setting that restricts member pruning to administrators (api#101).

ALTER TABLE guilds
    ADD COLUMN prune_admin_only BOOLEAN NOT NULL DEFAULT FALSE;

-- Audit log lookups by target, for the history of members who have left or were banned.
CREATE INDEX audit_log_target_idx ON audit_log (guild_id, target_id, id DESC) WHERE target_id IS NOT NULL;
