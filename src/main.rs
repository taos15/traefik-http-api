use std::{
    collections::HashMap,
    env,
    net::SocketAddr,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::Context;
use argon2::{Argon2, PasswordHash, PasswordHasher, PasswordVerifier, password_hash::SaltString};
use axum::{
    Json, Router,
    extract::{Path, Query, Request, State},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, post, put},
};
use bollard::query_parameters::{RestartContainerOptionsBuilder, StopContainerOptionsBuilder};
use rand::{RngCore, rngs::OsRng};
use serde::{Deserialize, Serialize};
use sqlx::{
    Row, SqlitePool,
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions},
};
use tokio::sync::{Mutex, RwLock, watch};
use tower::ServiceExt;
use tower_http::{services::ServeDir, trace::TraceLayer};
use traefik_http_api::{
    Container, DockerServer, ServerDiscovery, ServerInput, auth, docker, provider, traefik,
};

#[derive(Clone)]
struct Settings {
    domain: String,
    traefik_api_url: Option<String>,
    traefik_api_username: Option<String>,
    traefik_api_password: Option<String>,
    provider_token: Option<String>,
    database_url: String,
}

#[derive(Clone)]
struct AppState {
    db: SqlitePool,
    settings: Settings,
    refresh: Arc<Mutex<()>>,
    trigger: watch::Sender<u64>,
    last_error: Arc<RwLock<Option<String>>>,
    api_error: Arc<RwLock<Option<String>>>,
    auth_status: Arc<RwLock<Option<AuthStatus>>>,
    route_warnings: Arc<RwLock<Vec<String>>>,
}

#[derive(Debug)]
struct ApiError(StatusCode, String);
impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.0, Json(serde_json::json!({"error":self.1}))).into_response()
    }
}
type ApiResult<T> = Result<T, ApiError>;
fn bad(message: impl ToString) -> ApiError {
    ApiError(StatusCode::BAD_REQUEST, message.to_string())
}
fn internal(error: impl ToString) -> ApiError {
    tracing::error!(message = %error.to_string());
    ApiError(
        StatusCode::INTERNAL_SERVER_ERROR,
        "internal server error".into(),
    )
}
fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

fn settings() -> anyhow::Result<Settings> {
    let domain = env::var("DOMAIN").context("DOMAIN is required")?;
    anyhow::ensure!(!domain.is_empty(), "DOMAIN must not be empty");
    let traefik_api_password = if let Ok(path) = env::var("TRAEFIK_API_PASSWORD_FILE")
        && !path.is_empty()
    {
        Some(
            std::fs::read_to_string(path)?
                .trim_end_matches(['\r', '\n'])
                .to_owned(),
        )
    } else {
        env::var("TRAEFIK_API_PASSWORD")
            .ok()
            .filter(|s| !s.is_empty())
    };
    let provider_token = env::var("PROVIDER_TOKEN").ok().filter(|s| !s.is_empty());
    let database_url =
        env::var("DATABASE_URL").unwrap_or_else(|_| "sqlite://data/traefik.db".into());
    Ok(Settings {
        domain,
        traefik_api_url: env::var("TRAEFIK_API_URL").ok().filter(|s| !s.is_empty()),
        traefik_api_username: env::var("TRAEFIK_API_USERNAME")
            .ok()
            .filter(|s| !s.is_empty()),
        traefik_api_password,
        provider_token,
        database_url,
    })
}

async fn servers(db: &SqlitePool) -> anyhow::Result<Vec<DockerServer>> {
    let rows = sqlx::query("SELECT id,name,transport,endpoint,tls_key,tls_cert,tls_ca,upstream_host,webui_host,prefer_network,docker_network,webui_same_as_server,enabled FROM docker_servers ORDER BY name")
        .fetch_all(db).await?;
    Ok(rows
        .into_iter()
        .map(|r| DockerServer {
            id: r.get("id"),
            name: r.get("name"),
            transport: r.get("transport"),
            endpoint: r.get("endpoint"),
            tls_key: r.get("tls_key"),
            tls_cert: r.get("tls_cert"),
            tls_ca: r.get("tls_ca"),
            upstream_host: r.get("upstream_host"),
            webui_host: r.get("webui_host"),
            prefer_network: r.get::<i64, _>("prefer_network") != 0,
            docker_network: r.get("docker_network"),
            webui_same_as_server: r.get::<i64, _>("webui_same_as_server") != 0,
            enabled: r.get::<i64, _>("enabled") != 0,
        })
        .collect())
}

fn validate_connection(input: &ServerInput) -> anyhow::Result<()> {
    anyhow::ensure!(!input.endpoint.trim().is_empty(), "endpoint required");
    match input.transport.as_str() {
        "http" => anyhow::ensure!(
            input.endpoint.starts_with("http://"),
            "HTTP endpoint must start with http://"
        ),
        "tls" => anyhow::ensure!(
            input.tls_key.is_some() && input.tls_cert.is_some() && input.tls_ca.is_some(),
            "TLS needs key, cert, CA paths"
        ),
        "ssh" => anyhow::ensure!(
            input.endpoint.starts_with("ssh://"),
            "SSH endpoint must start with ssh://"
        ),
        "unix" => anyhow::ensure!(
            input.endpoint.starts_with('/'),
            "Unix endpoint must be an absolute socket path"
        ),
        _ => anyhow::bail!("invalid transport"),
    }
    Ok(())
}

fn validate_host(host: &str) -> anyhow::Result<()> {
    let parsed = url::Url::parse(&format!("http://{host}"))?;
    anyhow::ensure!(
        parsed.host_str().is_some()
            && parsed.port().is_none()
            && parsed.path() == "/"
            && parsed.query().is_none()
            && parsed.username().is_empty()
            && parsed.password().is_none(),
        "hosts must be hostnames or IP addresses, without port or scheme"
    );
    anyhow::ensure!(
        !matches!(parsed.host_str(), Some("0.0.0.0" | "::")),
        "unspecified host cannot be used as upstream or WebUI host"
    );
    Ok(())
}

fn validate(input: &ServerInput) -> anyhow::Result<()> {
    validate_connection(input)?;
    anyhow::ensure!(!input.name.trim().is_empty(), "name required");
    anyhow::ensure!(
        !input.upstream_host.trim().is_empty(),
        "upstream host required"
    );
    if !input.webui_same_as_server {
        anyhow::ensure!(!input.webui_host.trim().is_empty(), "WebUI host required");
    }
    if input.prefer_network {
        anyhow::ensure!(
            input
                .docker_network
                .as_deref()
                .is_some_and(|network| !network.trim().is_empty()),
            "Docker network required when network preference is enabled"
        );
    }
    for h in std::iter::once(input.upstream_host.as_str())
        .chain((!input.webui_same_as_server).then_some(input.webui_host.as_str()))
    {
        validate_host(h)?;
    }
    Ok(())
}

fn server_from_input(id: String, input: ServerInput) -> DockerServer {
    DockerServer {
        id,
        name: input.name,
        transport: input.transport,
        endpoint: input.endpoint,
        tls_key: input.tls_key,
        tls_cert: input.tls_cert,
        tls_ca: input.tls_ca,
        upstream_host: input.upstream_host,
        webui_host: input.webui_host,
        prefer_network: input.prefer_network,
        docker_network: input.docker_network,
        webui_same_as_server: input.webui_same_as_server,
        enabled: input.enabled,
    }
}

fn load_session(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(header::COOKIE)?
        .to_str()
        .ok()?
        .split(';')
        .find_map(|c| c.trim().strip_prefix("session="))
}
async fn require_login(state: &AppState, headers: &HeaderMap) -> ApiResult<()> {
    let token = load_session(headers)
        .ok_or_else(|| ApiError(StatusCode::UNAUTHORIZED, "login required".into()))?;
    let hash = sha_token(token);
    let exists: Option<(String,)> =
        sqlx::query_as("SELECT user_id FROM sessions WHERE token_hash=? AND expires_at>?")
            .bind(hash)
            .bind(now())
            .fetch_optional(&state.db)
            .await
            .map_err(internal)?;
    exists
        .map(|_| ())
        .ok_or_else(|| ApiError(StatusCode::UNAUTHORIZED, "login required".into()))
}
fn require_same_origin(headers: &HeaderMap) -> ApiResult<()> {
    let origin = headers
        .get(header::ORIGIN)
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| ApiError(StatusCode::FORBIDDEN, "Origin header required".into()))?;
    let host = headers
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| ApiError(StatusCode::FORBIDDEN, "Host header required".into()))?;
    let url = url::Url::parse(origin).map_err(bad)?;
    let base = url::Url::parse(&format!("{}://{host}", url.scheme())).map_err(bad)?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || url.host_str() != base.host_str()
        || url.port_or_known_default() != base.port_or_known_default()
    {
        return Err(ApiError(
            StatusCode::FORBIDDEN,
            "cross-origin request rejected".into(),
        ));
    }
    Ok(())
}
fn sha_token(token: &str) -> String {
    // SHA-256 session lookup; token itself is a random 256-bit value.
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::digest(token.as_bytes()))
}

