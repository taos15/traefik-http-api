//! Traefik's API is an effective runtime view, not a round-trip copy of file sources.
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use sqlx::{Row, SqlitePool};
use std::collections::BTreeSet;
use std::time::Duration;

use crate::Container;
use std::collections::HashMap;

#[derive(Clone, Default, Serialize, Deserialize)]
pub struct RuntimeRouter {
    pub status: String,
    pub rule: String,
    pub service: String,
}

#[derive(Clone, Default, Serialize, Deserialize)]
pub struct RuntimeView {
    pub routers: HashMap<String, RuntimeRouter>,
    pub services: HashMap<String, String>,
    pub fetched_at: Option<i64>,
    pub api_reachable: bool,
}

#[derive(Clone, Default, Serialize, Deserialize)]
pub struct FileView {
    pub config: Value,
    #[serde(default)]
    pub status: Value,
    pub redacted_paths: Vec<String>,
    pub verified: bool,
    pub fetched_at: i64,
    #[serde(default)]
    pub resource_errors: Vec<String>,
}

#[derive(Clone, Serialize)]
pub struct SnapshotInfo {
    pub id: i64,
    pub verified: bool,
    pub created_at: i64,
    pub redacted_count: usize,
}

pub fn candidate_url(containers: &[Container], override_url: Option<&str>) -> Result<String> {
    if let Some(url) = override_url {
        return Ok(url.trim_end_matches('/').to_owned());
    }
    let matches: Vec<_> = containers
        .iter()
        .filter(|c| {
            c.state == "running"
                && c.image
                    .split(':')
                    .next()
                    .is_some_and(|image| image.rsplit('/').next() == Some("traefik"))
                && c.networks
                    .iter()
                    .any(|n| n != "bridge" && n != "host" && n != "none")
        })
        .collect();
    if matches.len() != 1 {
        bail!(
            "expected one Traefik container on a configured custom Docker network, found {} (set TRAEFIK_API_URL if necessary)",
            matches.len()
        );
    }
    Ok(format!("http://{}:8080", matches[0].name))
}

pub async fn fetch(url: &str, username: Option<&str>, password: Option<&str>) -> Result<Value> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(8))
        .redirect(reqwest::redirect::Policy::none())
        .build()?;
    let mut req = client.get(format!("{url}/api/rawdata"));
    if let Some(user) = username {
        req = req.basic_auth(user, password);
    }
    let response = req.send().await.context("Traefik API unreachable")?;
    if response.status().is_redirection() {
        bail!(
            "Traefik API redirected to login; configure a Docker-network API router with Basic auth"
        );
    }
    if matches!(response.status().as_u16(), 401 | 403) {
        bail!("Traefik API rejected credentials");
    }
    let response = response
        .error_for_status()
        .context("Traefik API returned an error")?;
    let value: Value = response
        .json()
        .await
        .context("Traefik API returned invalid JSON")?;
    for section in ["routers", "services", "middlewares"] {
        if let Some(v) = value.get(section) {
            anyhow::ensure!(v.is_object(), "unexpected Traefik {section} shape");
        }
    }
    Ok(value)
}

