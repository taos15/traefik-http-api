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
    filename: /etc/traefik/dynamic.yml
    watch: true
  http:
    endpoint: http://api:4000/api/v1/traefikconfig
```

Traefik polls that GET endpoint. It is anonymous by default on the same listener as the WebUI; restrict access with networking/firewall. To require a bearer token instead, set `PROVIDER_TOKEN` on the app and configure `providers.http.headers.Authorization: 'Bearer <token>'` in Traefik. Management routes and UI always require login.

To use authentication for generated routers, define a middleware once in Traefik's *dynamic file configuration* and set `DEFAULT_AUTH_MIDDLEWARE=default-forward-auth@file` on the app:

```yaml
http:
  middlewares:
    default-forward-auth:
      forwardAuth:
        address: https://auth.example.com/check
```

Without `DEFAULT_AUTH_MIDDLEWARE` automatic authentication is **off**. `traefik.forwardauth=false` opts a container out of the default when configured; `traefik.middlewares` supplies an explicit replacement list. The old `authelia_auth=false` remains an alias. Keep TLS defaults on Traefik entrypoints or configure them separately; the app does not replace Traefik's install configuration.

## Docker servers and labels

Add servers in the UI with `http://host:2375` (Docker socket proxy), `ssh://user@host`, a TLS endpoint with client key/cert/CA paths, or a Unix socket path. Configure a **Traefik-reachable host** and a **browser-reachable host** independently of the Engine API address. Docker publishes container port 80 as host port 8080 with `-p 8080:80`; the generated service URL uses `<traefik-host>:8080`, and the WebUI link uses `<browser-host>:8080`. Loopback-only ports do not provide remote reachability. Docker proxy permissions determine which read and write operations work.

The generator recognizes `traefik.enable=true` or `swag=enable`, `traefik.name`, `traefik.hostname`, `traefik.webuiport` (explicit published **host** port), `traefik.service.scheme`, `traefik.service`, `traefik.http.disable=true`, `traefik.middlewares`, `traefik.entrypoints`, `traefik.forwardauth=false`, and legacy `authelia_auth=false`. Only running containers get generated routers. `net.unraid.docker.webui` with `[IP]` and `[PORT:container-port]` and `traefik.webui.url` help construct browser links; `traefik.webui.host` overrides the browser host. `net.unraid.docker.icon` supplies an HTTPS icon.

## Verify

```sh
cargo fmt --check
cargo clippy --all-targets --features server -- -D warnings
cargo test --features server
cargo check --no-default-features --features web --target wasm32-unknown-unknown
```
