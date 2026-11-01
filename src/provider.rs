use crate::{Container, DockerServer, Port};
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};
use url::Url;

pub fn reachable(port: &Port) -> bool {
    port.public_port.is_some()
        && port.protocol == "tcp"
        && !matches!(port.host_ip.as_deref(), Some("127.0.0.1" | "::1" | "[::1]"))
}

fn host(value: &str) -> anyhow::Result<String> {
    let candidate = value.trim().trim_end_matches('/');
    let parsed = Url::parse(&format!("http://{candidate}"))?;
    anyhow::ensure!(
        parsed.host_str().is_some()
            && parsed.port().is_none()
            && parsed.path() == "/"
            && parsed.query().is_none()
            && parsed.username().is_empty()
            && parsed.password().is_none()
            && !matches!(parsed.host_str(), Some("0.0.0.0" | "::" | "[::]")),
        "invalid host"
    );
    Ok(parsed.host_str().unwrap().to_owned())
}

fn scheme(value: &str) -> anyhow::Result<&str> {
    anyhow::ensure!(matches!(value, "http" | "https"), "invalid URL scheme");
    Ok(value)
}

fn url(scheme: &str, hostname: &str, port: u16, path: &str) -> anyhow::Result<String> {
    let host = host(hostname)?;
    Ok(format!(
        "{}://{}:{}{}",
        scheme,
        if host.contains(':') {
            format!("[{host}]")
        } else {
            host
        },
        port,
        path
    ))
}

fn port_for(c: &Container, private: Option<u16>, published: Option<u16>) -> Option<u16> {
    let mut choices = c
        .ports
        .iter()
        .filter(|p| {
            reachable(p)
                && private.is_none_or(|v| p.private_port == v)
                && published.is_none_or(|v| p.public_port == Some(v))
        })
        .filter_map(|p| p.public_port)
        .collect::<Vec<_>>();
    choices.sort_unstable();
    choices.dedup();
    if choices.len() == 1 {
        choices.first().copied()
    } else {
        None
    }
}

fn private_port_for(c: &Container, published: u16) -> Option<u16> {
    let mut choices = c
        .ports
        .iter()
        .filter(|p| p.protocol == "tcp" && p.public_port == Some(published))
        .map(|p| p.private_port)
        .collect::<Vec<_>>();
    choices.sort_unstable();
    choices.dedup();
    (choices.len() == 1).then(|| choices[0])
}

fn single_published_tcp_port(c: &Container) -> Option<u16> {
    let mut choices = c
        .ports
        .iter()
        .filter(|p| p.protocol == "tcp")
        .filter_map(|p| p.public_port)
        .collect::<Vec<_>>();
    choices.sort_unstable();
    choices.dedup();
    (choices.len() == 1).then(|| choices[0])
}

fn upstream_target<'a>(
    c: &'a Container,
    server: &'a DockerServer,
    explicit_port: Option<u16>,
) -> Option<(&'a str, u16)> {
    let use_network = server.prefer_network
        && server
            .docker_network
            .as_ref()
            .is_some_and(|network| c.networks.contains(network));

    if use_network {
        let published = explicit_port.or_else(|| single_published_tcp_port(c));
        if let Some(private) = published.and_then(|port| private_port_for(c, port)) {
            return Some((&c.name, private));
        }
    }

    Some((&server.upstream_host, port_for(c, None, explicit_port)?))
}

// A single published TCP port does not imply an HTTP interface (for example,
// Redis, PostgreSQL, and Docker socket proxies publish TCP ports too).
fn non_web_port(port: u16) -> bool {
    matches!(
        port,
        22 | 25
            | 53
            | 110
            | 143
            | 389
            | 445
            | 465
            | 587
            | 636
            | 993
            | 995
            | 1883
            | 2375
            | 2376
            | 3306
            | 5432
            | 5672
            | 6379
            | 6380
            | 8883
            | 9092
            | 11211
            | 15672
            | 27017
    )
}