fn safe_key(key: &str) -> bool {
    matches!(
        key,
        "entryPoints"
            | "rule"
            | "service"
            | "middlewares"
            | "priority"
            | "tls"
            | "observability"
            | "loadBalancer"
            | "weighted"
            | "mirroring"
            | "failover"
            | "servers"
            | "url"
            | "weight"
            | "healthCheck"
            | "passHostHeader"
            | "responseForwarding"
            | "flushInterval"
            | "sticky"
            | "strategy"
            | "serversTransport"
            | "forwardAuth"
            | "address"
            | "trustForwardHeader"
            | "authResponseHeaders"
            | "authRequestHeaders"
            | "chain"
            | "errors"
            | "query"
            | "headers"
            | "customRequestHeaders"
            | "customResponseHeaders"
            | "basicAuth"
            | "digestAuth"
            | "users"
            | "usersFile"
            | "removeHeader"
            | "realm"
            | "ipAllowList"
            | "sourceRange"
            | "ipStrategy"
            | "depth"
            | "excludedIPs"
            | "plugin"
            | "redirectScheme"
            | "redirectRegex"
            | "replacePath"
            | "replacePathRegex"
            | "stripPrefix"
            | "addPrefix"
            | "rateLimit"
            | "inFlightReq"
            | "compress"
            | "retry"
            | "buffering"
            | "maxBodyBytes"
            | "memRequestBodyBytes"
            | "maxRequestBodyBytes"
            | "memResponseBodyBytes"
            | "average"
            | "burst"
            | "period"
            | "resolver"
            | "certResolver"
            | "options"
            | "domains"
            | "main"
            | "sans"
            | "certificates"
            | "stores"
            | "rootCAs"
            | "cert"
            | "key"
            | "clientAuth"
            | "sslRedirect"
            | "stsSeconds"
            | "browserXssFilter"
            | "contentTypeNosniff"
            | "contentType"
            | "forceSTSHeader"
            | "sslHost"
            | "stsIncludeSubdomains"
            | "stsPreload"
            | "frameDeny"
            | "customFrameOptionsValue"
            | "accessControlAllowOriginList"
            | "accessControlAllowCredentials"
            | "accessControlAllowMethods"
            | "accessControlAllowHeaders"
            | "accessControlExposeHeaders"
            | "accessControlMaxAge"
            | "addVaryHeader"
            | "hostsProxyHeaders"
            | "hostname"
            | "scheme"
            | "port"
            | "permanent"
            | "regex"
            | "replacement"
            | "prefixes"
            | "circuitBreaker"
            | "expression"
            | "passTLSClientCert"
            | "requestHeaderModifier"
            | "responseHeaderModifier"
            | "dialTimeout"
            | "dialKeepAlive"
            | "forwardingTimeouts"
            | "readIdleTimeout"
            | "pingTimeout"
            | "idleConnTimeout"
            | "responseHeaderTimeout"
            | "triggerDuration"
            | "interval"
            | "timeout"
            | "path"
            | "method"
            | "mode"
            | "field"
            | "sourceCriterion"
            | "requestHeaderName"
            | "requestHost"
            | "ip"
            | "ipv6Subnet"
            | "requestHeader"
            | "responseHeader"
            | "type"
            | "name"
    )
}

fn secret_key(key: &str) -> bool {
    let key = key.to_ascii_lowercase();
    [
        "authorization",
        "cookie",
        "token",
        "secret",
        "password",
        "api-key",
        "apikey",
        "x-api-",
        "credential",
        "privatekey",
        "clientkey",
    ]
    .iter()
    .any(|v| key.contains(v))
}

fn strip_userinfo(s: &str) -> Option<String> {
    let mut url = url::Url::parse(s).ok()?;
    if url.username().is_empty() && url.password().is_none() && url.query().is_none() {
        return None;
    }
    if !url.username().is_empty() || url.password().is_some() {
        url.set_username("REDACTED").ok()?;
        url.set_password(None).ok()?;
    }
    // Queries are arbitrary user-defined input; they may contain credentials
    // even when their parameter names do not look sensitive.
    if url.query().is_some() {
        url.set_query(Some("REDACTED"));
    }
    Some(url.to_string())
}

fn redact(value: &mut Value, path: &str, paths: &mut Vec<String>) {
    match value {
        Value::Object(map) => {
            for (key, child) in map {
                let next = format!("{path}.{key}");
                let parent = path.rsplit('.').next().unwrap_or("");
                if (parent == "customRequestHeaders" || parent == "customResponseHeaders")
                    || (key == "users"
                        && (path.ends_with("basicAuth") || path.ends_with("digestAuth")))
                    || matches!(
                        key.as_str(),
                        "usersFile"
                            | "key"
                            | "cert"
                            | "clientKey"
                            | "clientCert"
                            | "rootCAs"
                            | "plugin"
                    )
                    || (secret_key(key)
                        && parent != "headers"
                        && !matches!(key.as_str(), "authRequestHeaders" | "authResponseHeaders"))
                {
                    *child = Value::String("REDACTED".into());
                    paths.push(next);
                } else {
                    redact(child, &next, paths);
                }
            }
        }
        Value::Array(arr) => {
            for (index, child) in arr.iter_mut().enumerate() {
                redact(child, &format!("{path}[{index}]"), paths);
            }
        }
        Value::String(s) => {
            if let Some(replacement) = strip_userinfo(s) {
                *s = replacement;
                paths.push(path.into());
            }
        }
        _ => {}
    }
}

