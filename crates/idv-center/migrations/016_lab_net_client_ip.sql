-- Lab static-network defaults + per-client assigned IP
ALTER TABLE labs ADD COLUMN netmask TEXT NOT NULL DEFAULT '';
ALTER TABLE labs ADD COLUMN gateway TEXT NOT NULL DEFAULT '';
ALTER TABLE labs ADD COLUMN dns TEXT NOT NULL DEFAULT '';
ALTER TABLE clients ADD COLUMN assigned_ip TEXT NOT NULL DEFAULT '';