fn fallback_web_port(c: &Container) -> Option<u16> {
    let mut candidates: Vec<_> = c
        .ports
        .iter()
        .filter(|p| reachable(p) && !non_web_port(p.private_port))
        .filter_map(|p| p.public_port)
        .collect();
    candidates.sort_unstable();
    candidates.dedup();
    // If the container exposes several services, don't guess which one is a WebUI.
    if candidates.len() == 1 && !non_web_port(candidates[0]) {
        candidates.first().copied()
    } else {
        None
    }
}

fn default_route_name(name: &str) -> String {
    let mut chars = name.chars();
    chars
        .next()
        .map(|first| first.to_uppercase().collect::<String>() + chars.as_str())
        .unwrap_or_default()
}

pub fn webui_url(c: &Container, server: &DockerServer) -> Option<String> {
    if c.state != "running" {
        return None;
    }
    let labels = &c.labels;
    let explicit_port = labels
        .get("traefik.webuiport")
        .map(|value| value.parse::<u16>())
        .transpose()
        .ok()?;
    let override_host = labels
        .get("traefik.webui.host")
        .map(String::as_str)
        .unwrap_or_else(|| server.browser_host());
    let label = labels
        .get("net.unraid.docker.webui")
        .or_else(|| labels.get("traefik.webui.url"));
    if let Some(template) = label {
        let mut text = template.replace("[IP]", override_host);
        while let Some(start) = text.find("[PORT:") {
            let end = text[start..].find(']')? + start;
            let private = text[start + 6..end].parse::<u16>().ok()?;
            let published = port_for(c, Some(private), None)?;
            text.replace_range(start..=end, &published.to_string());
        }
        let parsed = Url::parse(&text).ok()?;
        let protocol = scheme(parsed.scheme()).ok()?;
        if parsed.username() != "" || parsed.password().is_some() || parsed.fragment().is_some() {
            return None;
        }
        let published = parsed
            .port()
            .or(explicit_port)
            .or_else(|| fallback_web_port(c));
        if let Some(port) = published {
            port_for(c, None, Some(port))?;
        }
        let h = host(override_host).ok()?;
        let path = if parsed.path().is_empty() {
            "/"
        } else {
            parsed.path()
        };
        let mut result = if let Some(port) = published {
            Url::parse(&url(protocol, &h, port, path).ok()?).ok()?
        } else {
            let mut target = parsed.clone();
            target.set_host(Some(&h)).ok()?;
            target
        };
        result.set_query(parsed.query());
        return Some(result.into());
    }
    let published = if let Some(port) = explicit_port {
        port_for(c, None, Some(port))?
    } else {
        fallback_web_port(c)?
    };
    url("http", override_host, published, "/").ok()
}

pub struct AuthOutput<'a> {
    pub reference: Option<&'a str>,
    pub required: bool,
    pub containers: HashSet<(String, String)>,
    pub middlewares: serde_json::Map<String, Value>,
    pub services: serde_json::Map<String, Value>,
}

impl<'a> AuthOutput<'a> {
    pub fn from_reference(reference: Option<&'a str>) -> Self {
        Self {
            reference,
            required: false,
            containers: HashSet::new(),
            middlewares: Default::default(),
            services: Default::default(),
        }
    }
}

