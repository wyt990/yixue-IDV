-- Phase 3 / stage 7
ALTER TABLE admins ADD COLUMN role TEXT NOT NULL DEFAULT 'super';

CREATE TABLE IF NOT EXISTS client_tags (
    client_id TEXT NOT NULL REFERENCES clients(id) ON DELETE CASCADE,
    tag TEXT NOT NULL,
    PRIMARY KEY (client_id, tag)
);
CREATE INDEX IF NOT EXISTS idx_client_tags_tag ON client_tags(tag);

CREATE TABLE IF NOT EXISTS ops_settings (
    key TEXT PRIMARY KEY NOT NULL,
    value TEXT NOT NULL DEFAULT '',
    updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS software_packages (
    id TEXT PRIMARY KEY NOT NULL,
    name TEXT NOT NULL,
    version_label TEXT NOT NULL DEFAULT '',
    checksum_sha256 TEXT NOT NULL DEFAULT '',
    size_bytes INTEGER NOT NULL DEFAULT 0,
    storage_uri TEXT NOT NULL DEFAULT '',
    notes TEXT NOT NULL DEFAULT '',
    created_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS software_jobs (
    id TEXT PRIMARY KEY NOT NULL,
    package_id TEXT NOT NULL REFERENCES software_packages(id) ON DELETE CASCADE,
    mac TEXT NOT NULL,
    state TEXT NOT NULL DEFAULT 'pending',
    detail TEXT NOT NULL DEFAULT '',
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_software_jobs_mac_state ON software_jobs(mac, state);

CREATE TABLE IF NOT EXISTS assist_sessions (
    id TEXT PRIMARY KEY NOT NULL,
    mac TEXT NOT NULL,
    kind TEXT NOT NULL DEFAULT 'view',
    state TEXT NOT NULL DEFAULT 'pending',
    requested_by TEXT NOT NULL DEFAULT '',
    detail TEXT NOT NULL DEFAULT '',
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_assist_mac_state ON assist_sessions(mac, state);

CREATE TABLE IF NOT EXISTS power_schedules (
    id TEXT PRIMARY KEY NOT NULL,
    name TEXT NOT NULL,
    kind TEXT NOT NULL,
    mac_or_tag TEXT NOT NULL,
    target_type TEXT NOT NULL DEFAULT 'mac',
    minute_utc INTEGER NOT NULL,
    hour_utc INTEGER NOT NULL,
    enabled INTEGER NOT NULL DEFAULT 1,
    last_fired_date TEXT NOT NULL DEFAULT '',
    created_at TEXT NOT NULL
);
