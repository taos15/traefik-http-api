CREATE TABLE IF NOT EXISTS file_snapshots (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    config TEXT NOT NULL,
    redacted_paths TEXT NOT NULL,
    verified INTEGER NOT NULL,
    created_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS auth_selection (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    provider TEXT,
    container_id TEXT,
    server_id TEXT,
    network TEXT,
    dns_name TEXT,
    protected INTEGER NOT NULL DEFAULT 0
);
