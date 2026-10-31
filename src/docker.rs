use crate::{Container, DockerServer, Port};
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
