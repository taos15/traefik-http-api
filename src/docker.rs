use crate::{Container, DockerServer, Port, ServerDiscovery};
use anyhow::Context;
use bollard::{API_DEFAULT_VERSION, Docker, query_parameters::ListContainersOptionsBuilder};
use futures::{StreamExt, stream};
use std::{collections::HashSet, env, path::Path, time::Duration};

const LOCAL_DOCKER_SOCKET: &str = "/var/run/docker.sock";
const LOCAL_DISCOVERY_TIMEOUT: Duration = Duration::from_secs(15);

fn connect_unix(endpoint: &str) -> anyhow::Result<Docker> {
    Ok(Docker::connect_with_socket(
        endpoint,
        10,
        API_DEFAULT_VERSION,
    )?)
}

pub fn connect(server: &DockerServer) -> anyhow::Result<Docker> {
    let timeout = 10;
    let docker = match server.transport.as_str() {
        "http" => Docker::connect_with_http(&server.endpoint, timeout, API_DEFAULT_VERSION)?,
        "ssh" => Docker::connect_with_ssh(&server.endpoint, timeout, API_DEFAULT_VERSION, None)?,
        "unix" => connect_unix(&server.endpoint)?,
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

fn local_identity(hostname: Option<String>) -> anyhow::Result<String> {
    hostname
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            anyhow::anyhow!(
                "local app container identity is unavailable because HOSTNAME is unset or empty; local discovery must run inside this app's Docker container"
            )
        })
}

fn eligible_attached_networks(
    attached: &HashSet<String>,
    networks: Vec<bollard::models::Network>,
) -> Vec<String> {
    eligible_networks(
        networks
            .into_iter()
            .filter(|network| {
                network
                    .name
                    .as_ref()
                    .is_some_and(|id| attached.contains(id))
                    || network.id.as_ref().is_some_and(|id| attached.contains(id))
            })
            .collect(),
    )
}

async fn discover_local_at(socket: &str, identity: &str) -> anyhow::Result<ServerDiscovery> {
    // This path deliberately uses only the explicit local socket. DOCKER_HOST and
    // configured DockerServer endpoints are not consulted.
    let docker = connect_unix(socket)
        .with_context(|| format!("failed to configure local Docker socket {socket}"))?;
    let docker = docker.negotiate_version().await.with_context(|| {
        format!("local Docker socket {socket} is unavailable; mount the host Docker socket at /var/run/docker.sock read-only")
    })?;
    let container = docker.inspect_container(identity, None).await.with_context(|| {
        format!("failed to inspect this app's container {identity:?} through the local Docker socket; Docker's default HOSTNAME must identify this app container")
    })?;
    let attached = container
        .network_settings
        .and_then(|settings| settings.networks)
        .map(|networks| networks.into_keys().collect::<HashSet<_>>())
        .unwrap_or_default();
    if attached.is_empty() {
        return Ok(ServerDiscovery {
            suggested_host: None,
            networks: Vec::new(),
        });
    }
    let networks = docker
        .list_networks(None)
        .await
        .with_context(|| format!("failed to list networks through local Docker socket {socket}"))?;
    Ok(ServerDiscovery {
        suggested_host: None,
        networks: eligible_attached_networks(&attached, networks),
    })
}

