-- 2026-11-17: per-target Edge assignment for multi-Edge deploy scheduling.
ALTER TABLE maintenance_task_targets ADD COLUMN edge_id TEXT;
CREATE INDEX IF NOT EXISTS idx_task_targets_edge_state
    ON maintenance_task_targets(edge_id, state);
