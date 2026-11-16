-- 2026-11-16: real remote assist (screen frames, input, clipboard, file transfer)
ALTER TABLE assist_sessions ADD COLUMN clipboard TEXT NOT NULL DEFAULT '';
ALTER TABLE assist_sessions ADD COLUMN last_frame_at TEXT;
ALTER TABLE assist_sessions ADD COLUMN frame_count INTEGER NOT NULL DEFAULT 0;

CREATE TABLE IF NOT EXISTS assist_commands (
    id TEXT PRIMARY KEY NOT NULL,
    session_id TEXT NOT NULL,
    kind TEXT NOT NULL,
    payload TEXT NOT NULL DEFAULT '',
    state TEXT NOT NULL DEFAULT 'pending',
    created_at TEXT NOT NULL,
    delivered_at TEXT,
    finished_at TEXT,
    error_message TEXT NOT NULL DEFAULT ''
);
CREATE INDEX IF NOT EXISTS idx_assist_commands_session_state ON assist_commands(session_id, state);

CREATE TABLE IF NOT EXISTS assist_file_requests (
    id TEXT PRIMARY KEY NOT NULL,
    session_id TEXT NOT NULL,
    mac TEXT NOT NULL,
    kind TEXT NOT NULL,
    path TEXT NOT NULL,
    state TEXT NOT NULL DEFAULT 'pending',
    file_name TEXT NOT NULL DEFAULT '',
    size_bytes INTEGER NOT NULL DEFAULT 0,
    payload TEXT NOT NULL DEFAULT '',
    error_message TEXT NOT NULL DEFAULT '',
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_assist_file_requests_session ON assist_file_requests(session_id, created_at);

-- Batch file collect/distribute reuses maintenance_tasks; keep params in payload.
ALTER TABLE maintenance_tasks ADD COLUMN payload TEXT NOT NULL DEFAULT '';
