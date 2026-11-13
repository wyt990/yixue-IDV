ALTER TABLE images ADD COLUMN partition_map TEXT NOT NULL DEFAULT '[]';

CREATE TABLE IF NOT EXISTS maintenance_tasks (
    id TEXT PRIMARY KEY NOT NULL,
    kind TEXT NOT NULL,
    state TEXT NOT NULL,
    image_id TEXT,
    edge_id TEXT,
    disk_plan TEXT NOT NULL DEFAULT 'wipe_whole_disk',
    name TEXT NOT NULL DEFAULT '',
    error_message TEXT NOT NULL DEFAULT '',
    created_by TEXT NOT NULL DEFAULT '',
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS maintenance_task_targets (
    id TEXT PRIMARY KEY NOT NULL,
    task_id TEXT NOT NULL REFERENCES maintenance_tasks(id) ON DELETE CASCADE,
    mac TEXT NOT NULL,
    state TEXT NOT NULL,
    progress REAL NOT NULL DEFAULT 0,
    error_message TEXT NOT NULL DEFAULT '',
    detail TEXT NOT NULL DEFAULT '',
    updated_at TEXT NOT NULL,
    UNIQUE (task_id, mac)
);

CREATE INDEX IF NOT EXISTS idx_task_targets_mac_state ON maintenance_task_targets(mac, state);
CREATE INDEX IF NOT EXISTS idx_tasks_state ON maintenance_tasks(state);
