-- 2026-11-17: lab default firmware boot target (disk / pxe).
ALTER TABLE labs ADD COLUMN boot_mode TEXT NOT NULL DEFAULT 'disk';
