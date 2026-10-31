# Rust rewrite plan and progress

## Progress (update after every verified task)

- Status: first Rust implementation built and locally verified; deployment-specific remote transport and Traefik integration tests remain.
- Last completed task: replaced tracked Node/TypeScript files, local `node_modules/`, hidden editor configuration, Docker/Compose/CI/docs with Rust equivalents; built Docker image and exercised the Dioxus UI in a browser.
- Next task: verify SSH and mutual-TLS connections against real remote Docker hosts, test an HTTP socket proxy with its actual permissions, and poll the endpoint from a real Traefik deployment.
- Verification: `cargo fmt --check`, `cargo test --features server` (3 passing tests), `cargo clippy --all-targets --features server -- -D warnings`, `cargo check --no-default-features --features web --target wasm32-unknown-unknown`, WebAssembly bundling, and `docker build` pass. Local Unix Docker Engine inventory, login, same-origin enforcement, server CRUD, Prisma record import, provider JSON, provider status, UI login/navigation tested. Remote SSH/TLS, arm64, and live Traefik poll unavailable in this environment.

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
- [ ] Deploy against real remote SSH/TLS/HTTP socket-proxy Engines and Traefik v3; verify cross-host reachability and Docker-proxy permissions.
- [ ] Verify arm64 image build and deployment in CI environment with multiarch support.

## References

- Default recipes: https://rust-lang-nursery.github.io/rust-cookbook/
- Language: https://doc.rust-lang.org/book/
- Traefik HTTP provider: https://doc.traefik.io/traefik/reference/install-configuration/providers/others/http/
- Docker Engine API: https://docs.docker.com/reference/api/engine/
- Dioxus: https://dioxuslabs.com/
