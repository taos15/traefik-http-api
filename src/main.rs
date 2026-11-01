use std::{
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
    routing::{get, post},
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

async fn inventory(state: &AppState) -> anyhow::Result<Vec<Container>> {
    let servers = servers(&state.db).await?;
    let index = servers
        .iter()
        .map(|s| (s.id.as_str(), s))
        .collect::<std::collections::HashMap<_, _>>();
    let mut all = docker::aggregate(servers.clone()).await?;
    for c in &mut all {
        c.webui_url = provider::webui_url(c, index[&c.server_id.as_str()]);
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
            let view = match traefik::normalize(&raw, now()) {
                Ok(view) => view,
                Err(error) => {
                    *s.api_error.write().await = Some(error.to_string());
                    anyhow::bail!(
                        "Traefik API payload unavailable; retaining last published document: {error}"
                    )
                }
            };
            traefik::save(&s.db, &view).await?;
            *s.api_error.write().await = None;
            view
        }
        Err(error) => {
            *s.api_error.write().await = Some(error.to_string());
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
    for (name, service, server_id) in provider::route_owners(&containers, &config) {
        sqlx::query(
            "INSERT INTO provider_routes(router_name,service_name,server_id) VALUES(?,?,?)",
        )
        .bind(name)
        .bind(service)
        .bind(server_id)
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
}
