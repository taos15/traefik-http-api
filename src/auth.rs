use crate::{Container, traefik::FileView};
use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeSet;

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct Selection {
    pub provider: Option<String>,
    pub container_id: Option<String>,
    pub server_id: Option<String>,
    pub network: Option<String>,
    pub dns_name: Option<String>,
    pub protected: bool,
    #[serde(default)]
    pub file_owned: bool,
    #[serde(default)]
    pub last_reference: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Candidate {
    pub provider: String,
    pub container_id: String,
    pub server_id: String,
    pub name: String,
    pub network: Option<String>,
    pub eligible: bool,
    pub published_url: Option<String>,
}

#[derive(Clone)]
pub struct Decision {
    pub reference: Option<String>,
    pub required: bool,
    pub middlewares: serde_json::Map<String, Value>,
    pub services: serde_json::Map<String, Value>,
    pub selection: Selection,
    pub candidates: Vec<Candidate>,
    pub note: String,
}

pub fn provider(image: &str) -> Option<&'static str> {
    let image = image
        .split('@')
        .next()
        .unwrap_or(image)
        .rsplit_once(':')
        .map_or(image, |(path, _)| path);
    let path = image.to_ascii_lowercase();
    match path.as_str() {
        "quay.io/oauth2-proxy/oauth2-proxy" | "oauth2proxy/oauth2-proxy" => Some("oauth2proxy"),
        "ghcr.io/goauthentik/server"
        | "ghcr.io/goauthentik/proxy"
        | "goauthentik/server"
        | "goauthentik/proxy" => Some("authentik"),
        "authelia/authelia" | "authelia" => Some("authelia"),
        _ => None,
    }
}

fn rank(name: &str) -> u8 {
    match name {
        "oauth2proxy" => 0,
        "authentik" => 1,
        "authelia" => 2,
        _ => 3,
    }
}

pub fn candidates(
    containers: &[Container],
    servers: &[crate::DockerServer],
) -> Result<Vec<Candidate>> {
    let traefik: Vec<_> = containers
        .iter()
        .filter(|c| {
            c.state == "running"
                && c.image
                    .to_ascii_lowercase()
                    .split(':')
                    .next()
                    .is_some_and(|s| s.rsplit('/').next() == Some("traefik"))
        })
        .collect();
    let local = match traefik.as_slice() {
        [one] => Some(*one),
        [] => None,
        _ => bail!("multiple running Traefik containers; local auth selection ambiguous"),
    };
    let mut out = containers
        .iter()
        .filter(|c| c.state == "running")
        .filter_map(|c| Some((c, provider(&c.image)?)))
        .map(|(c, kind)| {
            let network = local.and_then(|t| {
                (t.server_id == c.server_id)
                    .then(|| {
                        c.networks
                            .iter()
                            .find(|n| {
                                *n != "bridge"
                                    && *n != "host"
                                    && *n != "none"
                                    && t.networks.contains(*n)
                            })
                            .cloned()
                    })
                    .flatten()
            });
            let server = servers.iter().find(|s| s.id == c.server_id);
            let port = match kind {
                "oauth2proxy" => 4180,
                "authentik" => 9000,
                _ => 9091,
            };
            let mut available = c
                .ports
                .iter()
                .filter(|p| p.private_port == port && crate::provider::reachable(p))
                .filter_map(|p| p.public_port)
                .collect::<Vec<_>>();
            available.sort_unstable();
            available.dedup();
            let published_url = server.and_then(|s| {
                (available.len() == 1)
                    .then(|| format!("http://{}:{}", s.upstream_host, available[0]))
            });
            Candidate {
                provider: kind.into(),
                container_id: c.id.clone(),
                server_id: c.server_id.clone(),
                name: c.name.clone(),
                eligible: network.is_some(),
                network,
                published_url,
            }
        })
        .collect::<Vec<_>>();
    out.sort_by_key(|c| (rank(&c.provider), c.name.clone()));
    Ok(out)
}

fn template(kind: &str) -> &'static str {
    match kind {
        "oauth2proxy" => include_str!("../templates/oauth2proxy_auth.yml"),
        "authentik" => include_str!("../templates/authentik_auth.yml"),
        _ => include_str!("../templates/authelia_auth.yml"),
    }
}

