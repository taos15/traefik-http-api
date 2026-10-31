use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(not(feature = "server"), allow(dead_code))]
pub struct DockerServer {
    pub id: String,
    pub name: String,
    pub transport: String,
    pub endpoint: String,
    pub tls_key: Option<String>,
    pub tls_cert: Option<String>,
    pub tls_ca: Option<String>,
    pub upstream_host: String,
    pub webui_host: String,
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerInput {
    pub name: String,
    pub transport: String,
    pub endpoint: String,
    pub tls_key: Option<String>,
    pub tls_cert: Option<String>,
    pub tls_ca: Option<String>,
    pub upstream_host: String,
    pub webui_host: String,
    pub enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Port {
    pub private_port: u16,
    pub public_port: Option<u16>,
    pub host_ip: Option<String>,
    pub protocol: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Container {
    pub id: String,
    pub name: String,
    pub image: String,
    pub state: String,
    pub health: Option<String>,
    pub server_id: String,
    pub server_name: String,
    pub labels: std::collections::HashMap<String, String>,
    pub ports: Vec<Port>,
    pub webui_url: Option<String>,
}

/// Docker does not define a general container icon field. Prefer an explicit
/// application icon, then the Unraid label, then Docker Desktop extension
/// metadata when it happens to be present on a container.
pub fn container_icon_url(labels: &std::collections::HashMap<String, String>) -> Option<&str> {
    [
        "traefik.icon",
        "net.unraid.docker.icon",
        "com.docker.desktop.extension.icon",
    ]
    .into_iter()
    .filter_map(|key| labels.get(key))
    .map(String::as_str)
    .find(|value| {
        url::Url::parse(value).is_ok_and(|url| url.scheme() == "https" && url.host_str().is_some())
    })
}

#[cfg(test)]
mod icon_tests {
    use super::container_icon_url;
    use std::collections::HashMap;

    #[test]
    fn icon_priority_and_fallback() {
        let mut labels = HashMap::new();
        assert_eq!(container_icon_url(&labels), None);
        labels.insert(
            "com.docker.desktop.extension.icon".into(),
            "https://example.org/desktop.png".into(),
        );
        assert_eq!(
            container_icon_url(&labels),
            Some("https://example.org/desktop.png")
        );
        labels.insert(
            "net.unraid.docker.icon".into(),
            "https://example.org/unraid.png".into(),
        );
        labels.insert(
            "traefik.icon".into(),
            "https://example.org/custom.png".into(),
        );
        assert_eq!(
            container_icon_url(&labels),
            Some("https://example.org/custom.png")
        );
        labels.insert("traefik.icon".into(), "file:///tmp/icon.png".into());
        assert_eq!(
            container_icon_url(&labels),
            Some("https://example.org/unraid.png")
        );
    }
}

#[cfg(feature = "server")]
pub mod docker;
#[cfg(feature = "server")]
pub mod provider;
