CREATE TABLE IF NOT EXISTS container_webui_urls (
    server_id TEXT NOT NULL,
    container_id TEXT NOT NULL,
    url TEXT,
    PRIMARY KEY (server_id, container_id)
);

CREATE TABLE IF NOT EXISTS runtime_routers (
    name TEXT PRIMARY KEY NOT NULL,
    status TEXT NOT NULL,
    rule TEXT NOT NULL,
    fetched_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS runtime_snapshot (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    fetched_at INTEGER NOT NULL,
    api_reachable INTEGER NOT NULL
);
