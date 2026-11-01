CREATE TABLE IF NOT EXISTS migration_state (
    name TEXT PRIMARY KEY NOT NULL,
    completed_at INTEGER NOT NULL
);