/// Preserve only known resource types and known configuration keys. An unknown key
/// excludes the entire resource rather than claiming it can be restored faithfully.
pub fn normalize(raw: &Value, time: i64) -> Result<FileView> {
    anyhow::ensure!(raw.is_object(), "Traefik rawdata must be an object");
    anyhow::ensure!(
        raw.get("routers").is_some_and(Value::is_object)
            && raw.get("middlewares").is_some_and(Value::is_object)
            && raw.get("services").is_some_and(Value::is_object),
        "Traefik rawdata is incomplete; refusing to replace file snapshot"
    );
    let mut http = Map::new();
    let mut redacted_paths = Vec::new();
    let mut resource_errors = Vec::new();
    if let Some(errors) = raw.get("errors")
        && errors.is_array()
        && errors.as_array().is_some_and(|items| !items.is_empty())
    {
        resource_errors
            .push("Traefik API reported resource errors; snapshot may be incomplete".into());
    }
    let mut verified = !raw
        .get("errors")
        .is_some_and(|v| !v.is_null() && v.as_array().is_none_or(|a| !a.is_empty()));
    if raw
        .get("errors")
        .is_some_and(|v| !v.is_null() && !v.is_array())
    {
        resource_errors.push("Traefik API returned an unrecognized error structure".into());
    }
    if raw.get("tcp").is_some() || raw.get("udp").is_some() || raw.get("tls").is_some() {
        verified = false;
    }
    if raw.get("tcpRouters").is_some()
        || raw.get("tcpServices").is_some()
        || raw.get("tcpMiddlewares").is_some()
        || raw.get("udpRouters").is_some()
        || raw.get("udpServices").is_some()
    {
        verified = false;
    }
    let mut status = Map::new();
    let types = [
        ("routers", "routers"),
        ("services", "services"),
        ("middlewares", "middlewares"),
    ];
    for (source, dest) in types {
        let mut resources = Map::new();
        if let Some(map) = raw.get(source).and_then(Value::as_object) {
            for (full_name, value) in map.iter().filter(|(key, _)| key.ends_with("@file")) {
                let name = full_name.trim_end_matches("@file");
                let Some(fields) = value.as_object() else {
                    verified = false;
                    continue;
                };
                if fields
                    .get("status")
                    .and_then(Value::as_str)
                    .is_some_and(|s| s != "enabled")
                {
                    verified = false;
                    resource_errors.push(format!("{dest}:{name}@file"));
                    // A broken middleware is not a usable file override.
                    continue;
                }
                let unknown: Vec<_> = fields
                    .keys()
                    .filter(|k| {
                        !safe_key(k)
                            && !matches!(
                                k.as_str(),
                                "status" | "usedBy" | "using" | "serverStatus" | "error" | "type"
                            )
                    })
                    .collect();
                // `errors` is a valid middleware type, not a runtime error
                // field. Traefik reports resource errors via `status`.
                if !unknown.is_empty() {
                    verified = false;
                    resource_errors.push(format!("{dest}:{name}@file (unsupported fields)"));
                    tracing::warn!(%full_name, ?unknown, "unknown Traefik fields; omitting from export");
                    continue;
                }
                if source == "routers" && fields.get("middlewares").is_some_and(|v| !v.is_array()) {
                    verified = false;
                    resource_errors.push(format!("{dest}:{name}@file (invalid middleware list)"));
                    continue;
                }
                status.insert(format!("{dest}:{name}"), json!("enabled"));
                let mut clean = fields
                    .iter()
                    .filter(|(k, _)| {
                        !matches!(
                            k.as_str(),
                            "status" | "usedBy" | "using" | "serverStatus" | "error" | "type"
                        )
                    })
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect::<Map<_, _>>();
                let mut entry = Value::Object(std::mem::take(&mut clean));
                redact(
                    &mut entry,
                    &format!("http.{dest}.{name}"),
                    &mut redacted_paths,
                );
                resources.insert(name.to_owned(), entry);
            }
        }
        http.insert(dest.into(), Value::Object(resources));
    }
    // Runtime resources include inherited entrypoint TLS/middleware defaults, which
    // cannot be reliably reversed. Export is explicitly a recovery aid, not a backup.
    Ok(FileView {
        config: json!({"http":http}),
        status: Value::Object(status),
        redacted_paths,
        verified,
        fetched_at: time,
        resource_errors,
    })
}

