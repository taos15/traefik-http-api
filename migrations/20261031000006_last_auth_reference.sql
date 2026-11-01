ALTER TABLE auth_selection ADD COLUMN last_reference TEXT;
ALTER TABLE file_snapshots ADD COLUMN resource_errors TEXT NOT NULL DEFAULT '[]';
