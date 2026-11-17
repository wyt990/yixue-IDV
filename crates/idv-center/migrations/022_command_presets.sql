-- 2026-11-17: reusable remote command presets (command / url / program).
CREATE TABLE IF NOT EXISTS command_presets (
    id TEXT PRIMARY KEY NOT NULL,
    kind TEXT NOT NULL,
    name TEXT NOT NULL,
    command TEXT NOT NULL DEFAULT '',
    url TEXT NOT NULL DEFAULT '',
    program_path TEXT NOT NULL DEFAULT '',
    args TEXT NOT NULL DEFAULT '',
    created_by TEXT NOT NULL DEFAULT '',
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_command_presets_kind ON command_presets(kind, name);