#[derive(Deserialize)]
struct Login {
    username: String,
    password: String,
}
async fn login(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(form): Json<Login>,
) -> ApiResult<Response> {
    require_same_origin(&headers)?;
    let row: Option<(String, String)> =
        sqlx::query_as("SELECT id,password_hash FROM users WHERE username=?")
            .bind(&form.username)
            .fetch_optional(&state.db)
            .await
            .map_err(internal)?;
    let (id, hash) =
        row.ok_or_else(|| ApiError(StatusCode::UNAUTHORIZED, "invalid credentials".into()))?;
    let password = form.password;
    let valid = tokio::task::spawn_blocking(move || {
        PasswordHash::new(&hash).ok().is_some_and(|parsed| {
            Argon2::default()
                .verify_password(password.as_bytes(), &parsed)
                .is_ok()
        })
    })
    .await
    .map_err(internal)?;
    if !valid {
        return Err(ApiError(
            StatusCode::UNAUTHORIZED,
            "invalid credentials".into(),
        ));
    }
    let mut bytes = [0u8; 32];
    OsRng.fill_bytes(&mut bytes);
    let token = hex::encode(bytes);
    sqlx::query("INSERT INTO sessions(token_hash,user_id,expires_at) VALUES(?,?,?)")
        .bind(sha_token(&token))
        .bind(id)
        .bind(now() + 86400)
        .execute(&state.db)
        .await
        .map_err(internal)?;
    let mut response = StatusCode::NO_CONTENT.into_response();
    response.headers_mut().insert(
        header::SET_COOKIE,
        format!("session={token}; HttpOnly; SameSite=Strict; Path=/; Max-Age=86400")
            .parse()
            .map_err(internal)?,
    );
    Ok(response)
}
async fn logout(State(state): State<AppState>, headers: HeaderMap) -> ApiResult<Response> {
    require_same_origin(&headers)?;
    require_login(&state, &headers).await?;
    if let Some(token) = load_session(&headers) {
        sqlx::query("DELETE FROM sessions WHERE token_hash=?")
            .bind(sha_token(token))
            .execute(&state.db)
            .await
            .map_err(internal)?;
    }
    let mut response = StatusCode::NO_CONTENT.into_response();
    response.headers_mut().insert(
        header::SET_COOKIE,
        "session=; HttpOnly; SameSite=Strict; Path=/; Max-Age=0"
            .parse()
            .unwrap(),
    );
    Ok(response)
}
async fn list_servers(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> ApiResult<Json<Vec<DockerServer>>> {
    require_login(&state, &headers).await?;
    Ok(Json(servers(&state.db).await.map_err(internal)?))
}
async fn create_server(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<ServerInput>,
) -> ApiResult<(StatusCode, Json<DockerServer>)> {
    require_login(&state, &headers).await?;
    require_same_origin(&headers)?;
    validate(&input).map_err(bad)?;
    let s = server_from_input(uuid::Uuid::new_v4().to_string(), input);
    sqlx::query("INSERT INTO docker_servers (id,name,transport,endpoint,tls_key,tls_cert,tls_ca,upstream_host,webui_host,prefer_network,docker_network,webui_same_as_server,enabled) VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?)")
        .bind(&s.id).bind(&s.name).bind(&s.transport).bind(&s.endpoint).bind(&s.tls_key).bind(&s.tls_cert).bind(&s.tls_ca)
        .bind(&s.upstream_host).bind(&s.webui_host).bind(s.prefer_network).bind(&s.docker_network).bind(s.webui_same_as_server)
        .bind(s.enabled).execute(&state.db).await.map_err(bad)?;
    trigger(&state);
    Ok((StatusCode::CREATED, Json(s)))
}
async fn update_server(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(input): Json<ServerInput>,
) -> ApiResult<Json<DockerServer>> {
    require_login(&state, &headers).await?;
    require_same_origin(&headers)?;
    validate(&input).map_err(bad)?;
    let changed = sqlx::query("UPDATE docker_servers SET name=?,transport=?,endpoint=?,tls_key=?,tls_cert=?,tls_ca=?,upstream_host=?,webui_host=?,prefer_network=?,docker_network=?,webui_same_as_server=?,enabled=? WHERE id=?")
        .bind(&input.name).bind(&input.transport).bind(&input.endpoint).bind(&input.tls_key).bind(&input.tls_cert).bind(&input.tls_ca)
        .bind(&input.upstream_host).bind(&input.webui_host).bind(input.prefer_network).bind(&input.docker_network)
        .bind(input.webui_same_as_server).bind(input.enabled).bind(&id).execute(&state.db).await.map_err(bad)?.rows_affected();
    if changed == 0 {
        return Err(ApiError(StatusCode::NOT_FOUND, "server not found".into()));
    }
    remove_server_routes(&state, &id).await.map_err(internal)?;
    trigger(&state);
    Ok(Json(server_from_input(id, input)))
}
async fn delete_server(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> ApiResult<StatusCode> {
    require_login(&state, &headers).await?;
    require_same_origin(&headers)?;
    let affected = sqlx::query("DELETE FROM docker_servers WHERE id=?")
        .bind(&id)
        .execute(&state.db)
        .await
        .map_err(internal)?
        .rows_affected();
    if affected == 0 {
        return Err(ApiError(StatusCode::NOT_FOUND, "server not found".into()));
    }
    sqlx::query("DELETE FROM container_webui_urls WHERE server_id=?")
        .bind(&id)
        .execute(&state.db)
        .await
        .map_err(internal)?;
    remove_server_routes(&state, &id).await.map_err(internal)?;
    trigger(&state);
    Ok(StatusCode::NO_CONTENT)
}
async fn test_server(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<ServerInput>,
) -> ApiResult<Json<serde_json::Value>> {
    require_login(&state, &headers).await?;
    require_same_origin(&headers)?;
    validate_connection(&input).map_err(bad)?;
    let s = server_from_input(String::new(), input);
    let result = docker::list(s)
        .await
        .map_err(|e| ApiError(StatusCode::BAD_GATEWAY, e.to_string()))?;
    Ok(Json(
        serde_json::json!({"ok":true,"containers":result.len()}),
    ))
}

async fn discover_server(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<ServerInput>,
) -> ApiResult<Json<ServerDiscovery>> {
    require_login(&state, &headers).await?;
    require_same_origin(&headers)?;
    validate_connection(&input).map_err(bad)?;
    let server = server_from_input(String::new(), input);
    Ok(Json(docker::discover(&server).await.map_err(|error| {
        ApiError(StatusCode::BAD_GATEWAY, error.to_string())
    })?))
}

async fn container_webui_urls(
    db: &SqlitePool,
) -> anyhow::Result<HashMap<(String, String), Option<String>>> {
    let rows = sqlx::query("SELECT server_id, container_id, url FROM container_webui_urls")
        .fetch_all(db)
        .await?;
    Ok(rows
        .into_iter()
        .map(|r| {
            (
                (r.get("server_id"), r.get("container_id")),
                r.get::<Option<String>, _>("url"),
            )
        })
        .collect())
}

async fn set_container_webui_url(
    db: &SqlitePool,
    server_id: &str,
    container_id: &str,
    url: Option<&str>,
) -> anyhow::Result<()> {
    if let Some(url) = url {
        sqlx::query(
            "INSERT INTO container_webui_urls(server_id,container_id,url) VALUES(?,?,?) ON CONFLICT(server_id,container_id) DO UPDATE SET url=excluded.url",
        )
        .bind(server_id)
        .bind(container_id)
        .bind(url)
        .execute(db)
        .await?;
    } else {
        sqlx::query("DELETE FROM container_webui_urls WHERE server_id=? AND container_id=?")
            .bind(server_id)
            .bind(container_id)
            .execute(db)
            .await?;
    }
    Ok(())
}

fn validate_webui_url_override(url: &str) -> anyhow::Result<String> {
    let url = url.trim();
    let scheme = url.split_once("://").map(|(s, _)| s.to_ascii_lowercase());
    anyhow::ensure!(
        scheme.as_deref() == Some("http") || scheme.as_deref() == Some("https"),
        "URL must start with http:// or https://"
    );
    let parsed = url::Url::parse(url).map_err(|e| anyhow::anyhow!("invalid URL: {e}"))?;
    anyhow::ensure!(parsed.host_str().is_some(), "URL must include a host");
    anyhow::ensure!(
        parsed.username().is_empty() && parsed.password().is_none(),
        "URL must not contain credentials"
    );
    Ok(url.to_owned())
}

async fn provider_snapshot(db: &SqlitePool) -> anyhow::Result<Option<serde_json::Value>> {
    let row: Option<(String,)> = sqlx::query_as("SELECT config FROM provider_snapshot WHERE id=1")
        .fetch_optional(db)
        .await?;
    row.map(|r| serde_json::from_str(&r.0).map_err(anyhow::Error::from))
        .transpose()
}

fn host_from_rule(rule: &str) -> Option<String> {
    // Extract the first Host(`...`) value from a Traefik rule.
    rule.split("Host(")
        .nth(1)?
        .split(')')
        .next()?
        .trim_matches('`')
        .split(',')
        .next()
        .map(|s| s.to_owned())
}

#[derive(Clone)]
struct RouteInfo {
    router_name: String,
    service_name: String,
    rule: String,
    upstream_fingerprint: String,
}

async fn persisted_routes(
    db: &SqlitePool,
) -> anyhow::Result<(
    HashMap<(String, String), RouteInfo>,
    HashMap<(String, String), RouteInfo>,
)> {
    let rows = sqlx::query(
        "SELECT router_name, service_name, server_id, container_id, rule, upstream_fingerprint FROM provider_routes",
    )
    .fetch_all(db)
    .await?;
    let mut by_container = HashMap::new();
    let mut legacy = HashMap::new();
    for row in rows {
        let info = RouteInfo {
            router_name: row.get("router_name"),
            service_name: row.get("service_name"),
            rule: row.get::<Option<String>, _>("rule").unwrap_or_default(),
            upstream_fingerprint: row
                .get::<Option<String>, _>("upstream_fingerprint")
                .unwrap_or_default(),
        };
        let server_id: String = row.get("server_id");
        if let Some(container_id) = row.get::<Option<String>, _>("container_id") {
            by_container.insert((server_id, container_id), info);
        } else {
            legacy.insert((server_id, info.router_name.clone()), info);
        }
    }
    Ok((by_container, legacy))
}

fn router_name(c: &Container) -> String {
    let fallback = c
        .name
        .chars()
        .next()
        .map(|first| first.to_uppercase().collect::<String>() + &c.name[first.len_utf8()..])
        .unwrap_or_default();
    c.labels.get("traefik.name").cloned().unwrap_or(fallback)
}

fn container_routing(
    c: &Container,
    snapshot: &Option<serde_json::Value>,
    by_container: &HashMap<(String, String), RouteInfo>,
    legacy: &HashMap<(String, String), RouteInfo>,
    runtime: &traefik::RuntimeView,
    now: i64,
) -> traefik_http_api::ContainerRouting {
    use traefik_http_api::ContainerRouting;

    fn stale_unconfirmed(
        info: &RouteInfo,
        runtime: &traefik::RuntimeView,
        reason: &str,
    ) -> traefik_http_api::ContainerRouting {
        let hostname = host_from_rule(&info.rule);
        ContainerRouting {
            state: "unconfirmed".into(),
            reason: Some(reason.into()),
            hostname,
            checked_at: runtime.fetched_at,
            forward_auth: None,
            tls: None,
        }
    }

    if snapshot.is_none() {
        if c.state != "running" {
            return ContainerRouting {
                state: "not_routed".into(),
                reason: Some("Container stopped".into()),
                hostname: None,
                checked_at: None,
                forward_auth: None,
                tls: None,
            };
        }
        return ContainerRouting {
            state: "unavailable".into(),
            reason: Some("Provider snapshot not yet available".into()),
            hostname: None,
            checked_at: None,
            forward_auth: None,
            tls: None,
        };
    }

    let key = (c.server_id.clone(), c.id.clone());
    let (info, legacy_match) = if let Some(info) = by_container.get(&key) {
        (Some(info.clone()), false)
    } else {
        let name = router_name(c);
        match legacy.get(&(c.server_id.clone(), name)) {
            Some(info) => (Some(info.clone()), true),
            None => (None, false),
        }
    };

    if legacy_match {
        return stale_unconfirmed(
            &info.expect("legacy_match implies info"),
            runtime,
            "Route ownership pending refresh",
        );
    }

    if c.state != "running" {
        return match info {
            Some(info) => stale_unconfirmed(
                &info,
                runtime,
                "Container stopped; route ownership pending refresh",
            ),
            None => ContainerRouting {
                state: "not_routed".into(),
                reason: Some("Container stopped".into()),
                hostname: None,
                checked_at: None,
                forward_auth: None,
                tls: None,
            },
        };
    }

    if let Some(reason) = provider::route_excluded_reason(c) {
        return ContainerRouting {
            state: "not_routed".into(),
            reason: Some(reason.into()),
            hostname: None,
            checked_at: None,
            forward_auth: None,
            tls: None,
        };
    }

    let Some(info) = info else {
        return ContainerRouting {
            state: "not_routed".into(),
            reason: Some("Route not generated (check configuration)".into()),
            hostname: None,
            checked_at: None,
            forward_auth: None,
            tls: None,
        };
    };

    let hostname = host_from_rule(&info.rule);

    if !runtime.api_reachable {
        return ContainerRouting {
            state: "unconfirmed".into(),
            reason: Some("Traefik API is unreachable".into()),
            hostname,
            checked_at: runtime.fetched_at,
            forward_auth: None,
            tls: None,
        };
    }

    const STALE_CUTOFF_SECONDS: i64 = 30;
    let stale = match runtime.fetched_at {
        None => true,
        Some(fetched_at) => now - fetched_at > STALE_CUTOFF_SECONDS,
    };
    if stale {
        return ContainerRouting {
            state: "unconfirmed".into(),
            reason: Some("Traefik API data is stale".into()),
            hostname,
            checked_at: runtime.fetched_at,
            forward_auth: None,
            tls: None,
        };
    }

    let Some(router) = runtime.routers.get(&info.router_name) else {
        return ContainerRouting {
            state: "unconfirmed".into(),
            reason: Some("Generated route not yet observed in Traefik".into()),
            hostname,
            checked_at: runtime.fetched_at,
            forward_auth: None,
            tls: None,
        };
    };

    if router.status != "enabled" {
        return ContainerRouting {
            state: "unconfirmed".into(),
            reason: Some(format!("Router status is {}", router.status)),
            hostname,
            checked_at: runtime.fetched_at,
            forward_auth: None,
            tls: None,
        };
    }

    let runtime_service_name = router
        .service
        .strip_suffix("@http")
        .unwrap_or(&router.service);
    if runtime_service_name != info.service_name {
        return ContainerRouting {
            state: "unconfirmed".into(),
            reason: Some("Runtime router points to a different service".into()),
            hostname,
            checked_at: runtime.fetched_at,
            forward_auth: None,
            tls: None,
        };
    }

    if router.rule != info.rule {
        return ContainerRouting {
            state: "unconfirmed".into(),
            reason: Some("Runtime router rule does not match generated rule".into()),
            hostname,
            checked_at: runtime.fetched_at,
            forward_auth: None,
            tls: None,
        };
    }

    let runtime_fp = runtime
        .services
        .get(&info.service_name)
        .or_else(|| runtime.services.get(&format!("{}@http", info.service_name)));
    if runtime_fp.map(|fp| fp.as_str()) != Some(info.upstream_fingerprint.as_str()) {
        return ContainerRouting {
            state: "unconfirmed".into(),
            reason: Some("Runtime service upstream does not match generated service".into()),
            hostname,
            checked_at: runtime.fetched_at,
            forward_auth: None,
            tls: None,
        };
    }

    ContainerRouting {
        state: "routed".into(),
        reason: None,
        hostname,
        checked_at: runtime.fetched_at,
        forward_auth: router.forward_auth,
        tls: router.tls,
    }
}

async fn inventory(state: &AppState) -> anyhow::Result<Vec<Container>> {
    let servers = servers(&state.db).await?;
    let index = servers
        .iter()
        .map(|s| (s.id.as_str(), s))
        .collect::<std::collections::HashMap<_, _>>();
    let mut all = docker::aggregate(servers.clone()).await?;
    let overrides = container_webui_urls(&state.db).await?;
    let snapshot = provider_snapshot(&state.db).await?;
    let runtime = traefik::load_runtime(&state.db).await?;
    let (by_container, legacy) = persisted_routes(&state.db).await?;
    let t = now();
    for c in &mut all {
        let default_url = provider::webui_url(c, index[&c.server_id.as_str()]);
        c.webui_url_default = default_url.clone();
        c.webui_url_override = overrides
            .get(&(c.server_id.clone(), c.id.clone()))
            .and_then(|v| v.clone());
        c.webui_url = c.webui_url_override.clone().or(default_url);
        c.routing = Some(container_routing(
            c,
            &snapshot,
            &by_container,
            &legacy,
            &runtime,
            t,
        ));
    }
    Ok(all)
}
async fn list_containers(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> ApiResult<Json<Vec<Container>>> {
    require_login(&state, &headers).await?;
    Ok(Json(inventory(&state).await.map_err(|e| {
        ApiError(StatusCode::BAD_GATEWAY, e.to_string())
    })?))
}
async fn inspect_container(
    State(state): State<AppState>,
    Path((server_id, id)): Path<(String, String)>,
    headers: HeaderMap,
) -> ApiResult<Json<serde_json::Value>> {
    require_login(&state, &headers).await?;
    let server = servers(&state.db)
        .await
        .map_err(internal)?
        .into_iter()
        .find(|s| s.id == server_id && s.enabled)
        .ok_or_else(|| ApiError(StatusCode::NOT_FOUND, "server not found".into()))?;
    let d = tokio::time::timeout(
        Duration::from_secs(12),
        docker::connect(&server).map_err(bad)?.negotiate_version(),
    )
    .await
    .map_err(|e| ApiError(StatusCode::GATEWAY_TIMEOUT, e.to_string()))?
    .map_err(|e| ApiError(StatusCode::BAD_GATEWAY, e.to_string()))?;
    let inspect = tokio::time::timeout(Duration::from_secs(12), d.inspect_container(&id, None))
        .await
        .map_err(|e| ApiError(StatusCode::GATEWAY_TIMEOUT, e.to_string()))?
        .map_err(|e| ApiError(StatusCode::BAD_GATEWAY, e.to_string()))?;
    Ok(Json(serde_json::to_value(inspect).map_err(internal)?))
}
#[derive(Deserialize)]
struct Action {
    command: String,
}
async fn container_action(
    State(state): State<AppState>,
    Path((server_id, id)): Path<(String, String)>,
    headers: HeaderMap,
    Json(action): Json<Action>,
) -> ApiResult<StatusCode> {
    require_login(&state, &headers).await?;
    require_same_origin(&headers)?;
    let server = servers(&state.db)
        .await
        .map_err(internal)?
        .into_iter()
        .find(|s| s.id == server_id && s.enabled)
        .ok_or_else(|| ApiError(StatusCode::NOT_FOUND, "server not found".into()))?;
    let d = tokio::time::timeout(
        Duration::from_secs(12),
        docker::connect(&server).map_err(bad)?.negotiate_version(),
    )
    .await
    .map_err(|e| ApiError(StatusCode::GATEWAY_TIMEOUT, e.to_string()))?
    .map_err(|e| ApiError(StatusCode::BAD_GATEWAY, e.to_string()))?;
    let command = async {
        match action.command.as_str() {
            "start" => d.start_container(&id, None).await,
            "stop" => {
                d.stop_container(&id, Some(StopContainerOptionsBuilder::default().build()))
                    .await
            }
            "pause" => d.pause_container(&id).await,
            "unpause" => d.unpause_container(&id).await,
            "restart" => {
                d.restart_container(&id, Some(RestartContainerOptionsBuilder::default().build()))
                    .await
            }
            _ => return Err(ApiError(StatusCode::BAD_REQUEST, "invalid command".into())),
        }
        .map_err(|e| ApiError(StatusCode::BAD_GATEWAY, e.to_string()))
    };
    tokio::time::timeout(Duration::from_secs(30), command)
        .await
        .map_err(|e| ApiError(StatusCode::GATEWAY_TIMEOUT, e.to_string()))??;
    trigger(&state);
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
struct WebuiUrlInput {
    url: Option<String>,
}

async fn update_webui_url(
    State(state): State<AppState>,
    Path((server_id, container_id)): Path<(String, String)>,
    headers: HeaderMap,
    Json(input): Json<WebuiUrlInput>,
) -> ApiResult<StatusCode> {
    require_login(&state, &headers).await?;
    require_same_origin(&headers)?;
    let server = servers(&state.db)
        .await
        .map_err(internal)?
        .into_iter()
        .find(|s| s.id == server_id && s.enabled)
        .ok_or_else(|| ApiError(StatusCode::NOT_FOUND, "server not found".into()))?;
    let containers = docker::list(server)
        .await
        .map_err(|e| ApiError(StatusCode::BAD_GATEWAY, e.to_string()))?;
    if !containers.iter().any(|c| c.id == container_id) {
        return Err(ApiError(
            StatusCode::NOT_FOUND,
            "container not found".into(),
        ));
    }
    let normalized = match input.url {
        Some(ref url) => Some(validate_webui_url_override(url).map_err(bad)?),
        None => None,
    };
    set_container_webui_url(&state.db, &server_id, &container_id, normalized.as_deref())
        .await
        .map_err(internal)?;
    trigger(&state);
    Ok(StatusCode::NO_CONTENT)
}

fn trigger(s: &AppState) {
    s.trigger.send_modify(|v| *v = v.wrapping_add(1));
}

async fn remove_server_routes(s: &AppState, server_id: &str) -> anyhow::Result<()> {
    let _guard = s.refresh.lock().await;
    prune_server_routes(&s.db, server_id).await
}

async fn prune_server_routes(db: &SqlitePool, server_id: &str) -> anyhow::Result<()> {
    let mut tx = db.begin().await?;
    let rows =
        sqlx::query("SELECT router_name, service_name FROM provider_routes WHERE server_id=?")
            .bind(server_id)
            .fetch_all(&mut *tx)
            .await?;
    if !rows.is_empty()
        && let Some((raw,)) =
            sqlx::query_as::<_, (String,)>("SELECT config FROM provider_snapshot WHERE id=1")
                .fetch_optional(&mut *tx)
                .await?
    {
        let mut doc: serde_json::Value = serde_json::from_str(&raw)?;
        for row in &rows {
            let router: String = row.get("router_name");
            let service: String = row.get("service_name");
            doc["http"]["routers"]
                .as_object_mut()
                .context("snapshot routers missing")?
                .remove(&router);
            doc["http"]["services"]
                .as_object_mut()
                .context("snapshot services missing")?
                .remove(&service);
        }
        sqlx::query("UPDATE provider_snapshot SET config=?, refreshed_at=? WHERE id=1")
            .bind(doc.to_string())
            .bind(now())
            .execute(&mut *tx)
            .await?;
    }
    sqlx::query("DELETE FROM provider_routes WHERE server_id=?")
        .bind(server_id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(())
}

async fn reconcile_snapshot(s: &AppState) -> anyhow::Result<()> {
    let _guard = s.refresh.lock().await;
    let active = servers(&s.db)
        .await?
        .into_iter()
        .filter(|server| server.enabled)
        .map(|server| server.id)
        .collect::<std::collections::HashSet<_>>();
    let rows = sqlx::query("SELECT DISTINCT server_id FROM provider_routes")
        .fetch_all(&s.db)
        .await?;
    for row in rows {
        let id: String = row.get("server_id");
        if !active.contains(&id) {
            prune_server_routes(&s.db, &id).await?;
        }
    }
    Ok(())
}
async fn refresh(s: &AppState) -> anyhow::Result<()> {
    let _lock = s.refresh.lock().await;
    let srv = servers(&s.db).await?;
    let containers = docker::aggregate(srv.clone()).await?;
    let previous = auth_selection(&s.db).await?;
    let api_url = traefik::candidate_url(&containers, s.settings.traefik_api_url.as_deref());
    let api_result = match api_url {
        Ok(url) => {
            traefik::fetch(
                &url,
                s.settings.traefik_api_username.as_deref(),
                s.settings.traefik_api_password.as_deref(),
            )
            .await
        }
        Err(error) => Err(error),
    };
    let view = match api_result {
        Ok(raw) => {
            let t = now();
            let view = match traefik::normalize(&raw, t) {
                Ok(view) => view,
                Err(error) => {
                    *s.api_error.write().await = Some(error.to_string());
                    let current = traefik::load_runtime(&s.db).await?;
                    let _ = traefik::save_runtime(
                        &s.db,
                        &traefik::RuntimeView {
                            routers: current.routers,
                            services: current.services,
                            fetched_at: current.fetched_at,
                            api_reachable: false,
                        },
                    )
                    .await;
                    anyhow::bail!(
                        "Traefik API payload unavailable; retaining last published document: {error}"
                    )
                }
            };
            let runtime = traefik::runtime_routers(&raw, t);
            traefik::save_runtime(&s.db, &runtime).await?;
            traefik::save(&s.db, &view).await?;
            *s.api_error.write().await = None;
            view
        }
        Err(error) => {
            *s.api_error.write().await = Some(error.to_string());
            let current = traefik::load_runtime(&s.db).await?;
            let _ = traefik::save_runtime(
                &s.db,
                &traefik::RuntimeView {
                    routers: current.routers,
                    services: current.services,
                    fetched_at: current.fetched_at,
                    api_reachable: false,
                },
            )
            .await;
            // A fresh installation with no configured auth and no auth provider
            // can still serve routes without requiring Traefik's optional API.
            // Once auth has been active, an API outage must never change its
            // provider or remove its middleware reference.
            if previous.protected || traefik::latest(&s.db).await?.is_some() {
                anyhow::bail!(
                    "Traefik API unavailable; retaining last published document: {error}"
                );
            }
            let candidates = auth::candidates(&containers, &srv)?;
            if candidates.iter().any(|c| c.eligible) {
                anyhow::bail!(
                    "Traefik API unavailable with detected auth containers; cannot determine file override: {error}"
                );
            }
            traefik::FileView::default()
        }
    };
    // An unreadable API cannot establish that a file middleware disappeared.
    // Preserve the last published document, including its auth references.
    let decision = auth::choose(&containers, &srv, Some(&view), &previous)?;
    let config = provider::generate(
        &containers,
        &srv,
        &s.settings.domain,
        provider::AuthOutput {
            reference: decision.reference.as_deref(),
            required: decision.required,
            containers: decision
                .candidates
                .iter()
                .map(|c| (c.server_id.clone(), c.container_id.clone()))
                .collect(),
            middlewares: decision.middlewares.clone(),
            services: decision.services.clone(),
        },
    )?;
    let warnings =
        traefik::route_warnings(&config, s.api_error.read().await.is_none().then_some(&view));
    let mut tx = s.db.begin().await?;
    sqlx::query("INSERT INTO provider_snapshot(id,config,refreshed_at) VALUES(1,?,?) ON CONFLICT(id) DO UPDATE SET config=excluded.config,refreshed_at=excluded.refreshed_at")
        .bind(config.to_string()).bind(now()).execute(&mut *tx).await?;
    sqlx::query("DELETE FROM provider_routes")
        .execute(&mut *tx)
        .await?;
    for (name, service, server_id, container_id) in provider::route_owners(&containers, &config) {
        let rule = config["http"]["routers"][&name]["rule"]
            .as_str()
            .unwrap_or("");
        let upstream_fingerprint = config["http"]["services"]
            .get(&service)
            .and_then(traefik::service_fingerprint)
            .unwrap_or_default();
        sqlx::query(
            "INSERT INTO provider_routes(router_name,service_name,server_id,container_id,rule,upstream_fingerprint) VALUES(?,?,?,?,?,?)",
        )
        .bind(&name)
        .bind(&service)
        .bind(&server_id)
        .bind(&container_id)
        .bind(rule)
        .bind(&upstream_fingerprint)
        .execute(&mut *tx)
        .await?;
    }
    sqlx::query("INSERT INTO auth_selection(id,provider,container_id,server_id,network,dns_name,protected,file_owned,last_reference) VALUES(1,?,?,?,?,?,?,?,?) ON CONFLICT(id) DO UPDATE SET provider=excluded.provider,container_id=excluded.container_id,server_id=excluded.server_id,network=excluded.network,dns_name=excluded.dns_name,protected=excluded.protected,file_owned=excluded.file_owned,last_reference=excluded.last_reference")
        .bind(&decision.selection.provider).bind(&decision.selection.container_id).bind(&decision.selection.server_id).bind(&decision.selection.network).bind(&decision.selection.dns_name).bind(decision.selection.protected).bind(decision.selection.file_owned).bind(&decision.selection.last_reference).execute(&mut *tx).await?;
    tx.commit().await?;
    *s.auth_status.write().await = Some(AuthStatus {
        reference: decision.reference,
        note: decision.note,
        candidates: decision.candidates,
        protected: decision.selection.protected,
        api_reachable: s.api_error.read().await.is_none(),
        credentials_configured: s.settings.traefik_api_username.is_some(),
    });
    *s.route_warnings.write().await = warnings;
    *s.last_error.write().await = None;
    Ok(())
}
async fn provider_config(
    State(s): State<AppState>,
    headers: HeaderMap,
) -> ApiResult<Json<serde_json::Value>> {
    if let Some(expected) = &s.settings.provider_token {
        use subtle::ConstantTimeEq;
        let provided = headers
            .get(header::AUTHORIZATION)
            .and_then(|h| h.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "))
            .unwrap_or("");
        if provided.as_bytes().ct_eq(expected.as_bytes()).unwrap_u8() != 1 {
            return Err(ApiError(
                StatusCode::UNAUTHORIZED,
                "invalid provider token".into(),
            ));
        }
    }
    let row: Option<(String,)> = sqlx::query_as("SELECT config FROM provider_snapshot WHERE id=1")
        .fetch_optional(&s.db)
        .await
        .map_err(internal)?;
    let snapshot = row.ok_or_else(|| {
        ApiError(
            StatusCode::SERVICE_UNAVAILABLE,
            "no successful snapshot yet".into(),
        )
    })?;
    Ok(Json(serde_json::from_str(&snapshot.0).map_err(internal)?))
}
async fn provider_preview(
    State(s): State<AppState>,
    headers: HeaderMap,
) -> ApiResult<Json<serde_json::Value>> {
    require_login(&s, &headers).await?;
    let row: Option<(String,)> = sqlx::query_as("SELECT config FROM provider_snapshot WHERE id=1")
        .fetch_optional(&s.db)
        .await
        .map_err(internal)?;
    let raw = row.ok_or_else(|| {
        ApiError(
            StatusCode::SERVICE_UNAVAILABLE,
            "no successful snapshot yet".into(),
        )
    })?;
    Ok(Json(serde_json::from_str(&raw.0).map_err(internal)?))
}
#[derive(Serialize)]
struct Status {
    refreshed_at: Option<i64>,
    auth: Option<AuthStatus>,
    api_error: Option<String>,
    last_error: Option<String>,
    route_warnings: Vec<String>,
}
#[derive(Clone, Serialize)]
struct AuthStatus {
    reference: Option<String>,
    note: String,
    candidates: Vec<auth::Candidate>,
    protected: bool,
    api_reachable: bool,
    credentials_configured: bool,
}
async fn auth_selection(db: &SqlitePool) -> anyhow::Result<auth::Selection> {
    let row=sqlx::query("SELECT provider,container_id,server_id,network,dns_name,protected,file_owned,last_reference FROM auth_selection WHERE id=1").fetch_optional(db).await?;
    Ok(row
        .map(|r| auth::Selection {
            provider: r.get("provider"),
            container_id: r.get("container_id"),
            server_id: r.get("server_id"),
            network: r.get("network"),
            dns_name: r.get("dns_name"),
            protected: r.get::<i64, _>("protected") != 0,
            file_owned: r.get::<i64, _>("file_owned") != 0,
            last_reference: r.get("last_reference"),
        })
        .unwrap_or_default())
}
async fn provider_status(State(s): State<AppState>, headers: HeaderMap) -> ApiResult<Json<Status>> {
    require_login(&s, &headers).await?;
    let row: Option<(i64,)> =
        sqlx::query_as("SELECT refreshed_at FROM provider_snapshot WHERE id=1")
            .fetch_optional(&s.db)
            .await
            .map_err(internal)?;
    let api_error = s.api_error.read().await.clone();
    let mut auth = s.auth_status.read().await.clone();
    if let Some(status) = auth.as_mut() {
        status.api_reachable = api_error.is_none();
    }
    if auth.is_none() {
        let selection = auth_selection(&s.db).await.map_err(internal)?;
        auth = Some(AuthStatus {
            reference: selection.last_reference,
            note: "Waiting for first refresh; saved auth intent is retained".into(),
            candidates: vec![],
            protected: selection.protected,
            api_reachable: api_error.is_none(),
            credentials_configured: s.settings.traefik_api_username.is_some(),
        });
    }
    Ok(Json(Status {
        refreshed_at: row.map(|r| r.0),
        auth,
        api_error,
        last_error: s.last_error.read().await.clone(),
        route_warnings: s.route_warnings.read().await.clone(),
    }))
}

async fn reset_auth(State(s): State<AppState>, headers: HeaderMap) -> ApiResult<StatusCode> {
    require_login(&s, &headers).await?;
    require_same_origin(&headers)?;
    // Reset the selected identity without dropping the already-established
    // protection requirement. A stopped auth service must never become open.
    sqlx::query("UPDATE auth_selection SET provider=NULL,container_id=NULL,server_id=NULL,network=NULL,dns_name=NULL,file_owned=0,last_reference=NULL WHERE id=1")
        .execute(&s.db).await.map_err(internal)?;
    trigger(&s);
    Ok(StatusCode::NO_CONTENT)
}
async fn file_history(
    State(s): State<AppState>,
    headers: HeaderMap,
) -> ApiResult<Json<Vec<traefik::SnapshotInfo>>> {
    require_login(&s, &headers).await?;
    Ok(Json(traefik::history(&s.db).await.map_err(internal)?))
}
#[derive(Deserialize)]
struct ExportQuery {
    id: Option<i64>,
}
async fn export_files(
    State(s): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<ExportQuery>,
) -> ApiResult<Response> {
    require_login(&s, &headers).await?;
    let content = traefik::export(&s.db, query.id)
        .await
        .map_err(|e| ApiError(StatusCode::NOT_FOUND, e.to_string()))?;
    Ok((
        [
            (header::CONTENT_TYPE, "application/yaml"),
            (
                header::CONTENT_DISPOSITION,
                "attachment; filename=traefik-file-snapshot.yml",
            ),
        ],
        content,
    )
        .into_response())
}
#[derive(Deserialize)]
struct SnippetQuery {
    server_id: String,
    container_id: String,
}
async fn auth_snippet(
    State(s): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<SnippetQuery>,
) -> ApiResult<Json<serde_json::Value>> {
    require_login(&s, &headers).await?;
    let srv = servers(&s.db).await.map_err(internal)?;
    let all = docker::aggregate(srv.clone()).await.map_err(internal)?;
    let candidates = auth::candidates(&all, &srv).map_err(bad)?;
    let c = candidates
        .iter()
        .find(|v| v.server_id == q.server_id && v.container_id == q.container_id && !v.eligible)
        .ok_or_else(|| {
            ApiError(
                StatusCode::NOT_FOUND,
                "remote auth container not found".into(),
            )
        })?;
    let names = traefik::latest(&s.db)
        .await
        .map_err(internal)?
        .map(|v| traefik::file_resources(&v))
        .unwrap_or_default();
    if names.contains("middlewares:default-forward-auth@file") {
        return Err(bad("default-forward-auth@file already exists"));
    }
    let (yaml, warnings) = auth::snippet(c, &names).map_err(bad)?;
    Ok(Json(serde_json::json!({"yaml":yaml,"warnings":warnings})))
}

async fn protected_web(State(s): State<AppState>, request: Request) -> Response {
    let path = request.uri().path();
    if path.starts_with("/snippets/")
        || path == "/login"
        || path == "/login.html"
        || path == "/app.css"
        || path == "/app.js"
        || path == "/app_bg.wasm"
    {
        return match ServeDir::new("web/dist").oneshot(request).await {
            Ok(r) => r.into_response(),
            Err(_) => StatusCode::NOT_FOUND.into_response(),
        };
    }
    if require_login(&s, request.headers()).await.is_err() {
        return axum::response::Redirect::to("/login.html").into_response();
    }
    match ServeDir::new("web/dist").oneshot(request).await {
        Ok(r) => r.into_response(),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn import_legacy(db: &SqlitePool) -> anyhow::Result<()> {
    // Prisma uses the same SQLite file; migration adds new tables without touching original rows.
    let mut tx = db.begin().await?;
    let completed: Option<(String,)> =
        sqlx::query_as("SELECT name FROM migration_state WHERE name='prisma_servers'")
            .fetch_optional(&mut *tx)
            .await?;
    if completed.is_some() {
        return Ok(());
    }
    let found: Option<(String,)> = sqlx::query_as(
        "SELECT name FROM sqlite_master WHERE type='table' AND name='containerserver'",
    )
    .fetch_optional(&mut *tx)
    .await?;
    if found.is_some() {
        let old = sqlx::query("SELECT id,name,host,port,enable FROM containerserver")
            .fetch_all(&mut *tx)
            .await?;
        for row in old {
            let endpoint: String = row.get("host");
            let port: i64 = row.get("port");
            let address = if endpoint.contains("://") {
                endpoint.clone()
            } else {
                format!("http://{endpoint}")
            };
            let parsed = url::Url::parse(&address)
                .with_context(|| format!("invalid legacy Docker API endpoint: {endpoint}"))?;
            anyhow::ensure!(
                parsed.scheme() == "http",
                "legacy Docker endpoint must use http: {endpoint}"
            );
            let raw_host = parsed.host_str().context("legacy host missing")?;
            let api = format!(
                "http://{}:{}",
                if raw_host.contains(':') {
                    format!("[{raw_host}]")
                } else {
                    raw_host.into()
                },
                parsed.port().unwrap_or(port as u16)
            );
            sqlx::query("INSERT OR IGNORE INTO docker_servers(id,name,transport,endpoint,upstream_host,webui_host,enabled) VALUES(?,?,'http',?,?,?,?)")
                .bind(row.get::<String,_>("id")).bind(row.get::<String,_>("name")).bind(api).bind(raw_host).bind(raw_host)
                .bind(row.get::<i64,_>("enable")).execute(&mut *tx).await?;
        }
    }
    sqlx::query("INSERT INTO migration_state(name,completed_at) VALUES('prisma_servers',?)")
        .bind(now())
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(())
}

async fn bootstrap(db: &SqlitePool) -> anyhow::Result<()> {
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users")
        .fetch_one(db)
        .await?;
    if count != 0 {
        return Ok(());
    }
    let password=env::var("ADMIN_BOOTSTRAP_PASSWORD").context("ADMIN_BOOTSTRAP_PASSWORD required on first start (legacy plaintext accounts cannot be imported)")?;
    anyhow::ensure!(
        password.len() >= 12,
        "ADMIN_BOOTSTRAP_PASSWORD must have at least 12 characters"
    );
    let salt = SaltString::generate(&mut OsRng);
    let hashed = Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map_err(|e| anyhow::anyhow!(e.to_string()))?
        .to_string();
    sqlx::query("INSERT INTO users(id,username,password_hash) VALUES(?,'admin',?)")
        .bind(uuid::Uuid::new_v4().to_string())
        .bind(hashed)
        .execute(db)
        .await?;
    Ok(())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();
    let cfg = settings()?;
    let options: SqliteConnectOptions = cfg
        .database_url
        .parse::<SqliteConnectOptions>()?
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        .busy_timeout(Duration::from_secs(5));
    let db = SqlitePoolOptions::new()
        .max_connections(5)
        .connect_with(options)
        .await?;
    sqlx::migrate!().run(&db).await?;
    import_legacy(&db).await?;
    bootstrap(&db).await?;
    let (tx, mut rx) = watch::channel(0u64);
    let state = AppState {
        db,
        settings: cfg,
        refresh: Arc::new(Mutex::new(())),
        trigger: tx,
        last_error: Arc::new(RwLock::new(None)),
        api_error: Arc::new(RwLock::new(None)),
        auth_status: Arc::new(RwLock::new(None)),
        route_warnings: Arc::new(RwLock::new(Vec::new())),
    };
    reconcile_snapshot(&state).await?;
    let worker = state.clone();
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(5));
        loop {
            tokio::select! {_ = tick.tick()=>{},changed=rx.changed()=>{if changed.is_err(){break}}}
            if let Err(error) = refresh(&worker).await {
                tracing::warn!(%error,"refresh failed; retaining last good provider snapshot");
                *worker.last_error.write().await = Some(error.to_string());
            }
        }
    });
    let app = Router::new()
        .route("/api/ping", get(|| async { "pong" }))
        .route("/api/v1/traefikconfig", get(provider_config))
        .route("/api/v1/provider/preview", get(provider_preview))
        .route("/api/v1/provider/status", get(provider_status))
        .route("/api/v1/provider/auth/reset", post(reset_auth))
        .route("/api/v1/provider/files", get(file_history))
        .route("/api/v1/provider/files/export", get(export_files))
        .route("/api/v1/provider/auth/snippet", get(auth_snippet))
        .route("/auth/login", post(login))
        .route("/auth/logout", post(logout))
        .route("/api/v1/servers", get(list_servers).post(create_server))
        .route("/api/v1/servers/test", post(test_server))
        .route("/api/v1/servers/discover", post(discover_server))
        .route(
            "/api/v1/servers/{id}",
            axum::routing::put(update_server).delete(delete_server),
        )
        .route("/api/v1/containers", get(list_containers))
        .route(
            "/api/v1/containers/{server_id}/{id}",
            get(inspect_container),
        )
        .route(
            "/api/v1/containers/{server_id}/{id}/actions",
            post(container_action),
        )
        .route(
            "/api/v1/containers/{server_id}/{id}/webui-url",
            put(update_webui_url),
        )
        .fallback(protected_web)
        .layer(TraceLayer::new_for_http())
        .with_state(state);
    let addr: SocketAddr = env::var("LISTEN_ADDR")
        .unwrap_or_else(|_| "0.0.0.0:4000".into())
        .parse()?;
    tracing::info!(%addr,"listening");
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

#[cfg(test)]
mod backend_tests {
    use super::*;
    use serde_json::json;
    use traefik_http_api::Port;

    async fn memory_db() -> SqlitePool {
        let db = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::migrate!().run(&db).await.unwrap();
        db
    }

    fn test_state(db: SqlitePool) -> AppState {
        AppState {
            db,
            settings: Settings {
                domain: "example.com".into(),
                traefik_api_url: None,
                traefik_api_username: None,
                traefik_api_password: None,
                provider_token: None,
                database_url: ":memory:".into(),
            },
            refresh: Arc::new(Mutex::new(())),
            trigger: watch::channel(0).0,
            last_error: Arc::new(RwLock::new(None)),
            api_error: Arc::new(RwLock::new(None)),
            auth_status: Arc::new(RwLock::new(None)),
            route_warnings: Arc::new(RwLock::new(Vec::new())),
        }
    }

    fn test_container() -> Container {
        Container {
            id: "cid".into(),
            name: "app".into(),
            image: "app".into(),
            state: "running".into(),
            health: None,
            server_id: "sid".into(),
            server_name: "server".into(),
            labels: HashMap::from([("traefik.enable".into(), "true".into())]),
            ports: vec![Port {
                private_port: 80,
                public_port: Some(8080),
                host_ip: Some("0.0.0.0".into()),
                protocol: "tcp".into(),
            }],
            networks: vec![],
            webui_url: None,
            webui_url_default: None,
            webui_url_override: None,
            routing: None,
        }
    }

    fn snapshot_config() -> Option<serde_json::Value> {
        Some(json!({
            "http": {
                "routers": {
                    "App": {
                        "rule": "Host(`app.example.com`) && PathPrefix(`/`)",
                        "service": "App",
                        "entryPoints": ["https"]
                    }
                },
                "services": {
                    "App": {
                        "loadBalancer": {
                            "servers": [{"url": "http://10.0.0.2:8080"}]
                        }
                    }
                }
            }
        }))
    }

    fn route_info(router_name: &str, snapshot: &Option<serde_json::Value>) -> RouteInfo {
        let snapshot = snapshot.as_ref().unwrap();
        let rule = snapshot["http"]["routers"][router_name]["rule"]
            .as_str()
            .unwrap_or("")
            .to_owned();
        let service_name = snapshot["http"]["routers"][router_name]["service"]
            .as_str()
            .unwrap_or("")
            .to_owned();
        let upstream_fingerprint = snapshot["http"]["services"]
            .get(&service_name)
            .and_then(traefik::service_fingerprint)
            .unwrap_or_default();
        RouteInfo {
            router_name: router_name.to_owned(),
            service_name,
            rule,
            upstream_fingerprint,
        }
    }

    fn routes_for(c: &Container, info: RouteInfo) -> HashMap<(String, String), RouteInfo> {
        HashMap::from([((c.server_id.clone(), c.id.clone()), info)])
    }

    fn enabled_runtime(t: i64, snapshot: &Option<serde_json::Value>) -> traefik::RuntimeView {
        let snapshot = snapshot.as_ref().unwrap();
        let mut services = HashMap::new();
        for (name, value) in snapshot["http"]["services"]
            .as_object()
            .unwrap_or(&Default::default())
        {
            if let Some(fp) = traefik::service_fingerprint(value) {
                services.insert(name.clone(), fp);
            }
        }
        traefik::RuntimeView {
            routers: HashMap::from([(
                "App".into(),
                traefik::RuntimeRouter {
                    status: "enabled".into(),
                    rule: "Host(`app.example.com`) && PathPrefix(`/`)".into(),
                    service: "App".into(),
                    forward_auth: Some(false),
                    tls: Some(true),
                },
            )]),
            services,
            fetched_at: Some(t),
            api_reachable: true,
        }
    }

    fn input() -> ServerInput {
        ServerInput {
            name: "server".into(),
            transport: "http".into(),
            endpoint: "http://docker.example:2375".into(),
            tls_key: None,
            tls_cert: None,
            tls_ca: None,
            upstream_host: "docker.example".into(),
            webui_host: "browser.example".into(),
            prefer_network: false,
            docker_network: None,
            webui_same_as_server: false,
            enabled: true,
        }
    }

    #[test]
    fn validation_allows_blank_browser_host_when_inherited() {
        let mut value = input();
        value.webui_host.clear();
        value.webui_same_as_server = true;
        assert!(validate(&value).is_ok());
        value.webui_same_as_server = false;
        assert!(validate(&value).is_err());
    }

    #[test]
    fn validation_requires_selected_network_when_preferred() {
        let mut value = input();
        value.prefer_network = true;
        assert!(validate(&value).is_err());
        value.docker_network = Some("apps".into());
        assert!(validate(&value).is_ok());
    }

    #[test]
    fn connection_validation_does_not_require_routing_fields() {
        let mut value = input();
        value.name.clear();
        value.upstream_host.clear();
        value.webui_host.clear();
        assert!(validate_connection(&value).is_ok());
    }

    #[tokio::test]
    async fn migration_defaults_and_server_reload_persist_preferences() {
        let db = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::migrate!().run(&db).await.unwrap();
        sqlx::query("INSERT INTO docker_servers(id,name,transport,endpoint,upstream_host,webui_host,enabled) VALUES('old','old','http','http://old:2375','old','old',1)")
            .execute(&db).await.unwrap();
        let old = servers(&db).await.unwrap().pop().unwrap();
        assert!(!old.prefer_network);
        assert_eq!(old.docker_network, None);
        assert!(!old.webui_same_as_server);

        sqlx::query("UPDATE docker_servers SET prefer_network=1,docker_network='apps',webui_same_as_server=1 WHERE id='old'")
            .execute(&db).await.unwrap();
        let updated = servers(&db).await.unwrap().pop().unwrap();
        assert!(updated.prefer_network);
        assert_eq!(updated.docker_network.as_deref(), Some("apps"));
        assert!(updated.webui_same_as_server);
        assert_eq!(updated.browser_host(), "old");

        sqlx::query("INSERT INTO docker_servers(id,name,transport,endpoint,upstream_host,webui_host,prefer_network,docker_network,webui_same_as_server,enabled) VALUES('new','new','http','http://new:2375','new','saved-browser',1,'frontend',1,1)")
            .execute(&db).await.unwrap();
        let created = servers(&db)
            .await
            .unwrap()
            .into_iter()
            .find(|server| server.id == "new")
            .unwrap();
        assert_eq!(created.docker_network.as_deref(), Some("frontend"));
        assert_eq!(created.browser_host(), "new");
        sqlx::query("DELETE FROM docker_servers WHERE id='new'")
            .execute(&db)
            .await
            .unwrap();
        assert!(
            !servers(&db)
                .await
                .unwrap()
                .iter()
                .any(|server| server.id == "new")
        );
    }

    #[test]
    fn webui_url_override_accepts_http_https_query_fragment() {
        assert!(validate_webui_url_override("http://host/path?q=1#frag").is_ok());
        assert!(validate_webui_url_override("https://host/").is_ok());
    }

    #[test]
    fn webui_url_override_rejects_unsafe_schemes_credentials_and_templates() {
        assert!(validate_webui_url_override("ftp://host/").is_err());
        assert!(validate_webui_url_override("http://user:pass@host/").is_err());
        // Literal [IP] is rejected by URL parsing, not by a blanket bracket ban.
        assert!(validate_webui_url_override("http://[IP]:[PORT:80]/").is_err());
        assert!(validate_webui_url_override("http://host/%5BPATH%5D").is_ok());
    }

    #[test]
    fn webui_url_override_requires_explicit_scheme_separator() {
        assert!(validate_webui_url_override("https:example.com").is_err());
        assert!(validate_webui_url_override("http:foo").is_err());
        assert!(validate_webui_url_override("http://foo").is_ok());
        assert!(validate_webui_url_override("HTTP://FOO/path").is_ok());
    }

    #[test]
    fn webui_url_override_preserves_trailing_slash_query_and_fragment() {
        let slash = validate_webui_url_override("http://host/path/").unwrap();
        assert_eq!(slash, "http://host/path/");
        let query = validate_webui_url_override("http://host/path?q=1&r=2").unwrap();
        assert_eq!(query, "http://host/path?q=1&r=2");
        let fragment = validate_webui_url_override("http://host/path#section").unwrap();
        assert_eq!(fragment, "http://host/path#section");
        let all = validate_webui_url_override("http://host/path/?q=1#section").unwrap();
        assert_eq!(all, "http://host/path/?q=1#section");
    }

    #[test]
    fn webui_url_override_accepts_ipv6() {
        assert_eq!(
            validate_webui_url_override("http://[::1]:8080/").unwrap(),
            "http://[::1]:8080/"
        );
        assert_eq!(
            validate_webui_url_override("https://[2001:db8::1]/ui").unwrap(),
            "https://[2001:db8::1]/ui"
        );
    }

    #[tokio::test]
    async fn webui_url_override_persists_and_clears() {
        let db = memory_db().await;
        set_container_webui_url(&db, "s", "c", Some("https://override.example/"))
            .await
            .unwrap();
        let overrides = container_webui_urls(&db).await.unwrap();
        assert_eq!(
            overrides.get(&("s".into(), "c".into())).cloned().flatten(),
            Some("https://override.example/".into())
        );
        set_container_webui_url(&db, "s", "c", None).await.unwrap();
        let overrides = container_webui_urls(&db).await.unwrap();
        assert!(!overrides.contains_key(&("s".into(), "c".into())));
    }

    #[tokio::test]
    async fn webui_url_override_isolated_by_server_and_container() {
        let db = memory_db().await;
        set_container_webui_url(&db, "s1", "c1", Some("https://a.example/"))
            .await
            .unwrap();
        set_container_webui_url(&db, "s1", "c2", Some("https://b.example/"))
            .await
            .unwrap();
        set_container_webui_url(&db, "s2", "c1", Some("https://c.example/"))
            .await
            .unwrap();
        let overrides = container_webui_urls(&db).await.unwrap();
        assert_eq!(
            overrides
                .get(&("s1".into(), "c1".into()))
                .cloned()
                .flatten(),
            Some("https://a.example/".into())
        );
        assert_eq!(
            overrides
                .get(&("s1".into(), "c2".into()))
                .cloned()
                .flatten(),
            Some("https://b.example/".into())
        );
        assert_eq!(
            overrides
                .get(&("s2".into(), "c1".into()))
                .cloned()
                .flatten(),
            Some("https://c.example/".into())
        );
    }

    #[tokio::test]
    async fn deleting_server_removes_webui_url_overrides() {
        let db = memory_db().await;
        sqlx::query("INSERT INTO docker_servers(id,name,transport,endpoint,upstream_host,webui_host,enabled) VALUES('s','s','http','http://s:2375','s','s',1)")
            .execute(&db).await.unwrap();
        set_container_webui_url(&db, "s", "c", Some("https://x.example/"))
            .await
            .unwrap();
        sqlx::query("DELETE FROM docker_servers WHERE id='s'")
            .execute(&db)
            .await
            .unwrap();
        sqlx::query("DELETE FROM container_webui_urls WHERE server_id='s'")
            .execute(&db)
            .await
            .unwrap();
        let overrides = container_webui_urls(&db).await.unwrap();
        assert!(overrides.is_empty());
    }

    #[test]
    fn host_extracted_from_traefik_rule() {
        assert_eq!(
            host_from_rule("Host(`app.example.com`)"),
            Some("app.example.com".into())
        );
        assert_eq!(
            host_from_rule("Host(`app.example.com`) && PathPrefix(`/`)").as_deref(),
            Some("app.example.com")
        );
        assert_eq!(host_from_rule("PathPrefix(`/`"), None);
    }

    #[test]
    fn routing_unavailable_without_snapshot() {
        let c = test_container();
        let r = container_routing(
            &c,
            &None,
            &HashMap::new(),
            &HashMap::new(),
            &traefik::RuntimeView::default(),
            100,
        );
        assert_eq!(r.state, "unavailable");
        assert!(r.reason.as_deref().unwrap().contains("snapshot"));
    }

    #[test]
    fn routing_stopped_not_routed_without_snapshot() {
        let mut c = test_container();
        c.state = "exited".into();
        let r = container_routing(
            &c,
            &None,
            &HashMap::new(),
            &HashMap::new(),
            &traefik::RuntimeView::default(),
            100,
        );
        assert_eq!(r.state, "not_routed");
        assert_eq!(r.reason.as_deref(), Some("Container stopped"));
    }

    #[test]
    fn routing_stopped_with_old_snapshot_is_unconfirmed_stale() {
        let mut c = test_container();
        c.state = "exited".into();
        let snapshot = snapshot_config();
        let info = route_info("App", &snapshot);
        let routes = routes_for(&c, info);
        let runtime = enabled_runtime(100, &snapshot);
        let r = container_routing(&c, &snapshot, &routes, &HashMap::new(), &runtime, 100);
        assert_eq!(r.state, "unconfirmed");
        assert!(r.reason.as_deref().unwrap().contains("stopped"));
        assert_eq!(r.hostname.as_deref(), Some("app.example.com"));
    }

    #[test]
    fn routing_not_routed_when_disabled_or_not_enabled() {
        let mut c = test_container();
        c.labels
            .insert("traefik.http.disable".into(), "true".into());
        let snapshot = snapshot_config();
        let info = route_info("App", &snapshot);
        let routes = routes_for(&c, info);
        let runtime = enabled_runtime(100, &snapshot);
        let r = container_routing(&c, &snapshot, &routes, &HashMap::new(), &runtime, 100);
        assert_eq!(r.state, "not_routed");
        assert!(r.reason.as_deref().unwrap().contains("disabled"));

        c.labels.clear();
        let info = route_info("App", &snapshot);
        let routes = routes_for(&c, info);
        let r = container_routing(&c, &snapshot, &routes, &HashMap::new(), &runtime, 100);
        assert_eq!(r.state, "not_routed");
        assert!(r.reason.as_deref().unwrap().contains("not enabled"));
    }

    #[test]
    fn routing_not_routed_without_generated_route() {
        let mut c = test_container();
        c.ports.clear();
        let snapshot = Some(json!({
            "http": {
                "routers": {},
                "services": {}
            }
        }));
        let r = container_routing(
            &c,
            &snapshot,
            &HashMap::new(),
            &HashMap::new(),
            &enabled_runtime(100, &snapshot_config()),
            100,
        );
        assert_eq!(r.state, "not_routed");
        assert!(r.reason.as_deref().unwrap().contains("not generated"));
    }

    #[test]
    fn routing_routed_when_snapshot_and_runtime_match() {
        let c = test_container();
        let snapshot = snapshot_config();
        let info = route_info("App", &snapshot);
        let routes = routes_for(&c, info);
        let runtime = enabled_runtime(100, &snapshot);
        let r = container_routing(&c, &snapshot, &routes, &HashMap::new(), &runtime, 100);
        assert_eq!(r.state, "routed");
        assert_eq!(r.hostname.as_deref(), Some("app.example.com"));
        assert_eq!(r.checked_at, Some(100));
        assert!(r.reason.is_none());
        assert_eq!(r.forward_auth, Some(false));
        assert_eq!(r.tls, Some(true));
    }

    #[test]
    fn routing_routed_propagates_auth_true_and_unknown() {
        let c = test_container();
        let snapshot = snapshot_config();
        let info = route_info("App", &snapshot);
        let routes = routes_for(&c, info);
        for (forward_auth, tls) in [(Some(true), Some(false)), (None, None)] {
            let mut runtime = enabled_runtime(100, &snapshot);
            let router = runtime.routers.get_mut("App").unwrap();
            router.forward_auth = forward_auth;
            router.tls = tls;
            let r = container_routing(&c, &snapshot, &routes, &HashMap::new(), &runtime, 100);
            assert_eq!(r.state, "routed");
            assert_eq!(r.forward_auth, forward_auth);
            assert_eq!(r.tls, tls);
        }
    }

    #[test]
    fn routing_unconfirmed_when_api_unreachable() {
        let c = test_container();
        let snapshot = snapshot_config();
        let info = route_info("App", &snapshot);
        let routes = routes_for(&c, info);
        let runtime = traefik::RuntimeView {
            api_reachable: false,
            ..Default::default()
        };
        let r = container_routing(&c, &snapshot, &routes, &HashMap::new(), &runtime, 100);
        assert_eq!(r.state, "unconfirmed");
        assert!(r.reason.as_deref().unwrap().contains("unreachable"));
        assert_eq!(r.forward_auth, None);
        assert_eq!(r.tls, None);
    }

    #[test]
    fn routing_unconfirmed_when_runtime_data_is_stale() {
        let c = test_container();
        let snapshot = snapshot_config();
        let info = route_info("App", &snapshot);
        let routes = routes_for(&c, info);
        let mut runtime = enabled_runtime(50, &snapshot);
        runtime.fetched_at = Some(50);
        let r = container_routing(&c, &snapshot, &routes, &HashMap::new(), &runtime, 100);
        assert_eq!(r.state, "unconfirmed");
        assert!(r.reason.as_deref().unwrap().contains("stale"));
        assert_eq!(r.forward_auth, None);
        assert_eq!(r.tls, None);
    }

    #[test]
    fn routing_unconfirmed_when_runtime_router_missing_or_disabled() {
        let c = test_container();
        let snapshot = snapshot_config();
        let info = route_info("App", &snapshot);
        let routes = routes_for(&c, info);
        let runtime = traefik::RuntimeView {
            routers: HashMap::new(),
            services: HashMap::new(),
            fetched_at: Some(100),
            api_reachable: true,
        };
        let r = container_routing(&c, &snapshot, &routes, &HashMap::new(), &runtime, 100);
        assert_eq!(r.state, "unconfirmed");
        assert!(r.reason.as_deref().unwrap().contains("not yet observed"));

        let runtime = traefik::RuntimeView {
            routers: HashMap::from([(
                "App".into(),
                traefik::RuntimeRouter {
                    status: "disabled".into(),
                    rule: "Host(`app.example.com`) && PathPrefix(`/`)".into(),
                    service: "App".into(),
                    forward_auth: None,
                    tls: None,
                },
            )]),
            services: HashMap::new(),
            fetched_at: Some(100),
            api_reachable: true,
        };
        let r = container_routing(&c, &snapshot, &routes, &HashMap::new(), &runtime, 100);
        assert_eq!(r.state, "unconfirmed");
        assert!(r.reason.as_deref().unwrap().contains("disabled"));
    }

    #[test]
    fn routing_does_not_confirm_changed_router_by_name() {
        let mut c = test_container();
        c.labels.insert("traefik.name".into(), "NewApp".into());
        let snapshot = Some(json!({
            "http": {
                "routers": {
                    "NewApp": {
                        "rule": "Host(`newapp.example.com`)",
                        "service": "NewApp"
                    }
                },
                "services": {
                    "NewApp": {
                        "loadBalancer": {
                            "servers": [{"url": "http://10.0.0.2:8080"}]
                        }
                    }
                }
            }
        }));
        let info = route_info("NewApp", &snapshot);
        let routes = routes_for(&c, info);
        // Runtime still reports the old "App" router as enabled.
        let runtime = enabled_runtime(100, &snapshot_config());
        let r = container_routing(&c, &snapshot, &routes, &HashMap::new(), &runtime, 100);
        assert_eq!(r.state, "unconfirmed");
        assert_eq!(r.hostname.as_deref(), Some("newapp.example.com"));
    }

    #[test]
    fn routing_does_not_confirm_changed_hostname_with_same_router_name() {
        let c = test_container();
        let snapshot = snapshot_config();
        let info = route_info("App", &snapshot);
        let routes = routes_for(&c, info);
        let mut runtime = enabled_runtime(100, &snapshot);
        runtime.routers.insert(
            "App".into(),
            traefik::RuntimeRouter {
                status: "enabled".into(),
                rule: "Host(`oldapp.example.com`)".into(),
                service: "App".into(),
                forward_auth: None,
                tls: None,
            },
        );
        let r = container_routing(&c, &snapshot, &routes, &HashMap::new(), &runtime, 100);
        assert_eq!(r.state, "unconfirmed");
        assert!(
            r.reason
                .as_deref()
                .unwrap()
                .contains("rule does not match generated rule")
        );
        assert_eq!(r.hostname.as_deref(), Some("app.example.com"));
    }

    #[test]
    fn routing_does_not_confirm_changed_service_with_same_router() {
        let c = test_container();
        let snapshot = snapshot_config();
        let info = route_info("App", &snapshot);
        let routes = routes_for(&c, info);
        let mut runtime = enabled_runtime(100, &snapshot);
        runtime.routers.insert(
            "App".into(),
            traefik::RuntimeRouter {
                status: "enabled".into(),
                rule: "Host(`app.example.com`) && PathPrefix(`/`)".into(),
                service: "OtherApp".into(),
                forward_auth: None,
                tls: None,
            },
        );
        let r = container_routing(&c, &snapshot, &routes, &HashMap::new(), &runtime, 100);
        assert_eq!(r.state, "unconfirmed");
        assert!(r.reason.as_deref().unwrap().contains("different service"));
    }

    #[test]
    fn routing_does_not_confirm_changed_upstream_fingerprint() {
        let c = test_container();
        let snapshot = snapshot_config();
        let info = route_info("App", &snapshot);
        let routes = routes_for(&c, info);
        let mut runtime = enabled_runtime(100, &snapshot);
        runtime
            .services
            .insert("App".into(), "http://other:8080".into());
        let r = container_routing(&c, &snapshot, &routes, &HashMap::new(), &runtime, 100);
        assert_eq!(r.state, "unconfirmed");
        assert!(
            r.reason
                .as_deref()
                .unwrap()
                .contains("upstream does not match")
        );
    }

    #[test]
    fn routing_new_container_id_does_not_inherit_old_route() {
        // Same server, same name, but a recreated container gets a new ID.
        let old_c = Container {
            id: "old-cid".into(),
            ..test_container()
        };
        let new_c = Container {
            id: "new-cid".into(),
            ..test_container()
        };
        let snapshot = snapshot_config();
        let info = route_info("App", &snapshot);
        let mut routes = HashMap::new();
        routes.insert((old_c.server_id.clone(), old_c.id.clone()), info.clone());
        let runtime = enabled_runtime(100, &snapshot);
        let r = container_routing(&new_c, &snapshot, &routes, &HashMap::new(), &runtime, 100);
        assert_eq!(r.state, "not_routed");
        assert!(r.reason.as_deref().unwrap().contains("not generated"));
    }

    #[test]
    fn routing_cross_server_same_name_is_isolated_by_container_id() {
        let mut c1 = test_container();
        c1.server_id = "s1".into();
        let mut c2 = test_container();
        c2.server_id = "s2".into();
        let snapshot = snapshot_config();
        let info = route_info("App", &snapshot);
        let mut routes = HashMap::new();
        routes.insert((c1.server_id.clone(), c1.id.clone()), info.clone());
        routes.insert((c2.server_id.clone(), c2.id.clone()), info);
        let runtime = enabled_runtime(100, &snapshot);
        let r1 = container_routing(&c1, &snapshot, &routes, &HashMap::new(), &runtime, 100);
        let r2 = container_routing(&c2, &snapshot, &routes, &HashMap::new(), &runtime, 100);
        assert_eq!(r1.state, "routed");
        assert_eq!(r2.state, "routed");
    }

    #[test]
    fn routing_legacy_null_container_id_is_unconfirmed() {
        let c = test_container();
        let snapshot = snapshot_config();
        let info = route_info("App", &snapshot);
        let mut legacy = HashMap::new();
        legacy.insert((c.server_id.clone(), "App".into()), info);
        let runtime = enabled_runtime(100, &snapshot);
        let r = container_routing(&c, &snapshot, &HashMap::new(), &legacy, &runtime, 100);
        assert_eq!(r.state, "unconfirmed");
        assert!(
            r.reason
                .as_deref()
                .unwrap()
                .contains("ownership pending refresh")
        );
    }

    #[tokio::test]
    async fn runtime_routers_persist_and_load_minimal_status() {
        let db = memory_db().await;
        let snapshot = snapshot_config();
        let view = traefik::RuntimeView {
            routers: HashMap::from([(
                "App".into(),
                traefik::RuntimeRouter {
                    status: "enabled".into(),
                    rule: "Host(`app.example.com`) && PathPrefix(`/`)".into(),
                    service: "App".into(),
                    forward_auth: Some(false),
                    tls: Some(true),
                },
            )]),
            services: traefik::service_fingerprint(
                &snapshot.as_ref().unwrap()["http"]["services"]["App"],
            )
            .into_iter()
            .map(|fp| ("App".into(), fp))
            .collect(),
            fetched_at: Some(123),
            api_reachable: true,
        };
        traefik::save_runtime(&db, &view).await.unwrap();
        let loaded = traefik::load_runtime(&db).await.unwrap();
        assert_eq!(loaded.fetched_at, Some(123));
        assert!(loaded.api_reachable);
        assert_eq!(loaded.routers["App"].status, "enabled");
        assert_eq!(
            loaded.routers["App"].rule,
            "Host(`app.example.com`) && PathPrefix(`/`)"
        );
        assert_eq!(loaded.routers["App"].service, "App");
        assert_eq!(loaded.routers["App"].forward_auth, Some(false));
        assert_eq!(loaded.routers["App"].tls, Some(true));
    }

    #[tokio::test]
    async fn routing_routed_after_runtime_save_and_load_roundtrip() {
        let db = memory_db().await;
        let snapshot = snapshot_config();
        let mut c = test_container();
        c.server_id = "srv".into();
        c.id = "cid".into();
        c.state = "running".into();

        let info = route_info("App", &snapshot);
        sqlx::query(
            "INSERT INTO provider_routes(router_name,service_name,server_id,container_id,rule,upstream_fingerprint) VALUES(?,?,?,?,?,?)"
        )
        .bind(&info.router_name)
        .bind(&info.service_name)
        .bind(&c.server_id)
        .bind(&c.id)
        .bind(&info.rule)
        .bind(&info.upstream_fingerprint)
        .execute(&db)
        .await
        .unwrap();

        let view = enabled_runtime(100, &snapshot);
        traefik::save_runtime(&db, &view).await.unwrap();

        let (by_container, legacy) = persisted_routes(&db).await.unwrap();
        let runtime = traefik::load_runtime(&db).await.unwrap();
        let routing = container_routing(&c, &snapshot, &by_container, &legacy, &runtime, 110);
        assert_eq!(routing.state, "routed");
    }

    #[tokio::test]
    async fn persisted_legacy_null_container_row_is_unconfirmed() {
        let db = memory_db().await;
        let snapshot = snapshot_config();
        let mut c = test_container();
        c.server_id = "srv".into();
        c.id = "cid".into();
        c.name = "app".into();
        c.state = "running".into();

        sqlx::query(
            "INSERT INTO provider_routes(router_name,service_name,server_id,container_id,rule,upstream_fingerprint) VALUES(?,?,?,?,?,?)"
        )
        .bind("App")
        .bind("App")
        .bind(&c.server_id)
        .bind::<Option<String>>(None)
        .bind::<Option<String>>(None)
        .bind::<Option<String>>(None)
        .execute(&db)
        .await
        .unwrap();

        let (by_container, legacy) = persisted_routes(&db).await.unwrap();
        let runtime = enabled_runtime(100, &snapshot);
        let routing = container_routing(&c, &snapshot, &by_container, &legacy, &runtime, 110);
        assert_eq!(routing.state, "unconfirmed");
        assert!(
            routing
                .reason
                .as_deref()
                .unwrap()
                .contains("pending refresh")
        );
    }

    #[tokio::test]
    async fn update_webui_url_handler_requires_login() {
        let db = memory_db().await;
        let state = test_state(db);
        let result = update_webui_url(
            State(state),
            Path(("s".into(), "c".into())),
            HeaderMap::new(),
            Json(WebuiUrlInput { url: None }),
        )
        .await;
        assert!(
            matches!(result, Err(ApiError(StatusCode::UNAUTHORIZED, _))),
            "expected UNAUTHORIZED, got {result:?}"
        );
    }

    #[tokio::test]
    async fn update_webui_url_handler_requires_same_origin() {
        let db = memory_db().await;
        sqlx::query("INSERT INTO users(id,username,password_hash) VALUES('u','admin','x')")
            .execute(&db)
            .await
            .unwrap();
        let token = "test-session-token";
        sqlx::query("INSERT INTO sessions(token_hash,user_id,expires_at) VALUES(?,?,9999999999)")
            .bind(sha_token(token))
            .bind("u")
            .execute(&db)
            .await
            .unwrap();
        let state = test_state(db);
        let mut headers = HeaderMap::new();
        headers.insert(header::COOKIE, format!("session={token}").parse().unwrap());
        let result = update_webui_url(
            State(state),
            Path(("s".into(), "c".into())),
            headers,
            Json(WebuiUrlInput { url: None }),
        )
        .await;
        assert!(
            matches!(result, Err(ApiError(StatusCode::FORBIDDEN, _))),
            "expected FORBIDDEN, got {result:?}"
        );
    }

    #[tokio::test]
    async fn update_webui_url_handler_returns_not_found_for_missing_server() {
        let db = memory_db().await;
        sqlx::query("INSERT INTO users(id,username,password_hash) VALUES('u','admin','x')")
            .execute(&db)
            .await
            .unwrap();
        let token = "test-session-token";
        sqlx::query("INSERT INTO sessions(token_hash,user_id,expires_at) VALUES(?,?,9999999999)")
            .bind(sha_token(token))
            .bind("u")
            .execute(&db)
            .await
            .unwrap();
        let state = test_state(db);
        let mut headers = HeaderMap::new();
        headers.insert(header::COOKIE, format!("session={token}").parse().unwrap());
        headers.insert(header::ORIGIN, "http://localhost".parse().unwrap());
        headers.insert(header::HOST, "localhost".parse().unwrap());
        let result = update_webui_url(
            State(state),
            Path(("missing".into(), "c".into())),
            headers,
            Json(WebuiUrlInput { url: None }),
        )
        .await;
        assert!(
            matches!(result, Err(ApiError(StatusCode::NOT_FOUND, _))),
            "expected NOT_FOUND, got {result:?}"
        );
    }
}
