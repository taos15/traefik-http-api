# Development guide

User-facing deployment, server settings, and container labels are documented in [README.md](README.md). This guide covers native development, implementation contracts, migrations, and publishing. The [HTTP API reference](Endpoints.MD) describes management endpoints.

## Project layout

| Path | Responsibility |
| --- | --- |
| `src/main.rs` | Axum server, management routes, SQLite persistence, and provider refresh loop. |
| `src/lib.rs` | Shared API models used by server and frontend. |
| `src/docker.rs` | Docker connections, host/network discovery, and container inventory through Bollard. |
| `src/provider.rs` | Container label interpretation, service targets, browser links, and generated Traefik configuration. |
| `src/auth.rs` | Automatic ForwardAuth provider selection and templates. |
| `src/traefik.rs` | Internal Traefik API integration, effective file snapshots, redaction, and exports. |
| `src/web.rs` | Dioxus WebAssembly frontend. |
| `migrations/` | SQLx SQLite migrations. |
| `templates/` | Embedded OAuth2 Proxy, Authentik, and Authelia definitions. |
| `web/dist/` | Static HTML/CSS and generated frontend assets. |

## Native setup

Requires Rust **1.99+**, the `wasm32-unknown-unknown` target, and **wasm-bindgen-cli 0.2.130**, matching the locked frontend dependency.

```sh
rustup target add wasm32-unknown-unknown
cargo install wasm-bindgen-cli --version 0.2.130 --locked
mkdir -p data
```

Build the frontend:

```sh
cargo build --release --no-default-features --features web --target wasm32-unknown-unknown
wasm-bindgen --target web --out-dir web/dist --out-name app target/wasm32-unknown-unknown/release/traefik-web.wasm
```

Start the backend from the repository root:

```sh
export DOMAIN=example.com
export DATABASE_URL=sqlite://data/traefik.db
export ADMIN_BOOTSTRAP_PASSWORD=replace-with-a-unique-long-password
cargo run --no-default-features --features server --bin traefik-server
```

Open `http://localhost:4000` and sign in as `admin`. The app does not load `.env` automatically when launched through Cargo; export the settings yourself. Container deployments use Compose's environment handling.

Rebuild `web/dist/app.js`, `app_bg.wasm`, and `snippets/` after UI changes. These generated files are ignored by Git; editing Rust source without rebuilding them leaves the browser running the previous UI.

The Dockerfile builds both binaries and frontend assets. For a containerized source deployment, follow the README. `compose.dev.yml` is a minimal build-based deployment, not a hot-reload development server; it does not provide the shared Traefik network or Docker credentials automatically.

## Verification

Run the project checks after reconciling backend and frontend changes:

```sh
cargo fmt --check
cargo clippy --all-targets --features server -- -D warnings
cargo test --features server
cargo check --no-default-features --features web --target wasm32-unknown-unknown
```

For routing changes, also verify a real Docker port mapping: a container with `8080:80` and `traefik.webuiport=8080` must produce `container-name:80` on the selected network and `server-host:8080` otherwise. Browser links must stay on the external port. Exercise add/edit checkboxes and reload/restart persistence after frontend changes.

## Server settings and routing contracts

- `upstream_host` is the concrete server address used for host/published-port routing. Discovery suggests a value; saving persists the user's choice. Configuration generation does not rediscover it.
- `prefer_network` and optional `docker_network` select Docker DNS routing for containers attached to that network. Network discovery lists user-defined, local-scope bridge networks, excluding built-in networks and configuration-only networks.
- `webui_same_as_server` selects dynamic browser-host inheritance through `DockerServer::browser_host()`. `webui_host` remains the explicit host when inheritance is off; the container's `traefik.webui.host` label takes precedence.
- All preference flags default off for existing database records and legacy API payloads. The additive migration must not change existing host strings.
- `/api/v1/servers/discover` and `/api/v1/servers/test` validate connection settings without requiring a name or routing hosts. Saving a server performs full validation. Discovery and test still require an authenticated session and a matching Origin header.
- Discovery uses endpoint hostnames and available Docker info as suggestions. It does not prove browser reachability or require automatic local-host detection. Do not replace user-entered addresses with discovery results.
- `traefik.webuiport` remains an **external published-port selector**. Resolve its Docker mapping to obtain the internal port for container DNS. Do not introduce an internal-port label or silently change the existing label's meaning.
- Target selection falls back to host/published-port routing if the preferred network target cannot be resolved. This is not runtime health-based failover. Generation errors retain the last successful snapshot; server edits/deletions can separately prune that server's previous routes.

## Legacy database migration

For an existing Prisma SQLite database:

1. Stop the old service and back up the database before starting the Rust version.
2. Use a copy of that backup as the new `DATABASE_URL` file. Never overwrite the original database with an empty file.
3. Startup migrations create the new tables alongside the old `containerserver` table.
4. Startup imports server IDs, names, HTTP endpoints, and their former `enable` state with `INSERT OR IGNORE`.
5. Inspect both imported host fields: the old Docker HTTP API address may not be reachable from Traefik or from a browser.
6. Set a new bootstrap password. The legacy plaintext `User` table is ignored; old accounts are not imported.
7. Keep the original backup until server connections, browser links, and generated routes have been verified.

