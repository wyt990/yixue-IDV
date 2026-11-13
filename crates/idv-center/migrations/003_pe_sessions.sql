CREATE TABLE IF NOT EXISTS pe_sessions (
    id TEXT PRIMARY KEY NOT NULL,
    edge_id TEXT NOT NULL,
    mac TEXT NOT NULL,
    source TEXT NOT NULL DEFAULT 'pe-agent',
    status TEXT NOT NULL DEFAULT 'maintenance',
    message TEXT NOT NULL DEFAULT '',
    created_at TEXT NOT NULL,
    last_seen TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_pe_sessions_mac ON pe_sessions(mac);
CREATE INDEX IF NOT EXISTS idx_pe_sessions_edge ON pe_sessions(edge_id);