pub fn generate(
    containers: &[Container],
    servers: &[DockerServer],
    domain: &str,
    auth: AuthOutput<'_>,
) -> anyhow::Result<Value> {
    let AuthOutput {
        reference: default_auth,
        required: auth_required,
        containers: auth_containers,
        middlewares: auth_middlewares,
        services: auth_services,
    } = auth;
    let mut routers = serde_json::Map::new();
    let mut services = auth_services;
    let mut rules = HashSet::new();
    let server_index: HashMap<_, _> = servers.iter().map(|s| (s.id.as_str(), s)).collect();
    for c in containers {
        let l = &c.labels;
        if c.state != "running"
            || !(l
                .get("traefik.enable")
                .is_some_and(|v| v.eq_ignore_ascii_case("true"))
                || l.get("swag").is_some_and(|v| v == "enable"))
            || l.get("traefik.http.disable")
                .is_some_and(|v| v.eq_ignore_ascii_case("true"))
        {
            continue;
        }
        let server = server_index
            .get(c.server_id.as_str())
            .ok_or_else(|| anyhow::anyhow!("unknown server"))?;
        let fallback_name = default_route_name(&c.name);
        let name = l
            .get("traefik.name")
            .map(String::as_str)
            .unwrap_or(&fallback_name);
        let service = l.get("traefik.service").map(String::as_str).unwrap_or(name);
        let hostname = l
            .get("traefik.hostname")
            .map(String::as_str)
            .unwrap_or(name);
        anyhow::ensure!(
            !name.is_empty()
                && !name.contains('@')
                && !service.is_empty()
                && !service.contains('@'),
            "invalid router/service name: {name}"
        );
        let rule_host = format!("{}.{}", hostname.to_lowercase(), domain.to_lowercase());
        anyhow::ensure!(
            rule_host.split('.').all(|label| !label.is_empty()
                && label.len() <= 63
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-')),
            "invalid Host rule: {rule_host}"
        );
        anyhow::ensure!(
            !routers.contains_key(name)
                && !services.contains_key(service)
                && !auth_middlewares.contains_key(name),
            "duplicate router, service, or middleware: {name}"
        );
        let explicit_port = l
            .get("traefik.webuiport")
            .map(|v| v.parse::<u16>())
            .transpose()?;
        let (upstream_host, port) = upstream_target(c, server, explicit_port).ok_or_else(|| {
            anyhow::anyhow!("missing or ambiguous published TCP port for {}", c.name)
        })?;
        let upstream = url(
            scheme(
                l.get("traefik.service.scheme")
                    .map(String::as_str)
                    .unwrap_or("http"),
            )?,
            upstream_host,
            port,
            "",
        )?;
        let is_auth_container = auth_containers.contains(&(c.server_id.clone(), c.id.clone()));
        let opted_out = l
            .get("traefik.forwardauth")
            .or_else(|| l.get("authelia_auth"))
            .is_some_and(|v| v.eq_ignore_ascii_case("false"));
        let middleware = if is_auth_container {
            vec![]
        } else if let Some(list) = l.get("traefik.middlewares") {
            list.split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>()
        } else if opted_out {
            vec![]
        } else {
            default_auth.into_iter().collect()
        };
        if middleware.is_empty()
            && auth_required
            && !is_auth_container
            && !l.contains_key("traefik.middlewares")
            && !opted_out
        {
            continue;
        }
        anyhow::ensure!(
            rules.insert(rule_host.clone()),
            "duplicate host: {rule_host}"
        );
        let entrypoints = l
            .get("traefik.entrypoints")
            .map(|v| {
                v.split(',')
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_else(|| vec!["https"]);
        routers.insert(name.into(), json!({"entryPoints": entrypoints, "rule": format!("Host(`{rule_host}`)"), "service": service, "middlewares": middleware}));
        services.insert(
            service.into(),
            json!({"loadBalancer":{"servers":[{"url":upstream}]}}),
        );
    }
    Ok(json!({"http":{"routers":routers,"services":services,"middlewares":auth_middlewares}}))
}

pub fn route_owners(containers: &[Container], config: &Value) -> Vec<(String, String, String)> {
    containers
        .iter()
        .filter_map(|c| {
            if c.state != "running"
                || c.labels
                    .get("traefik.http.disable")
                    .is_some_and(|v| v.eq_ignore_ascii_case("true"))
                || !(c
                    .labels
                    .get("traefik.enable")
                    .is_some_and(|v| v.eq_ignore_ascii_case("true"))
                    || c.labels.get("swag").is_some_and(|v| v == "enable"))
            {
                return None;
            }
            let fallback_name = default_route_name(&c.name);
            let name = c
                .labels
                .get("traefik.name")
                .map(String::as_str)
                .unwrap_or(&fallback_name);
            let service = c
                .labels
                .get("traefik.service")
                .map(String::as_str)
                .unwrap_or(name);
            config["http"]["routers"]
                .as_object()?
                .contains_key(name)
                .then(|| (name.to_owned(), service.to_owned(), c.server_id.clone()))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    fn generate(
        containers: &[Container],
        servers: &[DockerServer],
        domain: &str,
        auth: Option<&str>,
    ) -> anyhow::Result<Value> {
        super::generate(
            containers,
            servers,
            domain,
            AuthOutput::from_reference(auth),
        )
    }
    #[test]
    fn maps_unraid_port_and_keeps_api_separate() {
        let server = DockerServer {
            id: "1".into(),
            name: "host".into(),
            transport: "ssh".into(),
            endpoint: "ssh://user@remote".into(),
            tls_key: None,
            tls_cert: None,
            tls_ca: None,
            upstream_host: "10.0.0.1".into(),
            webui_host: "nas.lan".into(),
            prefer_network: false,
            docker_network: None,
            webui_same_as_server: false,
            enabled: true,
        };
        let mut c = Container {
            id: "abc".into(),
            name: "app".into(),
            image: "app:v1".into(),
            state: "running".into(),
            health: None,
            server_id: "1".into(),
            server_name: "host".into(),
            labels: HashMap::new(),
            ports: vec![Port {
                private_port: 80,
                public_port: Some(9090),
                host_ip: Some("0.0.0.0".into()),
                protocol: "tcp".into(),
            }],
            networks: vec![],
            webui_url: None,
        };
        c.labels.insert("traefik.enable".into(), "true".into());
        c.labels.insert(
            "net.unraid.docker.webui".into(),
            "https://[IP]:[PORT:80]/login".into(),
        );
        assert_eq!(
            webui_url(&c, &server).as_deref(),
            Some("https://nas.lan:9090/login")
        );
        let config = generate(
            &[c],
            [server].as_slice(),
            "example.com",
            Some("default-forward-auth@file"),
        )
        .unwrap();
        assert_eq!(
            config["http"]["services"]["App"]["loadBalancer"]["servers"][0]["url"],
            "http://10.0.0.1:9090"
        );
        assert_eq!(
            config["http"]["routers"]["App"]["middlewares"][0],
            "default-forward-auth@file"
        );
    }

    #[test]
    fn respects_auth_precedence_and_running_only() {
        let server = DockerServer {
            id: "host".into(),
            name: "docker".into(),
            transport: "unix".into(),
            endpoint: "/var/run/docker.sock".into(),
            tls_key: None,
            tls_cert: None,
            tls_ca: None,
            upstream_host: "10.10.0.2".into(),
            webui_host: "docker.lan".into(),
            prefer_network: false,
            docker_network: None,
            webui_same_as_server: false,
            enabled: true,
        };
        let mut c = Container {
            id: "one".into(),
            name: "app".into(),
            image: "app:latest".into(),
            state: "running".into(),
            health: None,
            server_id: server.id.clone(),
            server_name: server.name.clone(),
            labels: HashMap::new(),
            ports: vec![Port {
                private_port: 80,
                public_port: Some(8080),
                host_ip: Some("0.0.0.0".into()),
                protocol: "tcp".into(),
            }],
            networks: vec![],
            webui_url: None,
        };
        c.labels.insert("traefik.enable".into(), "true".into());
        c.labels
            .insert("traefik.forwardauth".into(), "false".into());
        let config = generate(
            &[c.clone()],
            std::slice::from_ref(&server),
            "example.com",
            Some("default-forward-auth@file"),
        )
        .unwrap();
        assert_eq!(config["http"]["routers"]["App"]["middlewares"], json!([]));
        c.labels
            .insert("traefik.middlewares".into(), "other@file,admin@file".into());
        let config = generate(
            &[c.clone()],
            std::slice::from_ref(&server),
            "example.com",
            Some("default-forward-auth@file"),
        )
        .unwrap();
        assert_eq!(
            config["http"]["routers"]["App"]["middlewares"],
            json!(["other@file", "admin@file"])
        );
        c.state = "exited".into();
        let config = generate(&[c], &[server], "example.com", None).unwrap();
        assert_eq!(config["http"]["routers"], json!({}));
    }

    #[test]
    fn rejects_loopback_and_ambiguous_ports() {
        let server = DockerServer {
            id: "h".into(),
            name: "h".into(),
            transport: "ssh".into(),
            endpoint: "ssh://h".into(),
            tls_key: None,
            tls_cert: None,
            tls_ca: None,
            upstream_host: "example.lan".into(),
            webui_host: "example.lan".into(),
            prefer_network: false,
            docker_network: None,
            webui_same_as_server: false,
            enabled: true,
        };
        let mut c = Container {
            id: "a".into(),
            name: "a".into(),
            image: "i".into(),
            state: "running".into(),
            health: None,
            server_id: "h".into(),
            server_name: "h".into(),
            labels: HashMap::from([("traefik.enable".into(), "true".into())]),
            ports: vec![Port {
                private_port: 80,
                public_port: Some(8080),
                host_ip: Some("127.0.0.1".into()),
                protocol: "tcp".into(),
            }],
            networks: vec![],
            webui_url: None,
        };
        assert!(webui_url(&c, &server).is_none());
        assert!(
            generate(
                &[c.clone()],
                std::slice::from_ref(&server),
                "example.com",
                None
            )
            .is_err()
        );
        c.ports[0].host_ip = Some("0.0.0.0".into());
        c.ports.push(Port {
            private_port: 443,
            public_port: Some(8443),
            host_ip: Some("0.0.0.0".into()),
            protocol: "tcp".into(),
        });
        assert!(webui_url(&c, &server).is_none());
        assert!(
            generate(
                &[c.clone()],
                std::slice::from_ref(&server),
                "example.com",
                None
            )
            .is_err()
        );
        c.labels.insert("traefik.webuiport".into(), "8443".into());
        assert!(generate(&[c], &[server], "example.com", None).is_ok());
    }

    #[test]
    fn a_published_database_or_docker_api_port_is_not_a_webui() {
        let server = DockerServer {
            id: "host".into(),
            name: "host".into(),
            transport: "http".into(),
            endpoint: "http://host:2375".into(),
            tls_key: None,
            tls_cert: None,
            tls_ca: None,
            upstream_host: "host.lan".into(),
            webui_host: "host.lan".into(),
            prefer_network: false,
            docker_network: None,
            webui_same_as_server: false,
            enabled: true,
        };
        let mut container = Container {
            id: "id".into(),
            name: "postgres".into(),
            image: "postgres".into(),
            state: "running".into(),
            health: None,
            server_id: server.id.clone(),
            server_name: server.name.clone(),
            labels: HashMap::new(),
            ports: vec![Port {
                private_port: 5432,
                public_port: Some(5433),
                host_ip: Some("0.0.0.0".into()),
                protocol: "tcp".into(),
            }],
            networks: vec![],
            webui_url: None,
        };
        assert_eq!(webui_url(&container, &server), None);
        container.ports[0].private_port = 2375;
        container.ports[0].public_port = Some(2375);
        assert_eq!(webui_url(&container, &server), None);
        container.ports[0].private_port = 80;
        container.ports[0].public_port = Some(8080);
        assert_eq!(
            webui_url(&container, &server).as_deref(),
            Some("http://host.lan:8080/")
        );
        container.ports.push(Port {
            host_ip: Some("::".into()),
            ..container.ports[0].clone()
        });
        assert_eq!(
            webui_url(&container, &server).as_deref(),
            Some("http://host.lan:8080/")
        );
    }

    #[test]
    fn webui_port_override_must_match_a_published_tcp_port() {
        let server = network_server();
        let mut container = network_container();
        container
            .labels
            .insert("traefik.webuiport".into(), "9999".into());
        assert_eq!(webui_url(&container, &server), None);
    }

    #[test]
    fn preserves_legacy_fallback_router_name_and_service() {
        let server = DockerServer {
            id: "host".into(),
            name: "host".into(),
            transport: "http".into(),
            endpoint: "http://host:2375".into(),
            tls_key: None,
            tls_cert: None,
            tls_ca: None,
            upstream_host: "host.lan".into(),
            webui_host: "host.lan".into(),
            prefer_network: false,
            docker_network: None,
            webui_same_as_server: false,
            enabled: true,
        };
        let container = Container {
            id: "id".into(),
            name: "sonarr-Anime".into(),
            image: "sonarr".into(),
            state: "running".into(),
            health: None,
            server_id: server.id.clone(),
            server_name: server.name.clone(),
            labels: HashMap::from([("traefik.enable".into(), "true".into())]),
            ports: vec![Port {
                private_port: 8989,
                public_port: Some(8990),
                host_ip: Some("0.0.0.0".into()),
                protocol: "tcp".into(),
            }],
            networks: vec![],
            webui_url: None,
        };
        let doc = generate(
            std::slice::from_ref(&container),
            &[server],
            "example.test",
            None,
        )
        .unwrap();
        assert_eq!(
            doc["http"]["routers"]["Sonarr-Anime"]["service"],
            "Sonarr-Anime"
        );
        assert_eq!(
            doc["http"]["routers"]["Sonarr-Anime"]["rule"],
            "Host(`sonarr-anime.example.test`)"
        );
        assert_eq!(route_owners(&[container], &doc)[0].0, "Sonarr-Anime");
    }

    #[test]
    fn protected_route_is_omitted_when_auth_is_missing_but_opt_out_remains() {
        let server = DockerServer {
            id: "host".into(),
            name: "host".into(),
            transport: "http".into(),
            endpoint: "http://host:2375".into(),
            tls_key: None,
            tls_cert: None,
            tls_ca: None,
            upstream_host: "host.local".into(),
            webui_host: "host.local".into(),
            prefer_network: false,
            docker_network: None,
            webui_same_as_server: false,
            enabled: true,
        };
        let mut c = Container {
            id: "a".into(),
            name: "app".into(),
            image: "app".into(),
            state: "running".into(),
            health: None,
            server_id: "host".into(),
            server_name: "host".into(),
            labels: HashMap::from([("traefik.enable".into(), "TRUE".into())]),
            ports: vec![Port {
                private_port: 80,
                public_port: Some(8080),
                host_ip: Some("0.0.0.0".into()),
                protocol: "tcp".into(),
            }],
            networks: vec![],
            webui_url: None,
        };
        let make = |containers: &[Container]| {
            super::generate(
                containers,
                std::slice::from_ref(&server),
                "example.com",
                AuthOutput {
                    reference: None,
                    required: true,
                    containers: HashSet::new(),
                    middlewares: Default::default(),
                    services: Default::default(),
                },
            )
            .unwrap()
        };
        assert!(
            make(std::slice::from_ref(&c))["http"]["routers"]
                .as_object()
                .unwrap()
                .is_empty()
        );
        c.labels
            .insert("traefik.forwardauth".into(), "FALSE".into());
        assert!(make(&[c])["http"]["routers"].get("App").is_some());
    }
    #[test]
    fn auth_container_does_not_receive_its_own_default_even_with_explicit_label() {
        let server = DockerServer {
            id: "host".into(),
            name: "host".into(),
            transport: "http".into(),
            endpoint: "http://host:2375".into(),
            tls_key: None,
            tls_cert: None,
            tls_ca: None,
            upstream_host: "host.local".into(),
            webui_host: "host.local".into(),
            prefer_network: false,
            docker_network: None,
            webui_same_as_server: false,
            enabled: true,
        };
        let c = Container {
            id: "auth".into(),
            name: "auth".into(),
            image: "authelia/authelia".into(),
            state: "running".into(),
            health: None,
            server_id: "host".into(),
            server_name: "host".into(),
            labels: HashMap::from([
                ("traefik.enable".into(), "true".into()),
                (
                    "traefik.middlewares".into(),
                    "default-forward-auth@file".into(),
                ),
            ]),
            ports: vec![Port {
                private_port: 9091,
                public_port: Some(9091),
                host_ip: Some("0.0.0.0".into()),
                protocol: "tcp".into(),
            }],
            networks: vec!["proxynet".into()],
            webui_url: None,
        };
        let doc = super::generate(
            &[c],
            &[server],
            "example.test",
            AuthOutput {
                reference: Some("default-forward-auth@file"),
                required: true,
                containers: HashSet::from([("host".into(), "auth".into())]),
                middlewares: Default::default(),
                services: Default::default(),
            },
        )
        .unwrap();
        assert_eq!(doc["http"]["routers"]["Auth"]["middlewares"], json!([]));
    }

    fn network_server() -> DockerServer {
        DockerServer {
            id: "host".into(),
            name: "host".into(),
            transport: "unix".into(),
            endpoint: "/var/run/docker.sock".into(),
            tls_key: None,
            tls_cert: None,
            tls_ca: None,
            upstream_host: "10.0.0.2".into(),
            webui_host: "docker.lan".into(),
            prefer_network: true,
            docker_network: Some("proxy".into()),
            webui_same_as_server: false,
            enabled: true,
        }
    }

    fn network_container() -> Container {
        Container {
            id: "app-id".into(),
            name: "app".into(),
            image: "app".into(),
            state: "running".into(),
            health: None,
            server_id: "host".into(),
            server_name: "host".into(),
            labels: HashMap::from([
                ("traefik.enable".into(), "true".into()),
                ("traefik.webuiport".into(), "8080".into()),
            ]),
            ports: vec![
                Port {
                    private_port: 80,
                    public_port: Some(8080),
                    host_ip: Some("0.0.0.0".into()),
                    protocol: "tcp".into(),
                },
                Port {
                    private_port: 9000,
                    public_port: Some(9000),
                    host_ip: Some("0.0.0.0".into()),
                    protocol: "tcp".into(),
                },
            ],
            networks: vec!["proxy".into()],
            webui_url: None,
        }
    }

    #[test]
    fn network_preference_uses_private_port_but_browser_keeps_published_port() {
        let server = network_server();
        let container = network_container();
        let doc = generate(
            std::slice::from_ref(&container),
            std::slice::from_ref(&server),
            "example.test",
            None,
        )
        .unwrap();
        assert_eq!(
            doc["http"]["services"]["App"]["loadBalancer"]["servers"][0]["url"],
            "http://app:80"
        );
        assert_eq!(
            webui_url(&container, &server).as_deref(),
            Some("http://docker.lan:8080/")
        );

        let mut inherited_server = server.clone();
        inherited_server.webui_same_as_server = true;
        assert_eq!(
            webui_url(&container, &inherited_server).as_deref(),
            Some("http://10.0.0.2:8080/")
        );

        let mut override_container = container;
        override_container
            .labels
            .insert("traefik.webui.host".into(), "apps.lan".into());
        assert_eq!(
            webui_url(&override_container, &server).as_deref(),
            Some("http://apps.lan:8080/")
        );
    }

    #[test]
    fn network_preference_falls_back_for_disabled_or_nonmember_container() {
        let mut server = network_server();
        let mut container = network_container();

        server.prefer_network = false;
        let doc = generate(
            &[container.clone()],
            &[server.clone()],
            "example.test",
            None,
        )
        .unwrap();
        assert_eq!(
            doc["http"]["services"]["App"]["loadBalancer"]["servers"][0]["url"],
            "http://10.0.0.2:8080"
        );

        server.prefer_network = true;
        container.networks.clear();
        let doc = generate(&[container], &[server], "example.test", None).unwrap();
        assert_eq!(
            doc["http"]["services"]["App"]["loadBalancer"]["servers"][0]["url"],
            "http://10.0.0.2:8080"
        );
    }

    #[test]
    fn network_mapping_deduplicates_bindings_and_ambiguous_private_port_falls_back() {
        let server = network_server();
        let mut container = network_container();
        container.ports.push(Port {
            host_ip: Some("::".into()),
            ..container.ports[0].clone()
        });
        let doc = generate(
            &[container.clone()],
            std::slice::from_ref(&server),
            "example.test",
            None,
        )
        .unwrap();
        assert_eq!(
            doc["http"]["services"]["App"]["loadBalancer"]["servers"][0]["url"],
            "http://app:80"
        );

        container.ports.push(Port {
            private_port: 81,
            public_port: Some(8080),
            host_ip: Some("0.0.0.0".into()),
            protocol: "tcp".into(),
        });
        let doc = generate(&[container], &[server], "example.test", None).unwrap();
        assert_eq!(
            doc["http"]["services"]["App"]["loadBalancer"]["servers"][0]["url"],
            "http://10.0.0.2:8080"
        );
    }

    #[test]
    fn network_preference_uses_loopback_published_mapping_without_override() {
        let server = network_server();
        let mut container = network_container();
        container.labels.remove("traefik.webuiport");
        container.ports.truncate(1);
        container.ports[0].host_ip = Some("127.0.0.1".into());

        let doc = generate(&[container], &[server], "example.test", None).unwrap();
        assert_eq!(
            doc["http"]["services"]["App"]["loadBalancer"]["servers"][0]["url"],
            "http://app:80"
        );
    }
}
