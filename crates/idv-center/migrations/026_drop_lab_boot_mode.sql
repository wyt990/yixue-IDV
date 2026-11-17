-- 2026-11-17: remove the abandoned per-lab firmware boot target.
ALTER TABLE labs DROP COLUMN boot_mode;
