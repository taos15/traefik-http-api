# Rust rewrite plan and progress

## Progress (update after every verified task)

- Status: first Rust implementation locally verified against the user's two HTTP Docker socket proxies; SSH/TLS and live Traefik integration remain.
- Last completed task: verified Containers tab state filters and per-host selector in browser; added a two-letter icon fallback and explicit `traefik.icon`/Docker Desktop extension icon support.
- Next task: verify SSH and mutual-TLS connections against real remote Docker hosts, exercise an authorized container action where write permissions are available, and poll the new endpoint from a real Traefik deployment.
- Verification: `cargo test --features server` (6 passing tests), `cargo clippy --all-targets --features server -- -D warnings`, `cargo fmt --check`, wasm build and browser UI. Isolated instance on loopback port 44001 and SQLite under `/tmp/opencode/`: two server records, 59 containers (33 Unraid + 26 PromoxLxc), inspect succeeds for both, 20 running routes/services (15 + 5). The All / Non-running / Running controls show 59 / 9 / 50; the Unraid non-running filter shows 4 Start/Inspect cards, Unraid running shows 29, and PromoxLxc running shows 21. The ProxmoxLxc cards show 20 local fallback icons and the Portainer extension label icon. With `DOMAIN=taos15.net` and `DEFAULT_AUTH_MIDDLEWARE=auth@file`, all 20 shared routes have the same upstream URLs and case-insensitive host rules as the deployed TypeScript endpoint. Six legacy-only routes belong to exited/created/restarting containers, several with invalid `:undefined` upstreams; intentionally excluded by running-only policy. Docker CLI lacks the Buildx plugin; `docker build` uses the deprecated legacy builder on this machine.

## Architecture and behavior

- Rust 2024, Tokio, Axum, SQLx/SQLite, Bollard, Dioxus web; one deployment.
- Merge containers from independent Docker Engines (not Docker Swarm). Keep source server identity for every container and command.
- Support HTTP Docker socket proxies, SSH, mutual TLS, and Unix sockets. Separate each server's Docker API endpoint, Traefik-reachable upstream host, and browser-reachable WebUI host.
- Import existing Prisma SQLite `containerserver` records; back up the original database. Do not import plaintext legacy user passwords as valid credentials. Use a new authenticated admin bootstrap and sessions.
- Traefik polls GET `/api/v1/traefikconfig` on the shared listener. Anonymous by default; optional provider bearer token, with no anonymous fallback when enabled. Protect management API and WebUI independently.
- Traefik install configuration enables file/HTTP providers and entrypoints. Stable middlewares (e.g. `default-forward-auth@file`) live in Traefik's dynamic file configuration; this app generates container routers and services. `DEFAULT_AUTH_MIDDLEWARE` is optional. Without it, the app must visibly report that automatic auth is off.
- Only running, explicitly opted-in containers contribute routes. Refresh all enabled Docker servers; atomically replace the complete snapshot on success, persist last good snapshot across restarts, and retain it on transient failures. Intentional disable/deletion removes affected routes.
- Resolve browser WebUI links from Unraid `net.unraid.docker.webui` (`[IP]`, `[PORT:container-port]`), then explicit application URL, then one unambiguous published TCP port. Use browser host rather than 0.0.0.0/internal IP; allow explicit per-container host override. Traefik upstream and browser URLs may differ.
- Retain legacy labels: `traefik.enable`, `swag=enable`, `traefik.name`, `traefik.hostname`, `traefik.webuiport` (published HOST port), `traefik.service.scheme`, `traefik.service`, `traefik.http.disable`, `traefik.middlewares`, `traefik.entrypoints`, `authelia_auth=false`. New `traefik.forwardauth` defaults true: when configured, attach `DEFAULT_AUTH_MIDDLEWARE` unless false. Explicit `traefik.middlewares` overrides automatic default; new label overrides old alias.
- Use complete Traefik `loadBalancer.servers[].url` values, current `ipAllowList` where needed, and do not override entrypoint TLS defaults inadvertently. Validate collisions, invalid hostnames, ambiguous/missing ports, and middleware inputs.
- Docker proxy permissions differ by deployment. Read-only discovery must work; denied write actions should report their actual error.

## Milestones

- [x] Save durable plan with progress ledger.
- [x] Rust crate, migrations, legacy SQLite import, and server configuration CRUD (local import verified).
- [x] Docker transport implementations, multi-host inventory, container inspect/actions (local Unix transport verified; SSH/TLS/HTTP remote require deployment tests).
- [x] Traefik dynamic config generator and persisted last-good snapshot (tested with local Docker Engine, no live Traefik).
- [x] Authentication, API, optional provider token (local login and same-origin behavior tested).
- [x] Dioxus browser UI with server forms, edit, cards, controls, WebUI links, inspect and provider preview (browser login/navigation verified).
- [x] Rust Docker/Compose/CI/docs; local fmt, clippy, unit and wasm checks; Docker image builds.
- [x] Delete tracked TypeScript, Prisma, Node, React build, local `node_modules/`, hidden editor config. Existing ignored SQLite data/backups preserved.
- [x] Verify HTTP socket-proxy discovery against two actual remote hosts and compare the output with the deployed legacy API (read-only test; no lifecycle actions).
- [ ] Deploy against real remote SSH/TLS Engines and Traefik v3; verify cross-host reachability and Docker-proxy write permissions where available.
- [ ] Verify arm64 image build and deployment in CI environment with multiarch support.

## References

- Default recipes: https://rust-lang-nursery.github.io/rust-cookbook/
- Language: https://doc.rust-lang.org/book/
- Traefik HTTP provider: https://doc.traefik.io/traefik/reference/install-configuration/providers/others/http/
- Docker Engine API: https://docs.docker.com/reference/api/engine/
- Dioxus: https://dioxuslabs.com/
