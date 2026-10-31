# HTTP API

- `GET /api/ping`: liveness.
- `GET /api/v1/traefikconfig`: last complete Traefik dynamic JSON; anonymous by default or protected by `PROVIDER_TOKEN` bearer header. Returns 503 until the first snapshot.
- `POST /auth/login` (`username`, `password`): admin session cookie; `POST /auth/logout`: invalidate session.
- `GET /api/v1/servers`, `POST /api/v1/servers`, `PUT /api/v1/servers/{id}`, `DELETE /api/v1/servers/{id}`: Docker server records. Input: `name`, `transport` (`http`, `ssh`, `tls`, `unix`), `endpoint`, optional `tls_key`, `tls_cert`, `tls_ca` paths, `upstream_host`, `webui_host`, `enabled`.
- `POST /api/v1/servers/test`: test an input server without saving it.
- `GET /api/v1/containers`: combined inventory including source server and resolved WebUI link.
- `GET /api/v1/containers/{server_id}/{id}`: inspect a container on its source Engine.
- `POST /api/v1/containers/{server_id}/{id}/actions` with `{"command":"start|stop|restart|pause|unpause"}`: send command to originating Engine.
- `GET /api/v1/provider/status`: last successful snapshot timestamp and default ForwardAuth setting.
- `GET /api/v1/provider/preview`: login-protected view of the last published Traefik configuration (works even when the polling endpoint requires a provider token).

Except for ping, login, and the Traefik provider GET, all endpoints require the admin session cookie. Container operations can be rejected by Docker socket proxies with restricted write permissions.
