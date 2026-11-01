ALTER TABLE provider_routes ADD COLUMN container_id TEXT;
ALTER TABLE provider_routes ADD COLUMN rule TEXT;
ALTER TABLE provider_routes ADD COLUMN upstream_fingerprint TEXT;
ALTER TABLE runtime_routers ADD COLUMN service TEXT;

CREATE TABLE IF NOT EXISTS runtime_services (
    name TEXT PRIMARY KEY NOT NULL,
    fingerprint TEXT NOT NULL,
    fetched_at INTEGER NOT NULL
);
