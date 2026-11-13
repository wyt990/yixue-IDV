-- Campus / lab default baseline image + preferred Edge bindings (lab → campus fallback).

ALTER TABLE campuses ADD COLUMN default_image_id TEXT REFERENCES images(id) ON DELETE SET NULL;
ALTER TABLE labs ADD COLUMN default_image_id TEXT REFERENCES images(id) ON DELETE SET NULL;

CREATE TABLE IF NOT EXISTS campus_edges (
    campus_id TEXT NOT NULL REFERENCES campuses(id) ON DELETE CASCADE,
    edge_id TEXT NOT NULL REFERENCES edges(id) ON DELETE CASCADE,
    priority INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (campus_id, edge_id)
);

CREATE TABLE IF NOT EXISTS lab_edges (
    lab_id TEXT NOT NULL REFERENCES labs(id) ON DELETE CASCADE,
    edge_id TEXT NOT NULL REFERENCES edges(id) ON DELETE CASCADE,
    priority INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (lab_id, edge_id)
);

CREATE INDEX IF NOT EXISTS idx_campus_edges_priority ON campus_edges(campus_id, priority);
CREATE INDEX IF NOT EXISTS idx_lab_edges_priority ON lab_edges(lab_id, priority);
