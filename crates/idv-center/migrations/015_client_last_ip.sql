-- Client last known IPv4 from agent heartbeat
ALTER TABLE clients ADD COLUMN last_ip TEXT NOT NULL DEFAULT '';
