CREATE TABLE IF NOT EXISTS edges (
    id TEXT PRIMARY KEY NOT NULL,
    name TEXT NOT NULL UNIQUE,
    node_token TEXT NOT NULL UNIQUE,
    listen_url TEXT NOT NULL DEFAULT '',
    last_seen TEXT,
    created_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS image_replicas (
    id TEXT PRIMARY KEY NOT NULL,
    edge_id TEXT NOT NULL REFERENCES edges(id) ON DELETE CASCADE,
    image_id TEXT NOT NULL REFERENCES images(id) ON DELETE CASCADE,
    state TEXT NOT NULL,
    progress REAL NOT NULL DEFAULT 0,
    local_path TEXT NOT NULL DEFAULT '',
    checksum_sha256 TEXT NOT NULL DEFAULT '',
    error_message TEXT NOT NULL DEFAULT '',
    updated_at TEXT NOT NULL,
    UNIQUE (edge_id, image_id)
);

CREATE TABLE IF NOT EXISTS edge_commands (
    id TEXT PRIMARY KEY NOT NULL,
    edge_id TEXT NOT NULL REFERENCES edges(id) ON DELETE CASCADE,
    kind TEXT NOT NULL,
    image_id TEXT,
    state TEXT NOT NULL DEFAULT 'pending',
    progress REAL NOT NULL DEFAULT 0,
    error_message TEXT NOT NULL DEFAULT '',
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_replicas_edge ON image_replicas(edge_id);
CREATE INDEX IF NOT EXISTS idx_commands_edge_state ON edge_commands(edge_id, state);
