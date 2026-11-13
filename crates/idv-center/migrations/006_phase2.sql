ALTER TABLE images ADD COLUMN content_type TEXT NOT NULL DEFAULT 'full';
ALTER TABLE images ADD COLUMN base_image_id TEXT;

CREATE TABLE IF NOT EXISTS mode_policies (
    id TEXT PRIMARY KEY NOT NULL,
    name TEXT NOT NULL UNIQUE,
    restore TEXT NOT NULL DEFAULT 'reboot',
    system_action TEXT NOT NULL DEFAULT 'restore',
    persistent_action TEXT NOT NULL DEFAULT 'keep',
    volatile_action TEXT NOT NULL DEFAULT 'wipe',
    notes TEXT NOT NULL DEFAULT '',
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS lab_mode_bindings (
    lab_id TEXT PRIMARY KEY NOT NULL REFERENCES labs(id) ON DELETE CASCADE,
    mode_policy_id TEXT NOT NULL REFERENCES mode_policies(id) ON DELETE RESTRICT,
    effective TEXT NOT NULL DEFAULT 'next_boot',
    updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS agent_sessions (
    id TEXT PRIMARY KEY NOT NULL,
    client_id TEXT,
    mac TEXT NOT NULL UNIQUE,
    hostname TEXT NOT NULL DEFAULT '',
    agent_version TEXT NOT NULL DEFAULT '',
    mode_policy_id TEXT,
    last_seen TEXT NOT NULL,
    last_restore_at TEXT,
    status TEXT NOT NULL DEFAULT 'online',
    detail TEXT NOT NULL DEFAULT ''
);

CREATE INDEX IF NOT EXISTS idx_agent_sessions_seen ON agent_sessions(last_seen);
