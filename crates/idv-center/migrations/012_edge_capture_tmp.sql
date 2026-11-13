-- Per-edge PE capture temp directory (Center-managed; Edge applies via heartbeat).
-- Empty = Edge uses local edge.toml / cache_dir/capture-tmp default.
ALTER TABLE edges ADD COLUMN capture_tmp_dir TEXT NOT NULL DEFAULT '';
