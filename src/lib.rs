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
    #[serde(default)]
    pub prefer_network: bool,
    pub docker_network: Option<String>,
    #[serde(default)]
    pub webui_same_as_server: bool,
    pub enabled: bool,
}

impl DockerServer {
    pub fn browser_host(&self) -> &str {
        if self.webui_same_as_server {
            &self.upstream_host
        } else {
            &self.webui_host
        }
    }
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
    #[serde(default)]
    pub prefer_network: bool,
    pub docker_network: Option<String>,
    #[serde(default)]
    pub webui_same_as_server: bool,
    pub enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ServerDiscovery {
    pub suggested_host: Option<String>,
    pub networks: Vec<String>,
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
    #[serde(default)]
    pub networks: Vec<String>,
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
    use super::{DockerServer, ServerInput, container_icon_url};
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

    #[test]
    fn legacy_server_input_json_defaults_new_preferences() {
        let input: ServerInput = serde_json::from_value(serde_json::json!({
            "name": "legacy",
            "transport": "http",
            "endpoint": "http://docker.example:2375",
            "tls_key": null,
            "tls_cert": null,
            "tls_ca": null,
            "upstream_host": "docker.example",
            "webui_host": "docker.example",
            "enabled": true
        }))
        .unwrap();

        assert!(!input.prefer_network);
        assert_eq!(input.docker_network, None);
        assert!(!input.webui_same_as_server);
    }

    #[test]
    fn legacy_docker_server_json_defaults_new_preferences() {
        let server: DockerServer = serde_json::from_value(serde_json::json!({
            "id": "legacy",
            "name": "legacy",
            "transport": "http",
            "endpoint": "http://docker.example:2375",
            "tls_key": null,
            "tls_cert": null,
            "tls_ca": null,
            "upstream_host": "docker.example",
            "webui_host": "browser.example",
            "enabled": true
        }))
        .unwrap();

        assert!(!server.prefer_network);
        assert_eq!(server.docker_network, None);
        assert!(!server.webui_same_as_server);
        assert_eq!(server.browser_host(), "browser.example");
    }
}

#[cfg(feature = "server")]
pub mod auth;
#[cfg(feature = "server")]
pub mod docker;
#[cfg(feature = "server")]
pub mod provider;
#[cfg(feature = "server")]
pub mod traefik;
