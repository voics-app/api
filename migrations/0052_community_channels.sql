-- Community is configured only when both required channels are set.
ALTER TABLE guilds ADD COLUMN rules_channel_id BIGINT,
    ADD COLUMN public_updates_channel_id BIGINT,
    ADD CONSTRAINT guild_community_pair CHECK ((rules_channel_id IS NULL) = (public_updates_channel_id IS NULL)),
    ADD CONSTRAINT guild_community_rules_fk FOREIGN KEY (rules_channel_id) REFERENCES channels(id) DEFERRABLE INITIALLY DEFERRED,
    ADD CONSTRAINT guild_community_updates_fk FOREIGN KEY (public_updates_channel_id) REFERENCES channels(id) DEFERRABLE INITIALLY DEFERRED;
