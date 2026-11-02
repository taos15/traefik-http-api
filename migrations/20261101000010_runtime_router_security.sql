ALTER TABLE runtime_routers ADD COLUMN forward_auth INTEGER CHECK (forward_auth IN (0, 1));
ALTER TABLE runtime_routers ADD COLUMN tls INTEGER CHECK (tls IN (0, 1));
