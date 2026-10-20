-- Device activity (api#25): `last_seen_at` is bumped when the device's session refreshes its
-- tokens, and devices idle for longer than DEVICE_MAX_IDLE_DAYS are hidden from E2EE listings and
-- pruned. Existing devices start fresh so nobody loses a device the moment this migration runs.

ALTER TABLE devices
    ADD COLUMN last_seen_at TIMESTAMPTZ NOT NULL DEFAULT now();
CREATE INDEX devices_last_seen_idx ON devices (last_seen_at);
