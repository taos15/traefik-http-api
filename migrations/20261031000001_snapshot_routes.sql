CREATE TABLE IF NOT EXISTS provider_routes (
    router_name TEXT PRIMARY KEY NOT NULL,
    service_name TEXT NOT NULL,
    server_id TEXT NOT NULL
);