pub fn render(kind: &str, host: &str, service: &str) -> Result<Value> {
    if !host
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        || host.is_empty()
        || !service
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        || service.is_empty()
    {
        bail!("invalid Docker DNS or service name");
    }
    let rendered = template(kind)
        .replace("{{auth_host}}", host)
        .replace("{{auth_service}}", service);
    let value: Value = serde_yaml::from_str(&rendered)?;
    Ok(value)
}

pub fn service_name(c: &Container) -> String {
    let fallback = c
        .name
        .chars()
        .next()
        .map(|first| first.to_uppercase().collect::<String>() + &c.name[first.len_utf8()..])
        .unwrap_or_default();
    c.labels
        .get("traefik.service")
        .or_else(|| c.labels.get("traefik.name"))
        .cloned()
        .unwrap_or(fallback)
}

pub fn choose(
    containers: &[Container],
    servers: &[crate::DockerServer],
    view: Option<&FileView>,
    previous: &Selection,
) -> Result<Decision> {
    let candidates = candidates(containers, servers)?;
    let file = view.and_then(|v| v.config["http"]["middlewares"].get("default-forward-auth"));
    let file_error = view.is_some_and(|v| {
        v.resource_errors.iter().any(|e| {
            e.starts_with("middlewares:default-forward-auth@file")
                || e.starts_with("Traefik API reported resource errors")
                || e.starts_with("Traefik API returned an unrecognized error structure")
        })
    });
    if file.is_some() && view.is_some_and(crate::traefik::valid_default_auth) {
        let mut selection = previous.clone();
        selection.protected = true;
        selection.file_owned = true;
        selection.last_reference = Some("default-forward-auth@file".into());
        return Ok(Decision {
            reference: Some("default-forward-auth@file".into()),
            required: true,
            middlewares: Default::default(),
            services: Default::default(),
            selection,
            candidates,
            note: "Using default-forward-auth@file".into(),
        });
    }
    if file.is_some() || file_error {
        let mut selection = previous.clone();
        selection.protected = true;
        selection.file_owned = true;
        selection.last_reference = Some("default-forward-auth@file".into());
        return Ok(Decision {reference:None,required:true,middlewares:Default::default(),services:Default::default(),selection,candidates,note:"default-forward-auth@file is present but its dependencies are unavailable; protected routes omitted".into()});
    }
    let eligible: Vec<_> = candidates.iter().filter(|c| c.eligible).collect();
    let ambiguous = eligible.first().is_some_and(|first| {
        eligible
            .iter()
            .filter(|candidate| candidate.provider == first.provider)
            .count()
            > 1
    });
    let existing = if previous.file_owned {
        None
    } else {
        eligible.iter().copied().find(|c| {
            Some(&c.container_id) == previous.container_id.as_ref()
                && Some(&c.server_id) == previous.server_id.as_ref()
        })
    };
    let selected = if let Some(old) = existing {
        let best = eligible
            .iter()
            .copied()
            .filter(|c| rank(&c.provider) < rank(&old.provider))
            .collect::<Vec<_>>();
        let first = best.first().copied();
        if first.is_some_and(|c| best.iter().filter(|v| v.provider == c.provider).count() > 1) {
            Some(old)
        } else {
            first.or(Some(old))
        }
    } else if (previous.protected && previous.container_id.is_some() && !previous.file_owned)
        || ambiguous
    {
        None
    } else {
        let first = eligible.first().copied();
        if first.is_some_and(|c| eligible.iter().filter(|v| v.provider == c.provider).count() > 1) {
            None
        } else {
            first
        }
    };
    if let Some(selected) = selected {
        let name = format!(
            "default-forward-auth-signin-{}",
            selected.container_id.chars().take(12).collect::<String>()
        );
        let config = render(&selected.provider, &selected.name, &name)?;
        let middlewares = config["http"]["middlewares"]
            .as_object()
            .cloned()
            .unwrap_or_default();
        let mut services = serde_json::Map::new();
        if selected.provider == "oauth2proxy" {
            services.insert(name.clone(),json!({"loadBalancer":{"servers":[{"url":format!("http://{}:4180",selected.name)}]}}));
        }
        let selection = Selection {
            provider: Some(selected.provider.clone()),
            container_id: Some(selected.container_id.clone()),
            server_id: Some(selected.server_id.clone()),
            network: selected.network.clone(),
            dns_name: Some(selected.name.clone()),
            protected: true,
            file_owned: false,
            last_reference: Some("default-forward-auth@http".into()),
        };
        let note = format!(
            "Using {} from Docker network {}",
            selected.provider,
            selected.network.as_deref().unwrap_or("")
        );
        Ok(Decision {
            reference: Some("default-forward-auth@http".into()),
            required: true,
            middlewares,
            services,
            selection,
            candidates,
            note,
        })
    } else {
        let reason = if ambiguous {
            "Multiple eligible auth containers of the same priority; protected routers omitted until the ambiguity is resolved"
        } else if previous.protected {
            "Required default auth is unavailable; protected routers omitted"
        } else {
            "No automatic ForwardAuth detected; routes without explicit middleware are unprotected"
        };
        Ok(Decision {
            reference: None,
            required: previous.protected || ambiguous,
            middlewares: Default::default(),
            services: Default::default(),
            selection: Selection {
                protected: previous.protected || ambiguous,
                ..previous.clone()
            },
            candidates,
            note: reason.into(),
        })
    }
}

