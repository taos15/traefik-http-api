ALTER TABLE docker_servers ADD COLUMN prefer_network INTEGER NOT NULL DEFAULT 0 CHECK (prefer_network IN (0,1));
ALTER TABLE docker_servers ADD COLUMN docker_network TEXT;
ALTER TABLE docker_servers ADD COLUMN webui_same_as_server INTEGER NOT NULL DEFAULT 0 CHECK (webui_same_as_server IN (0,1));
