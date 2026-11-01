CREATE TABLE IF NOT EXISTS docker_servers (
    id TEXT PRIMARY KEY NOT NULL,
    name TEXT NOT NULL UNIQUE,
    transport TEXT NOT NULL CHECK (transport IN ('http','tls','ssh','unix')),
    endpoint TEXT NOT NULL,
    tls_key TEXT,
    tls_cert TEXT,
    tls_ca TEXT,
    upstream_host TEXT NOT NULL,
    webui_host TEXT NOT NULL,
    enabled INTEGER NOT NULL DEFAULT 1 CHECK (enabled IN (0,1))
);
CREATE TABLE IF NOT EXISTS users (
    id TEXT PRIMARY KEY NOT NULL,
    username TEXT NOT NULL UNIQUE,
    password_hash TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS sessions (
    token_hash TEXT PRIMARY KEY NOT NULL,
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    expires_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS provider_snapshot (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    config TEXT NOT NULL,
    refreshed_at INTEGER NOT NULL
);
