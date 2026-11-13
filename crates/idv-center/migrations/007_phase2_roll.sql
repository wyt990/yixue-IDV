ALTER TABLE mode_policies ADD COLUMN schedule_start TEXT;
ALTER TABLE mode_policies ADD COLUMN schedule_end TEXT;
ALTER TABLE mode_policies ADD COLUMN usb_restricted INTEGER NOT NULL DEFAULT 0;

CREATE TABLE IF NOT EXISTS power_commands (
    id TEXT PRIMARY KEY NOT NULL,
    client_id TEXT,
    mac TEXT NOT NULL,
    kind TEXT NOT NULL,
    state TEXT NOT NULL DEFAULT 'pending',
    detail TEXT NOT NULL DEFAULT '',
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_power_mac_state ON power_commands(mac, state);