pub async fn latest(db: &SqlitePool) -> Result<Option<FileView>> {
    let row = sqlx::query("SELECT config,status,redacted_paths,resource_errors,verified,created_at FROM file_snapshots ORDER BY id DESC LIMIT 1").fetch_optional(db).await?;
    row.map(|r| {
        Ok(FileView {
            config: serde_json::from_str(&r.get::<String, _>("config"))?,
            status: serde_json::from_str(&r.get::<String, _>("status"))?,
            redacted_paths: serde_json::from_str(&r.get::<String, _>("redacted_paths"))?,
            verified: r.get::<i64, _>("verified") != 0,
            fetched_at: r.get("created_at"),
            resource_errors: serde_json::from_str(&r.get::<String, _>("resource_errors"))?,
        })
    })
    .transpose()
}

pub async fn save(db: &SqlitePool, view: &FileView) -> Result<()> {
    let config = view.config.to_string();
    let paths = serde_json::to_string(&view.redacted_paths)?;
    let mut tx = db.begin().await?;
    let prev: Option<(i64, String, String, String, String, i64)> = sqlx::query_as(
        "SELECT id,config,status,redacted_paths,resource_errors,verified FROM file_snapshots ORDER BY id DESC LIMIT 1",
    )
    .fetch_optional(&mut *tx)
    .await?;
    let status = view.status.to_string();
    let errors = serde_json::to_string(&view.resource_errors)?;
    if prev.as_ref().is_some_and(|(_, c, st, p, e, v)| {
        c == &config
            && st == &status
            && p == &paths
            && e == &errors
            && *v == i64::from(view.verified)
    }) {
        return Ok(());
    }
    sqlx::query(
        "INSERT INTO file_snapshots(config,status,redacted_paths,resource_errors,verified,created_at) VALUES(?,?,?,?,?,?)",
    )
    .bind(config)
    .bind(status)
    .bind(paths)
    .bind(errors)
    .bind(view.verified)
    .bind(view.fetched_at)
    .execute(&mut *tx)
    .await?;
    sqlx::query("DELETE FROM file_snapshots WHERE id NOT IN (SELECT id FROM file_snapshots ORDER BY id DESC LIMIT 30) AND id NOT IN (SELECT id FROM file_snapshots WHERE verified=1 ORDER BY id DESC LIMIT 1)").execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(())
}

fn server_url_canonical(url: &str) -> Option<String> {
    let parsed = url::Url::parse(url).ok()?;
    let host = parsed.host_str()?;
    let authority = match parsed.port() {
        Some(port) => format!("{host}:{port}"),
        None => host.to_owned(),
    };
    Some(format!("{}://{}", parsed.scheme(), authority))
}

/// Stable fingerprint of a service's upstream servers, excluding credentials,
/// query strings and other variable runtime data.
pub fn service_fingerprint(service: &Value) -> Option<String> {
    let servers = service.get("loadBalancer")?.get("servers")?.as_array()?;
    let mut urls = Vec::new();
    for s in servers {
        let url = s.get("url").and_then(Value::as_str)?;
        let canonical = server_url_canonical(url)?;
        urls.push(canonical);
    }
    urls.sort_unstable();
    urls.dedup();
    if urls.is_empty() {
        return None;
    }
    Some(urls.join(","))
}

pub fn runtime_routers(raw: &Value, time: i64) -> RuntimeView {
    let mut routers = HashMap::new();
    let mut services = HashMap::new();
    let raw_services = raw.get("services").and_then(Value::as_object);
    if let Some(map) = raw.get("routers").and_then(Value::as_object) {
        for (full_name, value) in map {
            if !full_name.ends_with("@http") {
                continue;
            }
            let name = full_name.trim_end_matches("@http");
            let status = value
                .get("status")
                .and_then(Value::as_str)
                .unwrap_or("unknown");
            let rule = value.get("rule").and_then(Value::as_str).unwrap_or("");
            let service = value.get("service").and_then(Value::as_str).unwrap_or("");
            routers.insert(
                name.to_owned(),
                RuntimeRouter {
                    status: status.to_owned(),
                    rule: rule.to_owned(),
                    service: service.to_owned(),
                },
            );
            if status == "enabled" {
                let service_full = if service.ends_with("@http") {
                    service.to_owned()
                } else {
                    format!("{service}@http")
                };
                let service_name = service.strip_suffix("@http").unwrap_or(service);
                let raw_service = raw_services
                    .and_then(|s| s.get(&service_full))
                    .or_else(|| raw_services.and_then(|s| s.get(service_name)));
                let service_status = raw_service
                    .and_then(|s| s.get("status"))
                    .and_then(Value::as_str)
                    .unwrap_or("unknown");
                if service_status == "enabled"
                    && let Some(fp) = raw_service.and_then(service_fingerprint)
                {
                    services.insert(service_name.to_owned(), fp);
                }
            }
        }
    }
    RuntimeView {
        routers,
        services,
        fetched_at: Some(time),
        api_reachable: true,
    }
}

