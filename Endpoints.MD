# HTTP API

- `GET /api/ping`: liveness.
- `GET /api/v1/traefikconfig`: last complete Traefik dynamic JSON; anonymous by default or protected by `PROVIDER_TOKEN` bearer header. Returns 503 until the first snapshot.
- `POST /auth/login` (`username`, `password`): admin session cookie; `POST /auth/logout`: invalidate session.
- `GET /api/v1/servers`, `POST /api/v1/servers`, `PUT /api/v1/servers/{id}`, `DELETE /api/v1/servers/{id}`: Docker server records. Input: `name`, `transport` (`http`, `ssh`, `tls`, `unix`), `endpoint`, optional `tls_key`, `tls_cert`, `tls_ca` paths, `upstream_host`, `webui_host`, `enabled`, `prefer_network` (default `false`), `docker_network` (optional; required when network preference is on), and `webui_same_as_server` (default `false`). Browser-host inheritance uses `upstream_host` when enabled; otherwise `webui_host` is required.
- `POST /api/v1/servers/test`: test an input server without saving it. Accepts the server-input shape but only validates connection settings; name and routing hosts may be blank.
- `POST /api/v1/servers/discover`: discover using unsaved server-input connection settings, with name/routing hosts allowed to be blank. Returns `{"suggested_host":"host-or-address","networks":["custom-network"]}`; `suggested_host` may be `null`. Networks are eligible user-defined local bridges. Requires login and a matching Origin header.
- `GET /api/v1/containers`: combined inventory including source server and resolved WebUI link.
- `GET /api/v1/containers/{server_id}/{id}`: inspect a container on its source Engine.
- `POST /api/v1/containers/{server_id}/{id}/actions` with `{"command":"start|stop|restart|pause|unpause"}`: send command to originating Engine.
- `GET /api/v1/provider/status`: last successful snapshot, Traefik API error, auth selection and detected auth candidates (no credentials returned).
- `GET /api/v1/provider/preview`: login-protected view of the last published Traefik configuration (works even when the polling endpoint requires a provider token).
- `GET /api/v1/provider/files`: history of redacted effective `@file` snapshots (never HTTP-provider output).
- `GET /api/v1/provider/files/export[?id=N]`: download best-effort redacted YAML; defaults to the newest verified snapshot.
- `GET /api/v1/provider/auth/snippet?server_id=…&container_id=…`: standalone `@file` snippet for an auth container outside Traefik's Docker network when it has a reachable published port.
- `POST /api/v1/provider/auth/reset`: reset the sticky automatic-auth selection (requires session and same-origin POST).

Except for ping, login, and the Traefik provider GET, all endpoints require the admin session cookie. Container operations can be rejected by Docker socket proxies with restricted write permissions.