pub async fn discover_local() -> anyhow::Result<ServerDiscovery> {
    let identity = local_identity(env::var("HOSTNAME").ok())?;
    tokio::time::timeout(
        LOCAL_DISCOVERY_TIMEOUT,
        discover_local_at(LOCAL_DOCKER_SOCKET, &identity),
    )
    .await
    .map_err(|_| {
        anyhow::anyhow!(
            "local Docker network discovery timed out after {} seconds; verify /var/run/docker.sock and Docker's HOSTNAME container identity",
            LOCAL_DISCOVERY_TIMEOUT.as_secs()
        )
    })?
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
                webui_url_default: None,
                webui_url_override: None,
                routing: None,
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

    #[test]
    fn local_discovery_intersects_attachments_and_eligible_networks() {
        let network = |name: &str, id: &str, scope: &str, driver: &str| bollard::models::Network {
            name: Some(name.into()),
            id: Some(id.into()),
            scope: Some(scope.into()),
            driver: Some(driver.into()),
            ..Default::default()
        };
        let attached = HashSet::from([
            "zeta".to_owned(),
            "network-id".to_owned(),
            "overlay".to_owned(),
            "bridge".to_owned(),
        ]);
        assert_eq!(
            eligible_attached_networks(
                &attached,
                vec![
                    network("unattached", "other-id", "local", "bridge"),
                    network("zeta", "zeta-id", "local", "bridge"),
                    network("by-id", "network-id", "local", "bridge"),
                    network("overlay", "overlay-id", "swarm", "overlay"),
                    network("bridge", "bridge-id", "local", "bridge"),
                ],
            ),
            vec!["by-id", "zeta"]
        );
    }

    #[test]
    fn local_discovery_returns_empty_without_eligible_attachments() {
        let network = bollard::models::Network {
            name: Some("apps".into()),
            scope: Some("local".into()),
            driver: Some("bridge".into()),
            ..Default::default()
        };
        assert!(eligible_attached_networks(&HashSet::new(), vec![network]).is_empty());
    }

    #[test]
    fn local_identity_uses_only_the_default_hostname() {
        let error = local_identity(None).unwrap_err().to_string();
        assert!(error.contains("HOSTNAME"));
        assert!(local_identity(Some(String::new())).is_err());
        assert_eq!(
            local_identity(Some(" docker-container-id ".into())).unwrap(),
            "docker-container-id"
        );
    }

    #[tokio::test]
    async fn local_discovery_uses_unix_socket_wire_protocol_and_handles_missing_self() {
        use tokio::{
            io::{AsyncReadExt, AsyncWriteExt},
            net::UnixListener,
        };

        struct Cleanup(std::path::PathBuf);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }

        let unique = format!(
            "traefik-docker-mock-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let directory = Path::new("/tmp/opencode").join(unique);
        std::fs::create_dir_all(&directory).unwrap();
        let _cleanup = Cleanup(directory.clone());
        let socket = directory.join("docker.sock");
        let listener = UnixListener::bind(&socket).unwrap();
        let socket = socket.to_str().unwrap().to_owned();

        let expectations = vec![
            ("200 OK", r#"{"ApiVersion":"1.44"}"#),
            (
                "200 OK",
                r#"{
                    "Id":"self-id",
                    "NetworkSettings":{"Networks":{
                        "zeta":{"NetworkID":"zeta-id"},
                        "by-id":{"NetworkID":"network-id"},
                        "bridge":{"NetworkID":"bridge-id"},
                        "overlay":{"NetworkID":"overlay-id"}
                    }}
                }"#,
            ),
            (
                "200 OK",
                r#"[
                    {"Name":"unattached","Id":"other-id","Scope":"local","Driver":"bridge"},
                    {"Name":"zeta","Id":"zeta-id","Scope":"local","Driver":"bridge"},
                    {"Name":"by-id","Id":"network-id","Scope":"local","Driver":"bridge"},
                    {"Name":"bridge","Id":"bridge-id","Scope":"local","Driver":"bridge"},
                    {"Name":"overlay","Id":"overlay-id","Scope":"swarm","Driver":"overlay"}
                ]"#,
            ),
            ("200 OK", r#"{"ApiVersion":"1.44"}"#),
            (
                "404 Not Found",
                r#"{"message":"No such container: missing"}"#,
            ),
        ];
        let server = tokio::spawn(async move {
            let mut paths = Vec::new();
            for (status, body) in expectations {
                let (mut stream, _) =
                    tokio::time::timeout(Duration::from_secs(3), listener.accept())
                        .await
                        .unwrap()
                        .unwrap();
                let request = tokio::time::timeout(Duration::from_secs(2), async {
                    let mut request = Vec::new();
                    let mut buffer = [0; 1024];
                    loop {
                        let read = stream.read(&mut buffer).await.unwrap();
                        assert!(read > 0, "connection closed before request headers");
                        request.extend_from_slice(&buffer[..read]);
                        assert!(request.len() <= 16 * 1024, "request headers too large");
                        if request.windows(4).any(|window| window == b"\r\n\r\n") {
                            break;
                        }
                    }
                    String::from_utf8(request).unwrap()
                })
                .await
                .unwrap();
                paths.push(
                    request
                        .lines()
                        .next()
                        .and_then(|line| line.split_whitespace().nth(1))
                        .unwrap()
                        .to_owned(),
                );
                let response = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                tokio::time::timeout(
                    Duration::from_secs(2),
                    stream.write_all(response.as_bytes()),
                )
                .await
                .unwrap()
                .unwrap();
            }
            paths
        });

        let discovered = tokio::time::timeout(
            Duration::from_secs(5),
            discover_local_at(&socket, "self-id"),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(discovered.suggested_host, None);
        assert_eq!(discovered.networks, vec!["by-id", "zeta"]);

        let error = tokio::time::timeout(
            Duration::from_secs(5),
            discover_local_at(&socket, "missing"),
        )
        .await
        .unwrap()
        .unwrap_err()
        .to_string();
        assert!(error.contains("HOSTNAME"), "unexpected error: {error}");
        assert!(error.contains("missing"), "unexpected error: {error}");

        let paths = tokio::time::timeout(Duration::from_secs(3), server)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            paths,
            vec![
                "/version",
                "/containers/self-id/json",
                "/networks",
                "/version",
                "/containers/missing/json",
            ]
        );
    }
}