pub async fn save_runtime(db: &SqlitePool, view: &RuntimeView) -> Result<()> {
    let mut tx = db.begin().await?;
    sqlx::query("DELETE FROM runtime_routers")
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM runtime_services")
        .execute(&mut *tx)
        .await?;
    let fetched_at = view.fetched_at.unwrap_or(0);
    for (name, router) in &view.routers {
        sqlx::query(
            "INSERT INTO runtime_routers(name,status,rule,service,fetched_at) VALUES(?,?,?,?,?)",
        )
        .bind(name)
        .bind(&router.status)
        .bind(&router.rule)
        .bind(&router.service)
        .bind(fetched_at)
        .execute(&mut *tx)
        .await?;
    }
    for (name, fingerprint) in &view.services {
        sqlx::query("INSERT INTO runtime_services(name,fingerprint,fetched_at) VALUES(?,?,?)")
            .bind(name)
            .bind(fingerprint)
            .bind(fetched_at)
            .execute(&mut *tx)
            .await?;
    }
    sqlx::query(
        "INSERT INTO runtime_snapshot(id,fetched_at,api_reachable) VALUES(1,?,?) ON CONFLICT(id) DO UPDATE SET fetched_at=excluded.fetched_at, api_reachable=excluded.api_reachable",
    )
    .bind(fetched_at)
    .bind(view.api_reachable)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(())
}

pub async fn load_runtime(db: &SqlitePool) -> Result<RuntimeView> {
    let mut tx = db.begin().await?;
    let meta: Option<(i64, i64)> =
        sqlx::query_as("SELECT fetched_at, api_reachable FROM runtime_snapshot WHERE id=1")
            .fetch_optional(&mut *tx)
            .await?;
    let (fetched_at, api_reachable) = meta.unwrap_or((0, 0));
    let router_rows = sqlx::query("SELECT name, status, rule, service FROM runtime_routers")
        .fetch_all(&mut *tx)
        .await?;
    let mut routers = HashMap::new();
    for row in router_rows {
        routers.insert(
            row.get::<String, _>("name"),
            RuntimeRouter {
                status: row.get("status"),
                rule: row.get("rule"),
                service: row.get::<Option<String>, _>("service").unwrap_or_default(),
            },
        );
    }
    let service_rows = sqlx::query("SELECT name, fingerprint FROM runtime_services")
        .fetch_all(&mut *tx)
        .await?;
    let mut services = HashMap::new();
    for row in service_rows {
        services.insert(
            row.get::<String, _>("name"),
            row.get::<String, _>("fingerprint"),
        );
    }
    tx.commit().await?;
    Ok(RuntimeView {
        routers,
        services,
        fetched_at: (fetched_at != 0).then_some(fetched_at),
        api_reachable: api_reachable != 0,
    })
}

pub async fn history(db: &SqlitePool) -> Result<Vec<SnapshotInfo>> {
    let rows = sqlx::query("SELECT id,verified,created_at,redacted_paths FROM file_snapshots ORDER BY id DESC LIMIT 31").fetch_all(db).await?;
    rows.into_iter()
        .map(|r| {
            Ok(SnapshotInfo {
                id: r.get("id"),
                verified: r.get::<i64, _>("verified") != 0,
                created_at: r.get("created_at"),
                redacted_count: serde_json::from_str::<Vec<String>>(
                    &r.get::<String, _>("redacted_paths"),
                )?
                .len(),
            })
        })
        .collect()
}