pub fn snippet(
    candidate: &Candidate,
    file_names: &BTreeSet<String>,
) -> Result<(String, Vec<String>)> {
    let Some(base) = candidate.published_url.as_deref() else {
        bail!("auth container's internal port must be published on a Traefik-reachable host");
    };
    let parsed = url::Url::parse(base)?;
    let host = parsed
        .host_str()
        .ok_or_else(|| anyhow::anyhow!("missing host"))?;
    let endpoint = format!(
        "{}:{}",
        host,
        parsed
            .port()
            .ok_or_else(|| anyhow::anyhow!("missing port"))?
    );
    let mut doc = render(
        &candidate.provider,
        &candidate.name,
        "default-forward-auth-oauth2proxy",
    )?;
    let mut warnings = vec![];
    if candidate.provider == "oauth2proxy" {
        doc["http"]["middlewares"]["oauth-verify"]["forwardAuth"]["address"] =
            json!(format!("{base}/oauth2-proxy/auth"));
        doc["http"]["services"] =
            json!({"default-forward-auth-oauth2proxy":{"loadBalancer":{"servers":[{"url":base}]}}});
        warnings
            .push("OAuth2 Proxy needs a sign-in path router on the application hostnames".into());
    } else if candidate.provider == "authentik" {
        doc["http"]["middlewares"]["default-forward-auth"]["forwardAuth"]["address"] =
            json!(format!("{base}/outpost.goauthentik.io/auth/traefik"));
        warnings.push("Authentik needs an outpost-path router on the application hostnames".into());
    } else {
        doc["http"]["middlewares"]["default-forward-auth"]["forwardAuth"]["address"] =
            json!(format!("{base}/api/authz/forward-auth"));
    }
    for kind in ["middlewares", "services"] {
        if let Some(map) = doc["http"][kind].as_object() {
            for name in map.keys() {
                if file_names.contains(&format!("{kind}:{name}@file")) {
                    warnings.push(format!(
                        "{kind} {name}@file already exists; rename before pasting"
                    ));
                }
            }
        }
    }
    warnings.push(format!("Traefik must be able to reach {endpoint}"));
    Ok((serde_yaml::to_string(&doc)?, warnings))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn server(id: &str) -> crate::DockerServer {
        crate::DockerServer {
            id: id.into(),
            name: id.into(),
            transport: "http".into(),
            endpoint: "http://host:2375".into(),
            tls_key: None,
            tls_cert: None,
            tls_ca: None,
            upstream_host: "192.168.1.5".into(),
            webui_host: "host.local".into(),
            prefer_network: false,
            docker_network: None,
            webui_same_as_server: false,
            enabled: true,
        }
    }
    fn container(name: &str, image: &str, server: &str, networks: &[&str]) -> Container {
        Container {
            id: name.into(),
            name: name.into(),
            image: image.into(),
            state: "running".into(),
            health: None,
            server_id: server.into(),
            server_name: server.into(),
            labels: Default::default(),
            ports: vec![],
            networks: networks.iter().map(|s| s.to_string()).collect(),
            webui_url: None,
        }
    }
    #[test]
    fn selects_local_priority_and_internal_signin_service() {
        let containers = vec![
            container("Traefik", "traefik:latest", "host", &["proxynet"]),
            container(
                "oauth2proxy",
                "quay.io/oauth2-proxy/oauth2-proxy:latest",
                "host",
                &["proxynet"],
            ),
            container(
                "authelia",
                "authelia/authelia:latest",
                "host",
                &["proxynet"],
            ),
            container(
                "remote-auth",
                "ghcr.io/goauthentik/server:latest",
                "remote",
                &["other"],
            ),
        ];
        let decision = choose(
            &containers,
            &[server("host"), server("remote")],
            None,
            &Selection::default(),
        )
        .unwrap();
        assert_eq!(
            decision.reference.as_deref(),
            Some("default-forward-auth@http")
        );
        assert_eq!(decision.selection.provider.as_deref(), Some("oauth2proxy"));
        assert_eq!(
            decision.middlewares["oauth-verify"]["forwardAuth"]["address"],
            "http://oauth2proxy:4180/oauth2-proxy/auth"
        );
        assert_eq!(
            decision.services["default-forward-auth-signin-oauth2proxy"]["loadBalancer"]["servers"]
                [0]["url"],
            "http://oauth2proxy:4180"
        );
        assert!(
            !decision
                .candidates
                .iter()
                .find(|c| c.name == "remote-auth")
                .unwrap()
                .eligible
        );
    }
    #[test]
    fn stopped_provider_never_downgrades_protected_routes() {
        let initial = vec![
            container("Traefik", "traefik:latest", "host", &["proxynet"]),
            container(
                "oauth2proxy",
                "quay.io/oauth2-proxy/oauth2-proxy:latest",
                "host",
                &["proxynet"],
            ),
        ];
        let selected = choose(&initial, &[server("host")], None, &Selection::default())
            .unwrap()
            .selection;
        let without = choose(&initial[..1], &[server("host")], None, &selected).unwrap();
        assert!(without.required && without.reference.is_none() && without.selection.protected);
    }
    #[test]
    fn valid_file_resource_beats_detected_provider() {
        let containers = vec![
            container("Traefik", "traefik:latest", "host", &["proxynet"]),
            container("authelia", "authelia/authelia", "host", &["proxynet"]),
        ];
        let view=crate::traefik::normalize(&json!({"routers":{},"services":{},"middlewares":{"default-forward-auth@file":{"status":"enabled","forwardAuth":{"address":"http://custom:9091/auth"}}}}),1).unwrap();
        let decision = choose(
            &containers,
            &[server("host")],
            Some(&view),
            &Selection::default(),
        )
        .unwrap();
        assert_eq!(
            decision.reference.as_deref(),
            Some("default-forward-auth@file")
        );
        assert!(decision.middlewares.is_empty());
    }
    #[test]
    fn duplicate_top_priority_does_not_choose_arbitrarily() {
        let containers = vec![
            container("Traefik", "traefik:latest", "host", &["proxynet"]),
            container("first", "authelia/authelia", "host", &["proxynet"]),
            container("second", "authelia/authelia", "host", &["proxynet"]),
        ];
        let decision = choose(&containers, &[server("host")], None, &Selection::default()).unwrap();
        assert!(decision.required && decision.reference.is_none());
    }
    #[test]
    fn missing_file_override_can_fall_back_to_local_auth() {
        let containers = vec![
            container("Traefik", "traefik:latest", "host", &["proxynet"]),
            container(
                "oauth2proxy",
                "quay.io/oauth2-proxy/oauth2-proxy:latest",
                "host",
                &["proxynet"],
            ),
        ];
        let view =
            crate::traefik::normalize(&json!({"routers":{},"services":{},"middlewares":{}}), 1)
                .unwrap();
        let previous = Selection {
            protected: true,
            file_owned: true,
            last_reference: Some("default-forward-auth@file".into()),
            ..Default::default()
        };
        let output = choose(&containers, &[server("host")], Some(&view), &previous).unwrap();
        assert_eq!(
            output.reference.as_deref(),
            Some("default-forward-auth@http")
        );
    }
    #[test]
    fn reset_preserves_protection_and_selects_available_provider() {
        let containers = vec![
            container("Traefik", "traefik:latest", "host", &["proxynet"]),
            container("authelia", "authelia/authelia", "host", &["proxynet"]),
        ];
        let previous = Selection {
            protected: true,
            ..Default::default()
        };
        let selected = choose(&containers, &[server("host")], None, &previous).unwrap();
        assert_eq!(
            selected.reference.as_deref(),
            Some("default-forward-auth@http")
        );
        let missing = choose(&containers[..1], &[server("host")], None, &previous).unwrap();
        assert!(missing.required && missing.reference.is_none());
    }
}
