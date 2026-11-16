-- 2026-11-16: instant agent command channel + online/restore state columns.
CREATE TABLE IF NOT EXISTS agent_commands (
    id TEXT PRIMARY KEY NOT NULL,
    mac TEXT NOT NULL,
    kind TEXT NOT NULL,
    payload TEXT NOT NULL DEFAULT '',
    state TEXT NOT NULL DEFAULT 'pending',
    created_by TEXT NOT NULL DEFAULT '',
    created_at TEXT NOT NULL,
    delivered_at TEXT,
    finished_at TEXT,
    error_message TEXT NOT NULL DEFAULT ''
);
CREATE INDEX IF NOT EXISTS idx_agent_commands_mac_state ON agent_commands(mac, state);

ALTER TABLE agent_sessions ADD COLUMN frozen INTEGER NOT NULL DEFAULT 0;
ALTER TABLE agent_sessions ADD COLUMN armed INTEGER NOT NULL DEFAULT 0;
ALTER TABLE agent_sessions ADD COLUMN mode_name TEXT NOT NULL DEFAULT '';
ALTER TABLE agent_sessions ADD COLUMN current_hostname TEXT NOT NULL DEFAULT '';
ALTER TABLE agent_sessions ADD COLUMN identity_state TEXT NOT NULL DEFAULT '';
ALTER TABLE agent_sessions ADD COLUMN last_error TEXT NOT NULL DEFAULT '';