pub async fn export(db: &SqlitePool, id: Option<i64>) -> Result<String> {
    let row = if let Some(id) = id {
        sqlx::query("SELECT config,redacted_paths FROM file_snapshots WHERE id=?")
            .bind(id)
            .fetch_optional(db)
            .await?
    } else {
        sqlx::query("SELECT config,redacted_paths FROM file_snapshots WHERE verified=1 ORDER BY id DESC LIMIT 1").fetch_optional(db).await?
    };
    let row = row.context("no matching file snapshot")?;
    let paths: Vec<String> = serde_json::from_str(&row.get::<String, _>("redacted_paths"))?;
    let value: Value = serde_json::from_str(&row.get::<String, _>("config"))?;
    let mut output = String::from(
        "# Best-effort effective @file configuration, not an original-file backup.\n# Review before use; inherited defaults, static config, secrets and certificates may be absent.\n",
    );
    for path in paths {
        output.push_str(&format!("# REDACTED: {path} (restore manually)\n"));
    }
    output.push_str(&serde_yaml::to_string(&value)?);
    Ok(output)
}

/// Check the dependencies that make an auth middleware usable. An effective
/// middleware name alone does not prove its chain or errors service exists.
pub fn valid_default_auth(view: &FileView) -> bool {
    fn middleware(view: &FileView, name: &str, seen: &mut BTreeSet<String>) -> bool {
        let name = name.strip_suffix("@file").unwrap_or(name);
        if name.contains('@') || !seen.insert(name.to_owned()) {
            return false;
        }
        let valid = view.status[format!("middlewares:{name}")] == "enabled"
            && view.config["http"]["middlewares"]
                .get(name)
                .is_some_and(|value| {
                    let chain = value["chain"]["middlewares"]
                        .as_array()
                        .is_none_or(|items| {
                            items.iter().all(|item| {
                                item.as_str()
                                    .is_some_and(|entry| middleware(view, entry, seen))
                            })
                        });
                    let service = value["errors"]["service"].as_str().is_none_or(|service| {
                        let service = service.strip_suffix("@file").unwrap_or(service);
                        !service.contains('@')
                            && view.status[format!("services:{service}")] == "enabled"
                    });
                    // The address can contain a query that is redacted in the
                    // stored snapshot. The live resource status, not the
                    // redacted export value, determines its availability.
                    chain && service
                });
        seen.remove(name);
        valid
    }
    middleware(view, "default-forward-auth", &mut BTreeSet::new())
}

pub fn file_resources(view: &FileView) -> BTreeSet<String> {
    ["routers", "services", "middlewares"]
        .into_iter()
        .flat_map(|kind| {
            view.config["http"][kind]
                .as_object()
                .into_iter()
                .flat_map(move |map| map.keys().map(move |name| format!("{kind}:{name}@file")))
        })
        .collect()
}

