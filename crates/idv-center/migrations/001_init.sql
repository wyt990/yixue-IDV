CREATE TABLE IF NOT EXISTS admins (
    id TEXT PRIMARY KEY NOT NULL,
    username TEXT NOT NULL UNIQUE,
    password_hash TEXT NOT NULL,
    display_name TEXT NOT NULL,
    created_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS sessions (
    token TEXT PRIMARY KEY NOT NULL,
    admin_id TEXT NOT NULL REFERENCES admins(id) ON DELETE CASCADE,
    expires_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS campuses (
    id TEXT PRIMARY KEY NOT NULL,
    name TEXT NOT NULL UNIQUE,
    created_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS labs (
    id TEXT PRIMARY KEY NOT NULL,
    campus_id TEXT NOT NULL REFERENCES campuses(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    created_at TEXT NOT NULL,
    UNIQUE (campus_id, name)
);

CREATE TABLE IF NOT EXISTS clients (
    id TEXT PRIMARY KEY NOT NULL,
    lab_id TEXT NOT NULL REFERENCES labs(id) ON DELETE CASCADE,
    mac TEXT NOT NULL UNIQUE,
    name TEXT NOT NULL DEFAULT '',
    notes TEXT NOT NULL DEFAULT '',
    last_seen TEXT,
    created_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS images (
    id TEXT PRIMARY KEY NOT NULL,
    name TEXT NOT NULL,
    os_family TEXT NOT NULL,
    version_label TEXT NOT NULL DEFAULT '',
    checksum_sha256 TEXT NOT NULL,
    size_bytes INTEGER NOT NULL DEFAULT 0,
    storage_uri TEXT NOT NULL DEFAULT '',
    boot_mode TEXT NOT NULL DEFAULT 'uefi',
    notes TEXT NOT NULL DEFAULT '',
    created_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_labs_campus ON labs(campus_id);
CREATE INDEX IF NOT EXISTS idx_clients_lab ON clients(lab_id);
CREATE INDEX IF NOT EXISTS idx_sessions_admin ON sessions(admin_id);
