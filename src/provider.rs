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

pub fn webui_url(c: &Container, server: &DockerServer) -> Option<String> {
    if c.state != "running" {
        return None;
    }
    let labels = &c.labels;
    let override_host = labels
        .get("traefik.webui.host")
        .map(String::as_str)
        .unwrap_or(&server.webui_host);
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
        let published = parsed.port().or_else(|| port_for(c, None, None));
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
    url("http", override_host, port_for(c, None, None)?, "/").ok()
}

pub fn generate(
    containers: &[Container],
    servers: &[DockerServer],
    domain: &str,
    default_auth: Option<&str>,
) -> anyhow::Result<Value> {
    let mut routers = serde_json::Map::new();
    let mut services = serde_json::Map::new();
    let mut rules = HashSet::new();
    let server_index: HashMap<_, _> = servers.iter().map(|s| (s.id.as_str(), s)).collect();
    for c in containers {
        let l = &c.labels;
        if c.state != "running"
            || !(l.get("traefik.enable").is_some_and(|v| v == "true")
                || l.get("swag").is_some_and(|v| v == "enable"))
            || l.get("traefik.http.disable").is_some_and(|v| v == "true")
        {
            continue;
        }
        let server = server_index
            .get(c.server_id.as_str())
            .ok_or_else(|| anyhow::anyhow!("unknown server"))?;
        let name = l.get("traefik.name").map(String::as_str).unwrap_or(&c.name);
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
            rules.insert(rule_host.clone())
                && !routers.contains_key(name)
                && !services.contains_key(service),
            "duplicate router, service, or host: {name}"
        );
        let explicit_port = l
            .get("traefik.webuiport")
            .map(|v| v.parse::<u16>())
            .transpose()?;
        let port = port_for(c, None, explicit_port).ok_or_else(|| {
            anyhow::anyhow!("missing or ambiguous published TCP port for {}", c.name)
        })?;
        let upstream = url(
            scheme(
                l.get("traefik.service.scheme")
                    .map(String::as_str)
                    .unwrap_or("http"),
            )?,
            &server.upstream_host,
            port,
            "",
        )?;
        let middleware = if let Some(list) = l.get("traefik.middlewares") {
            list.split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>()
        } else if l
            .get("traefik.forwardauth")
            .map(String::as_str)
            .or_else(|| l.get("authelia_auth").map(String::as_str))
            == Some("false")
        {
            vec![]
        } else {
            default_auth.into_iter().collect()
        };
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
    Ok(json!({"http":{"routers":routers,"services":services}}))
}

pub fn route_owners(containers: &[Container], config: &Value) -> Vec<(String, String, String)> {
    containers
        .iter()
        .filter_map(|c| {
            if c.state != "running"
                || c.labels
                    .get("traefik.http.disable")
                    .is_some_and(|v| v == "true")
                || !(c.labels.get("traefik.enable").is_some_and(|v| v == "true")
                    || c.labels.get("swag").is_some_and(|v| v == "enable"))
            {
                return None;
            }
            let name = c
                .labels
                .get("traefik.name")
                .map(String::as_str)
                .unwrap_or(&c.name);
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
            config["http"]["services"]["app"]["loadBalancer"]["servers"][0]["url"],
            "http://10.0.0.1:9090"
        );
        assert_eq!(
            config["http"]["routers"]["app"]["middlewares"][0],
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
        assert_eq!(config["http"]["routers"]["app"]["middlewares"], json!([]));
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
            config["http"]["routers"]["app"]["middlewares"],
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
}
