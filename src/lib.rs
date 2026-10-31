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

#[cfg(feature = "server")]
pub mod docker;
#[cfg(feature = "server")]
pub mod provider;
