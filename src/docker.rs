use crate::{Container, DockerServer, Port, ServerDiscovery};
use bollard::{API_DEFAULT_VERSION, Docker, query_parameters::ListContainersOptionsBuilder};
use futures::{StreamExt, stream};
use std::{path::Path, time::Duration};

pub fn connect(server: &DockerServer) -> anyhow::Result<Docker> {
    let timeout = 10;
    let docker = match server.transport.as_str() {
        "http" => Docker::connect_with_http(&server.endpoint, timeout, API_DEFAULT_VERSION)?,
        "ssh" => Docker::connect_with_ssh(&server.endpoint, timeout, API_DEFAULT_VERSION, None)?,
        "unix" => Docker::connect_with_socket(&server.endpoint, timeout, API_DEFAULT_VERSION)?,
        "tls" => Docker::connect_with_ssl(
            &server.endpoint,
            Path::new(
                server
                    .tls_key
                    .as_deref()
                    .ok_or_else(|| anyhow::anyhow!("missing TLS key"))?,
            ),
            Path::new(
                server
                    .tls_cert
                    .as_deref()
                    .ok_or_else(|| anyhow::anyhow!("missing TLS cert"))?,
            ),
            Path::new(
                server
                    .tls_ca
                    .as_deref()
                    .ok_or_else(|| anyhow::anyhow!("missing TLS CA"))?,
            ),
            timeout,
            API_DEFAULT_VERSION,
        )?,
        other => anyhow::bail!("unsupported transport: {other}"),
    };
    Ok(docker)
}

pub fn endpoint_host(transport: &str, endpoint: &str) -> Option<String> {
    if !matches!(transport, "http" | "tls" | "ssh") {
        return None;
    }
    url::Url::parse(endpoint)
        .ok()?
        .host_str()
        .map(ToOwned::to_owned)
}

pub fn eligible_networks(networks: Vec<bollard::models::Network>) -> Vec<String> {
    let mut names = networks
        .into_iter()
        .filter(|network| {
            network.scope.as_deref() == Some("local")
                && network.driver.as_deref() == Some("bridge")
                && !network.config_only.unwrap_or(false)
        })
        .filter_map(|network| network.name)
        .filter(|name| !matches!(name.as_str(), "bridge" | "host" | "none"))
        .collect::<Vec<_>>();
    names.sort();
    names.dedup();
    names
}

pub async fn discover(server: &DockerServer) -> anyhow::Result<ServerDiscovery> {
    let docker = tokio::time::timeout(
        Duration::from_secs(12),
        connect(server)?.negotiate_version(),
    )
    .await??;
    let networks =
        tokio::time::timeout(Duration::from_secs(12), docker.list_networks(None)).await??;
    let info = tokio::time::timeout(Duration::from_secs(8), docker.info())
        .await
        .ok()
        .and_then(Result::ok);
    let suggested_host = endpoint_host(&server.transport, &server.endpoint).or_else(|| {
        info.and_then(|info| {
            info.swarm
                .and_then(|swarm| swarm.node_addr)
                .filter(|host| !host.trim().is_empty() && host != "0.0.0.0" && host != "::")
                .or_else(|| info.name.filter(|name| !name.trim().is_empty()))
        })
    });
    Ok(ServerDiscovery {
        suggested_host,
        networks: eligible_networks(networks),
    })
}

pub async fn list(server: DockerServer) -> anyhow::Result<Vec<Container>> {
    let docker = tokio::time::timeout(
        Duration::from_secs(12),
        connect(&server)?.negotiate_version(),
    )
    .await??;
    let entries = tokio::time::timeout(
        Duration::from_secs(12),
        docker.list_containers(Some(
            ListContainersOptionsBuilder::default().all(true).build(),
        )),
    )
    .await??;
    Ok(entries
        .into_iter()
        .map(|entry| {
            let ports = entry
                .ports
                .unwrap_or_default()
                .into_iter()
                .map(|p| Port {
                    private_port: p.private_port,
                    public_port: p.public_port,
                    host_ip: p.ip,
                    protocol: p.typ.map(|t| t.to_string()).unwrap_or_default(),
                })
                .collect();
            let name = entry
                .names
                .unwrap_or_default()
                .first()
                .cloned()
                .unwrap_or_default()
                .trim_start_matches('/')
                .to_owned();
            Container {
                id: entry.id.unwrap_or_default(),
                name,
                image: entry.image.unwrap_or_default(),
                state: entry.state.map(|s| s.to_string()).unwrap_or_default(),
                health: entry.health.and_then(|h| h.status.map(|s| s.to_string())),
                server_id: server.id.clone(),
                server_name: server.name.clone(),
                labels: entry.labels.unwrap_or_default(),
                ports,
                networks: entry
                    .network_settings
                    .and_then(|n| n.networks)
                    .map(|n| n.into_keys().collect())
                    .unwrap_or_default(),
                webui_url: None,
            }
        })
        .collect())
}

pub async fn aggregate(servers: Vec<DockerServer>) -> anyhow::Result<Vec<Container>> {
    let results = stream::iter(servers.into_iter().filter(|s| s.enabled).map(list))
        .buffer_unordered(8)
        .collect::<Vec<_>>()
        .await;
    results
        .into_iter()
        .collect::<anyhow::Result<Vec<_>>>()
        .map(|v| v.into_iter().flatten().collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoint_host_strips_credentials_and_port() {
        assert_eq!(
            endpoint_host("ssh", "ssh://user:secret@example.test:22"),
            Some("example.test".into())
        );
        assert_eq!(endpoint_host("unix", "/var/run/docker.sock"), None);
    }

    #[test]
    fn eligible_networks_only_returns_user_defined_local_bridges() {
        let network = |name: &str, scope: &str, driver: &str| bollard::models::Network {
            name: Some(name.into()),
            scope: Some(scope.into()),
            driver: Some(driver.into()),
            ..Default::default()
        };
        assert_eq!(
            eligible_networks(vec![
                network("bridge", "local", "bridge"),
                network("apps", "local", "bridge"),
                network("swarm", "swarm", "overlay"),
                network("apps", "local", "bridge"),
            ]),
            vec!["apps"]
        );
    }
}
