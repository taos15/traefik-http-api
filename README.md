# Traefik HTTP API

Manage containers across multiple Docker hosts and publish their routes through a single [Traefik HTTP provider](https://doc.traefik.io/traefik/reference/install-configuration/providers/others/http/).

The WebUI lets you add Docker servers, browse and control containers, open their WebUIs, and inspect the configuration supplied to Traefik. Container labels select which applications to publish and override routing defaults.

- **Source:** [taos15/traefik-http-api](https://github.com/taos15/traefik-http-api)
- **Image:** `ghcr.io/taos15/traefik-http-api`
- **Developer guide:** [DEV_README.md](DEV_README.md)
- **API reference:** [Endpoints.MD](Endpoints.MD)

## Supported architectures and tags

Published images support **linux/amd64** and **linux/arm64**. Docker selects the matching architecture automatically.

| Tag | Description |
| --- | --- |
| `latest` | Most recently published stable release. |
| `edge` | Latest successfully published build from `master`; use for unreleased changes. |
| `1.2.3` | A specific release, when published. |
| `1.3.0-rc.1` | A specific prerelease, when published; does not update `latest`. |
| `sha-<full-commit>` | Build associated with a source commit. Pin an image digest for an immutable deployment. |

## Before you start

- Run this application and Traefik on the **same Docker host/daemon**, attached to the same **custom Docker bridge network**. The examples use `proxynet`.
- Connect additional Docker hosts through an HTTP socket proxy, SSH, or mutual TLS. Remote containers use a host address and published port reachable by Traefik.
- Configure DNS for application names such as `app.example.com` to point to Traefik. This application creates routes, not DNS records or certificates.
- Configure Traefik's entrypoints and TLS separately. Generated routers use an entrypoint named `https` unless overridden with `traefik.entrypoints`.

> [!WARNING]
> Docker socket access is highly privileged. A `:ro` socket mount does **not** make Docker API operations read-only. Protect the WebUI, do not expose an unauthenticated Docker API publicly, and use a restricted socket proxy where appropriate. Container controls require the corresponding Docker write permissions.

## Usage

### Docker Compose (recommended)

Create the network if it does not already exist, and attach your existing Traefik container to it through its deployment configuration:

```sh
docker network create proxynet
mkdir -p data
```

Create a `.env` file beside `compose.yml`:

```dotenv
DOMAIN=example.com
ADMIN_BOOTSTRAP_PASSWORD=replace-with-a-unique-long-password
```

Create `compose.yml`:

```yaml
services:
  api:
    image: ghcr.io/taos15/traefik-http-api:latest
    container_name: traefik-http-api
    environment:
      DOMAIN: ${DOMAIN:?set DOMAIN}
      DATABASE_URL: sqlite://data/traefik.db
      ADMIN_BOOTSTRAP_PASSWORD: ${ADMIN_BOOTSTRAP_PASSWORD:?set initial admin password}
    volumes:
      - ./data:/app/data
      - type: bind
        source: /var/run/docker.sock
        target: /var/run/docker.sock
        read_only: true
        bind:
          create_host_path: false
    ports:
      - "4000:4000"
    networks:
      - traefik
    restart: unless-stopped

networks:
  traefik:
    external: true
    name: proxynet
```

Start the application:

```sh
docker compose up -d
```

Open **`http://<server-ip>:4000`** and sign in as **`admin`** with your bootstrap password. The password must contain at least 12 characters and is used only to create the initial account; changing the environment variable later does not reset it.

Keep port 4000 on a trusted network, or restrict its binding/firewall access. Use HTTPS when exposing the WebUI through a reverse proxy.

### Deploy from source

To build your own image instead of pulling a published one:

```sh
git clone https://github.com/taos15/traefik-http-api.git
cd traefik-http-api
cp .env.example .env
mkdir -p data
```

Edit `.env` to set your domain, bootstrap password, and existing Traefik network name. The repository's `docker-compose.yml` builds from source and mounts `/var/run/docker.sock` read-only for discovery of this app container's attached custom networks. The socket must exist; Compose does not create a missing socket path. No additional discovery settings are required.

Then start it:

```sh
docker compose up -d --build
```

For SSH or mutual TLS instead, mount the required credentials read-only and use their **paths inside the application container** when adding a server. Native Rust setup and contributor commands are in [DEV_README.md](DEV_README.md).

## Traefik setup

Enable the HTTP provider in Traefik's **static configuration**:

```yaml
providers:
  http:
    endpoint: http://traefik-http-api:4000/api/v1/traefikconfig
```

Traefik polls this endpoint for the generated routes. The endpoint is anonymous by default; keep it on a trusted network. To require authentication, set `PROVIDER_TOKEN` on this application and configure the matching header in Traefik:

```yaml
providers:
  http:
    endpoint: http://traefik-http-api:4000/api/v1/traefikconfig
    headers:
      Authorization: "Bearer replace-with-your-provider-token"
```

Management endpoints and the WebUI require an admin login independently of this token. If the same containers are also handled by Traefik's Docker provider, avoid generating duplicate routes through both providers.

## Application setup

### Add a Docker server

Under **Servers**, enter a name and connection details:

| Transport | Endpoint / requirements |
| --- | --- |
| Unix socket | `/var/run/docker.sock`; mount the host socket into the application. |
| HTTP socket proxy | `http://docker-host:2375`; use a trusted, appropriately restricted proxy. |
| SSH | `ssh://user@docker-host`; make SSH keys, configuration, and known hosts available to the application. |
| Mutual TLS | A TLS endpoint with client key, certificate, and CA file paths accessible to the application. |

Checking **Prefer Docker network** automatically lists custom local bridge networks attached to **this application container**, using its local Docker socket. It never queries the Docker endpoint being added for this list. The app and Traefik must already share the selected network; selecting it does not attach containers. Enter the Traefik-reachable host manually. **Test connection** checks the configured server independently.

Use **Refresh networks** to reload the list, or **Retry** after a local discovery error. Changing the remote server's connection settings does not change this local list. Editing a server with network preference enabled loads networks automatically. A saved selection is retained if discovery fails or the network is missing, with a warning after a successful lookup cannot find it. Missing socket access or an unidentified app container produces a local discovery error, not a fallback request to a remote proxy.

Local discovery uses Docker's default container hostname to identify this app automatically. Keep the default hostname and standard socket mount shown above. Custom container hostnames, nonstandard socket paths, and native host processes are outside the local-network discovery contract; they do not affect published-port routing with network preference disabled.

| Setting | Behavior |
| --- | --- |
| **Traefik-reachable host** | Server hostname/IP used with published external ports. Enter a hostname or IP, without a URL scheme or port. |
| **Prefer Docker network** | Automatically loads a custom-network dropdown. For containers on that network, prefer container DNS name + mapped internal port; otherwise fall back to server host + external port. Selecting a network does not attach containers or Traefik to it. |
| **Same as server host** | Disables the browser-host input and uses the Traefik-reachable/server host address. Changes to that address also apply to browser links. |
| **Browser-reachable host** | Uncheck **Same as server host** to enter an independent hostname/IP reachable from your browser. Browser links use published external ports. |

The same settings are available when editing a server. Existing servers keep their explicit host settings until you opt into the new controls.

### Container routing contract

- Publish the application's TCP port in Docker and set `traefik.webuiport` to the **external host port** when the container has multiple port mappings. Without the override, routing needs one unambiguous published TCP port.
- With **Prefer Docker network**, the application, Traefik, and target container must share the selected custom network on the **same Docker daemon**. A network with the same name on a different host is not shared.
- The app uses the selected port mapping's **internal port for Docker DNS** and **external port for host-IP routing and browser links**. No additional internal-port label is needed.
- Host fallback needs a published binding reachable from Traefik. Loopback-only bindings are not suitable for remote host routing or generated browser links.
- The application must listen on the selected internal port on a container-reachable interface, not only its own loopback address.
- Fallback chooses a target when generating configuration; it is not connection-health failover. If neither target can be resolved, the Provider tab reports the error and the last successful configuration is retained.

For a container named `app`, published as `8080:80`, with `traefik.webuiport=8080`:

| Connection | Address |
| --- | --- |
| Preferred Docker-network target | `http://app:80` |
| Host fallback target | `http://<server-host>:8080` |
| Browser WebUI link | `http://<browser-host>:8080/` |

Only running, opted-in containers receive generated routes. The **Containers** tab also lists stopped and created containers, with filters and controls to start or manage them.

### Container cards and browser links

Each container card shows its Docker state separately from its Traefik routing status:

| Routing status | Meaning |
| --- | --- |
| **Routed** | A recent Traefik API observation confirms that the matching HTTP-provider router is enabled. This is not an application health or reachability check. |
| **Generated · unconfirmed** | This app generated a route, but Traefik has not recently confirmed it. The card explains unavailable/stale API data, a missing router, or a reported router error where known. |
| **Not routed** | The container has no route in this app's generated configuration. A known exclusion reason is shown where available. |
| **Status unavailable** | No usable routing snapshot or reliable container match is available. |

These statuses cover routes managed by this app, not routes independently created through Traefik's other providers. Confirmation requires access to Traefik's internal API; see [Traefik API settings](#parameters) below.

**Routed** is green with a lock when a recent Traefik observation confirms an enabled ForwardAuth middleware on that route, including through middleware chains. It is amber without a lock when ForwardAuth is absent or cannot be confirmed; the tooltip distinguishes those cases. The lock does not test the authentication service or a login flow, and does not indicate HTTPS.

**Route** links to the Traefik hostname independently of the **Browser URL**. It uses HTTP or HTTPS according to observed route TLS, defaulting to HTTPS when that information is unavailable. The link uses the scheme's standard port and does not imply application reachability, even when a generated route is unconfirmed.

The card's browser link is marked **Automatic** or **Custom**. Choose **Edit URL** to save a full `http://` or `https://` address, including an optional port, path, query, or fragment. Embedded usernames/passwords and other URL schemes are not accepted. You can set a custom URL even when automatic detection cannot produce a link.

**Reset to automatic** removes the saved override and restores the existing label/server-based link. Editing the browser URL changes only the card's browser link; it does not change Docker labels, published ports, or Traefik routes. Custom URLs are stored in the application's database per Docker server and container ID, survive refreshes and app restarts, and take precedence over automatic browser-link settings. A recreated container with a new ID starts in automatic mode.

## Container labels

Set these labels on the **application containers**, not just on this service. They are interpreted by this application's HTTP-provider generator; they are not a replacement for Traefik's own Docker-provider label reference.

| Label | Default / meaning |
| --- | --- |
| `traefik.enable=true` | Opt into generated routing. Containers are excluded unless enabled by this label or legacy `swag=enable`. |
| `traefik.name` | Override the router name. Default: container name with its first character capitalized. Keep generated router/service names unique. |
| `traefik.hostname` | Override the hostname prefix. Default: router name. `photos` with `DOMAIN=example.com` produces `photos.example.com`; do not supply the full domain here. |
| `traefik.service` | Override the service name. Default: router name. |
| `traefik.webuiport` | Select a **published external TCP port** and its Docker mapping. With `8080:80`, use `8080`, not `80`. Also supplies the direct browser-link port when no explicit WebUI-template port takes precedence. |
| `traefik.service.scheme` | Backend connection scheme: `http` (default) or `https`. Does not configure public-facing TLS. |
| `traefik.entrypoints` | Comma-separated Traefik entrypoints. Default: `https`. Names must exist in Traefik's static configuration. |
| `traefik.http.disable=true` | Suppress this container's generated HTTP route even when opted in. |
| `traefik.middlewares` | Explicit comma-separated middleware list, replacing the automatic default. Use provider suffixes such as `headers@file` when needed. |
| `traefik.forwardauth=false` | Opt out of automatic default authentication for this container. |
| `traefik.webui.host` | Override the server-level browser host for this container only; hostname/IP without scheme or port. |
| `traefik.webui.url` | Browser-link template for scheme, path, query, and port. Supports `[IP]` and `[PORT:internal-port]`; the browser host still follows the configured host/override. |
| `traefik.icon` | HTTPS image URL for the container card. |

**Compatibility labels:** `swag=enable` opts into routing; `authelia_auth=false` is an alias for opting out of default auth. `net.unraid.docker.webui` is supported and takes precedence over `traefik.webui.url`. Icons fall back to `net.unraid.docker.icon`, then `com.docker.desktop.extension.icon`, then a local two-letter placeholder.

Without an explicit WebUI template/port, browser-link detection excludes common database and other non-HTTP ports. Docker labels are set at container creation; recreate a container after changing its labels.

Example additions to an application's Compose service, assuming it listens on internal port 80:

```yaml
services:
  app:
    # Keep your application's image, volumes, and other settings.
    container_name: app
    ports:
      - "8080:80"
    labels:
      traefik.enable: "true"
      traefik.hostname: "app"
      traefik.webuiport: "8080"
      traefik.webui.url: "http://[IP]:[PORT:80]/"
    networks:
      - traefik

networks:
  traefik:
    external: true
    name: proxynet
```

This produces a route for `app.<DOMAIN>`. The example is a service-configuration excerpt, not a standalone application deployment.

## Parameters

### Ports and volumes

| Mapping | Purpose |
| --- | --- |
| `4000:4000` | WebUI, management API, and HTTP-provider endpoint. Restrict external access as appropriate. |
| `./data:/app/data` | Persistent SQLite database. Back up this directory before upgrading. |
| `/var/run/docker.sock:/var/run/docker.sock:ro` | Optional local Docker socket access; omit when using another connection method. |
| Credential/certificate mounts | Optional SSH configuration or mutual-TLS files; mount read-only at the paths used in server settings. |

### Environment variables

| Variable | Purpose |
| --- | --- |
| `DOMAIN` | **Required.** Base domain appended to generated application hostnames. |
| `ADMIN_BOOTSTRAP_PASSWORD` | **Required on first start.** Initial `admin` password, at least 12 characters. |
| `DATABASE_URL` | SQLite location. Default: `sqlite://data/traefik.db`; relative to `/app` in the image. |
| `LISTEN_ADDR` | Listener address. Default: `0.0.0.0:4000`. |
| `PROVIDER_TOKEN` | Optional bearer token required for Traefik's polling endpoint. |
| `TRAEFIK_API_URL` | Optional internal Traefik API base URL, for example `http://traefik:8080`. Otherwise detected from the Traefik container name. |
| `TRAEFIK_API_USERNAME` | Optional Basic-auth username for the internal Traefik API. |
| `TRAEFIK_API_PASSWORD` | Matching internal API password. |
| `TRAEFIK_API_PASSWORD_FILE` | Read that password from a mounted file instead; takes precedence over `TRAEFIK_API_PASSWORD`. |
| `TRAEFIK_NETWORK` | Repository Compose setting for the external network name; defaults to `proxynet`. Select the network separately in the WebUI. |

## Authentication and Provider tools

The **Provider** tab shows generated configuration, authentication selection, and Traefik API status. The app reads Traefik's internal `/api/rawdata`; keep that API private. For a secured API, set its internal URL and Basic-auth credentials above. Browser-only ForwardAuth login does not authenticate the application's internal API requests.

Automatic auth detects **OAuth2 Proxy**, then **Authentik**, then **Authelia**, on Traefik's daemon and shared custom network. A `default-forward-auth` middleware defined in Traefik's file provider takes precedence over automatic generation. Authentik/OAuth2 Proxy still need their outpost/sign-in path routers configured in Traefik.

> [!IMPORTANT]
> If no default authentication has ever been configured, generated routes are published without it. Once a protected default is confirmed unavailable, affected routes are withheld instead of silently made public. Use `traefik.forwardauth=false` only when intentionally exposing a container without the default auth.

**Generate config** downloads are redacted recovery aids, not full backups of your Traefik files. They cover effective HTTP resources only; review them, re-enter redacted secrets, and avoid duplicate resource names before use. **Generate @file snippet** helps configure auth containers outside Traefik's shared network using a reachable published port. Advanced API/auth setup and export limitations are in [DEV_README.md](DEV_README.md#traefik-integration-details).

## Updating and support

For the published-image Compose deployment:

```sh
docker compose pull api
docker compose up -d api
docker compose logs -f api
```

For a source deployment, update the checkout and repeat its `up -d --build` command with the same Compose files. Keep the `data` directory; do not delete it when recreating the container.

**Upgrading from the old TypeScript/Prisma version:** stop the old service and back up its SQLite database first. Migrate a copy, set a new bootstrap password, and review imported server host addresses before using generated routes. Legacy user passwords are not imported. See [migration details](DEV_README.md#legacy-database-migration) before upgrading.

For troubleshooting, check the **Provider** tab and `docker compose logs -f api`. Report problems through [GitHub Issues](https://github.com/taos15/traefik-http-api/issues), including the image tag/digest and redacted logs—never passwords, tokens, or private keys.