## Traefik integration details

### Internal API access

The app tries `http://<traefik-container-name>:8080/api/rawdata`. Override its base URL with `TRAEFIK_API_URL` when the API is secured or uses another internal address. Basic auth uses `TRAEFIK_API_USERNAME` and `TRAEFIK_API_PASSWORD`, or `TRAEFIK_API_PASSWORD_FILE`. Passwords are not stored in SQLite.

With `api.insecure=false`, expose `api@internal` on a dedicated internal entrypoint/router independently of the HTTP provider. For example, enable a static `traefik-api` entrypoint on `:8080`, set `TRAEFIK_API_URL=http://traefik:8080`, and load this dynamic file:

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

Keep this entrypoint reachable **only on the Docker network**. Replace the example with a real `htpasswd` hash. In Compose labels/environment, escape `$` as `$$`. Set `api.insecure=false` explicitly so Traefik does not expose its unprotected default API. The app needs the matching plaintext password in its environment or secret mount; browser ForwardAuth cannot authenticate this request.

### Default authentication

Define **one** `default-forward-auth` middleware in a Traefik dynamic file to override automatic detection:

```yaml
http:
  middlewares:
    default-forward-auth:
      forwardAuth:
        address: https://auth.example.com/check
```

Enable Traefik's file provider for that directory, for example `providers.file.directory: /conf.d` and `providers.file.watch: true`. The file definition wins in full; it is not merged with generated auth resources. Other manually defined ForwardAuth middlewares remain independent.

Without a file override, automatic selection prefers OAuth2 Proxy, then Authentik, then Authelia on Traefik's daemon and shared custom network. The embedded definitions are in `templates/`. OAuth2 Proxy also receives an internal sign-in service even if its container has `traefik.enable=false`. Authentik/OAuth2 Proxy require the appropriate outpost/sign-in **path routers** in Traefik's file configuration.

Explicit `traefik.middlewares` replaces the default list; `traefik.forwardauth=false` opts out, with `authelia_auth=false` retained as an alias. Previously protected routes must not be silently downgraded when auth becomes unavailable. The Provider tab exposes selection/freshness and offers a reset operation for the sticky auth selection.

### Snapshots and exports

The Provider tab's file snapshots reconstruct effective `@file` resources from `/api/rawdata`, not original source files. Secrets and password hashes are redacted **before SQLite storage** and must be re-entered in an export.

Exports cover effective HTTP routers, services, and middlewares only. They do not reproduce TCP, UDP, TLS certificate material, static options, comments, templates, or duplicate source-file definitions. Unknown fields are omitted and mark the snapshot unverified. Even a verified snapshot is only a recovery aid: review it before placing it in an empty dynamic-file directory. Loading it alongside original definitions can create duplicate names.

Auth containers on a different Docker host/network can use **Generate @file snippet** when they have a published port reachable by Traefik. Paste the reviewed snippet into Traefik's dynamic-file directory to override automatic detection; heed same-name resource warnings.

## Image publishing and releases

**MultiArch Master CI** (`.github/workflows/multiarch-master.yml`) publishes `ghcr.io/taos15/traefik-http-api` for `linux/amd64` and `linux/arm64`.

| Trigger | Public tags | GitHub Release |
| --- | --- | --- |
| Push/merge to `master` | `edge`, `sha-<full-commit>` | None |
| Push `v1.2.3` | `1.2.3`, `latest`, `sha-<full-commit>` | Generated release notes |
| Push `v1.3.0-rc.1` | `1.3.0-rc.1`, `sha-<full-commit>` | Prerelease |
| Pull request | Checks only, through Rust CI | None |

To release a commit already merged into `master`:

```sh
git switch master
git pull --ff-only
git tag v1.2.3
git push origin v1.2.3
```

Tags must follow `vMAJOR.MINOR.PATCH` with an optional SemVer prerelease suffix. Build metadata (`+...`) is not accepted in Docker tags. A local tag does not publish until pushed. Use normal Git credentials: tags created with a workflow's `GITHUB_TOKEN` do not trigger another workflow. GitHub Releases are created after image publication; creating a release in the UI is not the build trigger.

The workflow runs Rust checks, builds both architectures on native runners through Docker's pinned reusable builder, and pushes a `build-<run-id>-<attempt>` candidate. Both architectures must pass startup, database, `/api/ping`, and frontend-asset smoke checks before the tested digest is promoted. Failures leave public channel/version tags unchanged. Candidates remain available for diagnostics, and the run summary records the digest.

Publish stable releases in version order: rerunning an older stable tag can promote it to `latest`. Prereleases do not update `latest`. Commit tags identify source, but reruns can rebuild dependencies or base images; only digest pinning identifies an immutable image.

Manual runs support `master` or an existing valid version tag; other branches are rejected. Stale master builds can publish their commit tag but do not move `edge` backwards. Release creation is idempotent and preserves edited release notes.

GHCR login uses the repository's `GITHUB_TOKEN` with `packages: write`. In the package's **Manage Actions access**, grant this repository write access. The legacy `DOCKER_TOKEN` secret is not used. Native ARM64 execution and package permissions require verification in the actual GitHub environment, not just local checks.