/// Diagnostics are advisory: a manually selected middleware remains the
/// operator's choice, even if it is unavailable or intentionally skips auth.
pub fn route_warnings(output: &Value, view: Option<&FileView>) -> Vec<String> {
    let mut warnings = Vec::new();
    let Some(routers) = output["http"]["routers"].as_object() else {
        return warnings;
    };
    let Some(view) = view else { return warnings };
    let file_routers = view.config["http"]["routers"].as_object();
    for (name, router) in routers {
        let rule = router["rule"].as_str().unwrap_or("");
        if let Some(file_routers) = file_routers {
            for (file, resource) in file_routers {
                if !rule.is_empty() && resource["rule"].as_str().is_some_and(|r| r.contains(rule)) {
                    warnings.push(format!("{name}: Host rule may overlap {file}@file"));
                }
            }
        }
        if let Some(middlewares) = router["middlewares"].as_array() {
            for middleware in middlewares.iter().filter_map(Value::as_str) {
                if let Some(file) = middleware.strip_suffix("@file") {
                    if view.status[format!("middlewares:{file}")] != "enabled" {
                        warnings.push(format!("{name}: explicit {middleware} is unavailable in the latest file snapshot"));
                    }
                } else if let Some(http) = middleware.strip_suffix("@http") {
                    if output["http"]["middlewares"].get(http).is_none() {
                        warnings.push(format!(
                            "{name}: {middleware} is not defined in generated output"
                        ));
                    }
                } else if !middleware.contains('@')
                    && output["http"]["middlewares"].get(middleware).is_none()
                {
                    warnings.push(format!("{name}: {middleware}@http is not defined in generated output; use @file for file middlewares"));
                }
            }
            if middlewares.is_empty() {
                warnings.push(format!(
                    "{name}: no router middleware is attached; check whether this is intentional"
                ));
            }
        }
    }
    warnings
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn redaction_precedes_storage() {
        let raw = json!({"routers":{},"middlewares":{"auth@file":{"basicAuth":{"users":["admin:hash"]},"headers":{"customRequestHeaders":{"Authorization":"Bearer xyz","X-Api-Key":"abc"}},"status":"enabled"}},"services":{"up@file":{"loadBalancer":{"servers":[{"url":"https://user:pass@host:443"}]}}}});
        let view = normalize(&raw, 1).unwrap();
        let data = view.config.to_string();
        assert!(
            !data.contains("admin:hash") && !data.contains("Bearer xyz") && !data.contains("pass@")
        );
        assert!(!view.redacted_paths.is_empty());
    }
    #[test]
    fn snapshot_does_not_store_header_values_or_url_queries() {
        let raw = json!({"routers":{},"services":{},"middlewares":{"guard@file":{"status":"enabled","headers":{"customRequestHeaders":{"X-Private-Session":"very-secret"}},"forwardAuth":{"address":"http://auth:9000/check?session=topsecret"}}}});
        let view = normalize(&raw, 1).unwrap();
        let data = view.config.to_string();
        assert!(!data.contains("very-secret") && !data.contains("topsecret"));
    }
    #[tokio::test]
    async fn persisted_snapshot_and_export_do_not_contain_credentials() {
        let db = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::migrate!().run(&db).await.unwrap();
        let raw = json!({"routers":{},"services":{"backend@file":{"status":"enabled","loadBalancer":{"servers":[{"url":"http://user:password@internal:8080/path?token=secret"}]}}},"middlewares":{"login@file":{"status":"enabled","basicAuth":{"users":["admin:hashed-password"]}}}});
        let snapshot = normalize(&raw, 1).unwrap();
        save(&db, &snapshot).await.unwrap();
        let stored: (String,) = sqlx::query_as("SELECT config FROM file_snapshots LIMIT 1")
            .fetch_one(&db)
            .await
            .unwrap();
        let generated = export(&db, None).await.unwrap();
        for text in [&stored.0, &generated] {
            assert!(!text.contains("hashed-password"));
            assert!(!text.contains("user:password"));
            assert!(!text.contains("token=secret"));
        }
        assert!(generated.contains("REDACTED"));
    }
    #[test]
    fn file_override_checks_chain_and_signin_service() {
        let raw = json!({"routers":{},"middlewares":{
            "default-forward-auth@file":{"status":"enabled","chain":{"middlewares":["signin@file","verify@file"]}},
            "signin@file":{"status":"enabled","errors":{"service":"proxy@file","status":["401"],"query":"/login"}},
            "verify@file":{"status":"enabled","forwardAuth":{"address":"http://proxy:4180/auth"}}
        },"services":{"proxy@file":{"status":"enabled","loadBalancer":{"servers":[{"url":"http://proxy:4180"}]}}}});
        let valid = normalize(&raw, 1).unwrap();
        assert!(valid_default_auth(&valid));
        let mut broken = raw.clone();
        broken["services"]
            .as_object_mut()
            .unwrap()
            .remove("proxy@file");
        assert!(!valid_default_auth(&normalize(&broken, 2).unwrap()));
    }

    fn runtime_raw(service_status: &str) -> Value {
        json!({
            "routers": {
                "App@http": {
                    "status": "enabled",
                    "rule": "Host(`app.example.com`)",
                    "service": "App@http"
                }
            },
            "services": {
                "App@http": {
                    "status": service_status,
                    "loadBalancer": {
                        "servers": [{"url": "http://10.0.0.2:8080"}]
                    }
                }
            }
        })
    }

    #[test]
    fn runtime_routers_enabled_service_fingerprints() {
        let view = runtime_routers(&runtime_raw("enabled"), 1);
        assert!(view.services.contains_key("App"));
    }

    #[test]
    fn runtime_routers_disabled_service_does_not_fingerprint() {
        let view = runtime_routers(&runtime_raw("disabled"), 1);
        assert!(!view.services.contains_key("App"));
    }

    #[test]
    fn runtime_routers_missing_service_status_does_not_fingerprint() {
        let mut raw = runtime_raw("enabled");
        raw["services"]["App@http"]
            .as_object_mut()
            .unwrap()
            .remove("status");
        let view = runtime_routers(&raw, 1);
        assert!(!view.services.contains_key("App"));
    }
}
