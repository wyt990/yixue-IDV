-- 2026-11-17: Edge command payload (WOL relay needs target MACs/broadcast).
ALTER TABLE edge_commands ADD COLUMN payload TEXT NOT NULL DEFAULT '{}';
