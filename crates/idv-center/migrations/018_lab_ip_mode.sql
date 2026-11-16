-- 2026-11-16: per-lab IP assignment mode for batch identity provisioning.
-- dhcp   = Edge DHCP reservation by MAC (recommended for batch deployment)
-- static = agent writes static IP (legacy behavior, default for existing labs)
ALTER TABLE labs ADD COLUMN ip_mode TEXT NOT NULL DEFAULT 'static';
