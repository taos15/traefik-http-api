# Traefik HTTP API — Rust

Aggregate independent Docker Engines and provide a single Traefik HTTP-provider dynamic configuration and Dioxus WebUI. The [rewrite plan and progress ledger](RUST_REWRITE_PLAN.md) records decisions and remaining work.

## Run locally

Requires Rust 1.99+, the `wasm32-unknown-unknown` target, and `wasm-bindgen-cli` 0.2.130.

```sh
rustup target add wasm32-unknown-unknown
cargo install wasm-bindgen-cli --version 0.2.130 --locked
cargo build --release --no-default-features --features web --target wasm32-unknown-unknown
wasm-bindgen --target web --out-dir web/dist --out-name app target/wasm32-unknown-unknown/release/traefik-web.wasm
cargo run --no-default-features --features server --bin traefik-server
```

Create `data/`, set `DOMAIN`, `DATABASE_URL=sqlite://data/traefik.db`, and set a long `ADMIN_BOOTSTRAP_PASSWORD` before first run. Login as `admin`. The generated `web/dist/app.js`, `.wasm`, and `snippets/` assets must be rebuilt after UI changes. Container deployment: `docker compose up --build` with those settings provided in `.env` (see `.env.example`). Mount your Docker socket, SSH credentials, or TLS certificate files as appropriate for each connection.

**Existing Prisma SQLite database:** back it up while the old service is stopped, then use the backup as the new `DATABASE_URL` file. Migrations create new tables alongside `containerserver`, and startup imports server IDs/names/HTTP endpoints with their former `enable` state using INSERT OR IGNORE. Inspect the imported Traefik and browser hosts; the old HTTP API host might not be reachable from either. The legacy plaintext `User` table is ignored; a new admin password is required. Keep the original backup until verified. Never overwrite the original database with an empty file.

## Traefik setup

In Traefik install configuration, enable the HTTP provider:

```yaml
providers:
  file:
    directory: /conf.d
    watch: true
  http:
    endpoint: http://traefik-http-api:4000/api/v1/traefikconfig
```

Traefik polls that GET endpoint. It is anonymous by default on the same listener as the WebUI; restrict access with networking/firewall. To require a bearer token instead, set `PROVIDER_TOKEN` on the app and configure `providers.http.headers.Authorization: 'Bearer <token>'` in Traefik. Management routes and UI always require login.

The app and Traefik **must run on the same Docker daemon and custom Docker network**, with Docker DNS resolution of container names. For automatic auth, the auth container must also run on that daemon/network. The app tries Traefik's `http://<container-name>:8080/api/rawdata`; when the API is secured or exposed on another internal URL, use `TRAEFIK_API_URL` and optionally `TRAEFIK_API_USERNAME` with `TRAEFIK_API_PASSWORD` or `TRAEFIK_API_PASSWORD_FILE`. When `api.insecure=false`, expose `api@internal` on a dedicated internal entrypoint/router with Basic auth independently of the HTTP provider. Passwords are never stored in SQLite.

Example internal API router for `api.insecure=false` (enable a static `traefik-api` entrypoint on `:8080` in Traefik and set `TRAEFIK_API_URL=http://traefik:8080`). Keep the entrypoint reachable **only on the Docker network** and replace the example hash with a real `htpasswd` hash; in Compose, escape `$` as `$$`:

```yaml
http:
  routers:
    internal-traefik-api:
      entryPoints: [traefik-api]
      rule: PathPrefix(`/api/`)
      service: api@internal
      middlewares: [internal-api-basic]
  middlewares:
    internal-api-basic:
      basicAuth:
        users: ["internal:$apr1$REPLACE_WITH_REAL_HASH"]
```

The app needs the matching plaintext user/password through its own environment or mounted secret; browser-only ForwardAuth login cannot authenticate its internal API request. Static `api.insecure=false` must also be set so Traefik does not expose an unprotected API on its default entrypoint.

To override automatic authentication, define **one** `default-forward-auth` middleware in a Traefik dynamic file. The file definition wins in full; the app does not merge its resources with that file. Other manually defined ForwardAuth middlewares remain independent:

```yaml
http:
  middlewares:
    default-forward-auth:
      forwardAuth:
        address: https://auth.example.com/check
```

When no file definition exists, the app finds auth containers on Traefik's custom network and generates `default-forward-auth@http` from one of its embedded templates: OAuth2 Proxy first, then Authentik, then Authelia. The three example definitions are in `templates/`. OAuth2 Proxy also gets an internal sign-in service even when its container has `traefik.enable=false`. Authentik and OAuth2 Proxy still need their respective outpost/sign-in **path routers** in your Traefik file. When no auth provider was ever configured, routes work without default auth; the UI makes this visible. When a previously protected default is confirmed unavailable, affected routes are withheld instead of published unprotected. `traefik.forwardauth=false` opts a container out; `traefik.middlewares` supplies an explicit replacement list. The old `authelia_auth=false` remains an alias. Keep TLS defaults on Traefik entrypoints or configure them separately.

The Provider tab shows current auth selection, API freshness, file snapshots and redacted **Generate config** downloads. The snapshots are best-effort reconstructions from `/api/rawdata`, not full backups of source files. Secrets and password hashes are replaced before SQLite storage and must be re-entered in an export. Auth containers on another Docker host/network can use **Generate @file snippet** when their port is published. Traefik must be able to reach the generated address; paste the snippet into your dynamic file directory to override automatic detection. The UI warns about existing same-name file resources.

Exports currently cover effective HTTP routers, services and middlewares only; TCP, UDP, TLS certificate material, static options, comments, templates and duplicate file definitions are not reproduced. Unknown resource fields are left out and mark the snapshot unverified. Even a verified snapshot is only a recovery aid: review it and re-enter redacted values before placing it in an empty Traefik file directory. Placing it alongside the original files can create duplicate names.

## Docker servers and labels

Add servers in the UI with `http://host:2375` (Docker socket proxy), `ssh://user@host`, a TLS endpoint with client key/cert/CA paths, or a Unix socket path. Configure a **Traefik-reachable host** and a **browser-reachable host** independently of the Engine API address. Docker publishes container port 80 as host port 8080 with `-p 8080:80`; the generated service URL uses `<traefik-host>:8080`, and the WebUI link uses `<browser-host>:8080`. Loopback-only ports do not provide remote reachability. Docker proxy permissions determine which read and write operations work.

The generator recognizes `traefik.enable=true` or `swag=enable`, `traefik.name`, `traefik.hostname`, `traefik.webuiport` (explicit published **host** port), `traefik.service.scheme`, `traefik.service`, `traefik.http.disable=true`, `traefik.middlewares`, `traefik.entrypoints`, `traefik.forwardauth=false`, and legacy `authelia_auth=false`. Only running containers get generated routers. `net.unraid.docker.webui` with `[IP]` and `[PORT:container-port]` and `traefik.webui.url` help construct browser links; `traefik.webui.host` overrides the browser host. `net.unraid.docker.icon` supplies an HTTPS icon.

Without a WebUI label, a card offers a link only for one unambiguous published TCP port; common non-HTTP service ports (including database ports and the Docker API) are excluded from this fallback. Add `net.unraid.docker.webui` or `traefik.webui.url` to identify an unusual application port explicitly. Docker may report a published port twice for IPv4/IPv6; the UI displays a single mapping.

The **Containers** tab includes running and non-running containers from every enabled server. Use **All**, **Non-running**, or **Running**, the Docker host selector, and name/image search to find a container; stopped or created cards have a **Start** action. Only running containers are included in Traefik's generated routes.

Docker does not provide a general icon field for ordinary containers. Cards try `traefik.icon` first, then Unraid's `net.unraid.docker.icon`, then `com.docker.desktop.extension.icon` when present. These must be HTTPS image URLs. Otherwise cards show a local two-letter fallback. Add `traefik.icon=https://example.com/icon.png` to a container to supply its icon explicitly (recreate the container for a changed Docker label to take effect).

## Verify

```sh
cargo fmt --check
cargo clippy --all-targets --features server -- -D warnings
cargo test --features server
cargo check --no-default-features --features web --target wasm32-unknown-unknown
```
