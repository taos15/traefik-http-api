use dioxus::prelude::*;
use serde::Deserialize;
use traefik_http_api::{Container, DockerServer, ServerDiscovery, ServerInput, container_icon_url};

#[derive(Deserialize)]
struct ErrorMessage {
    error: String,
}

async fn request<T: serde::de::DeserializeOwned>(path: &str) -> Result<T, String> {
    let response = gloo_net::http::Request::get(path)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !response.ok() {
        return Err(response
            .json::<ErrorMessage>()
            .await
            .map(|v| v.error)
            .unwrap_or_else(|_| "request failed".into()));
    }
    response.json().await.map_err(|e| e.to_string())
}
async fn mutate(method: &str, path: &str, body: Option<String>) -> Result<(), String> {
    let request = match method {
        "POST" => gloo_net::http::Request::post(path),
        "DELETE" => gloo_net::http::Request::delete(path),
        "PUT" => gloo_net::http::Request::put(path),
        _ => return Err("invalid method".into()),
    };
    let request = request
        .header("X-Requested-With", "traefik-web")
        .header("Content-Type", "application/json")
        .body(body.unwrap_or_default())
        .map_err(|e| e.to_string())?;
    let response = request.send().await.map_err(|e| e.to_string())?;
    if !response.ok() {
        return Err(response
            .json::<ErrorMessage>()
            .await
            .map(|v| v.error)
            .unwrap_or_else(|_| "request failed".into()));
    }
    Ok(())
}

fn main() {
    dioxus::launch(App);
}

#[component]
fn App() -> Element {
    let mut section = use_signal(|| "servers");
    let mut revision = use_signal(|| 0_u64);
    let mut container_filter = use_signal(|| "all");
    let mut container_server = use_signal(|| "all".to_string());
    let mut container_search = use_signal(String::new);
    let message = use_signal(String::new);
    let servers = use_resource(move || {
        let _ = revision();
        async move { request::<Vec<DockerServer>>("/api/v1/servers").await }
    });
    let containers = use_resource(move || {
        let _ = revision();
        async move { request::<Vec<Container>>("/api/v1/containers").await }
    });
    rsx! {
        document::Stylesheet {href:"/app.css"}
        header { h1 {"Traefik · Docker Hosts"}
            nav {
                button {onclick:move |_|section.set("servers"),"Servers"}
                button {onclick:move |_|section.set("containers"),"Containers"}
                button {onclick:move |_|section.set("provider"),"Provider"}
                button {onclick:move |_|async move{let _=mutate("POST","/auth/logout",None).await;let _=web_sys::window().unwrap().location().set_href("/login.html");},"Log out"}
            }
        }
        main {
            if !message().is_empty(){p {class:"message","{message}"}}
            if section()=="servers" {
                ServerForm {revision,message}
                h2 {"Docker hosts"}
                match &*servers.read() {
                    Some(Ok(list))=>rsx! {div {class:"grid",for server in list.iter() { ServerCard {server:server.clone(),revision,message} }}},
                    Some(Err(e))=>rsx!{p {class:"error","{e}"}},
                    None=>rsx!{p {"Loading servers…"}}
                }
            } else if section()=="containers" {
                h2 {"Containers"}
                match &*containers.read() {
                    Some(Ok(list))=>{
                        let non_running=list.iter().filter(|c|c.state!="running").count();
                        let search=container_search().to_lowercase();
                        let mut visible=list.iter().filter(|c|{
                            (container_filter()=="all" || (container_filter()=="non-running" && c.state!="running") || (container_filter()=="running" && c.state=="running"))
                            && (container_server()=="all" || c.server_id==container_server())
                            && (search.is_empty() || c.name.to_lowercase().contains(&search) || c.image.to_lowercase().contains(&search))
                        }).cloned().collect::<Vec<_>>();
                        visible.sort_by(|a,b| (a.state=="running",a.server_name.as_str(),a.name.to_lowercase()).cmp(&(b.state=="running",b.server_name.as_str(),b.name.to_lowercase())));
                        rsx!{
                            p {"Stopped and created containers are available here to start. Only running containers receive Traefik routes."}
                            div {class:"inventory-toolbar",
                                div {class:"inventory-filters",
                                    button {class:if container_filter()=="all"{"selected"}else{""},onclick:move |_|container_filter.set("all"),"All ({list.len()})"}
                                    button {class:if container_filter()=="non-running"{"selected"}else{""},onclick:move |_|container_filter.set("non-running"),"Non-running ({non_running})"}
                                    button {class:if container_filter()=="running"{"selected"}else{""},onclick:move |_|container_filter.set("running"),"Running ({list.len()-non_running})"}
                                }
                                label {"Docker host" select {value:"{container_server}",onchange:move |e|container_server.set(e.value()),
                                    option {value:"all","All hosts"}
                                    if let Some(Ok(hosts))=&*servers.read(){for host in hosts.iter(){option {value:"{host.id}","{host.name}"}}}
                                }}
                                label {"Search" input {r#type:"search",placeholder:"Name or image",value:"{container_search}",oninput:move |e|container_search.set(e.value())}}
                                button {onclick:move |_|revision+=1,"Refresh"}
                            }
                            if visible.is_empty(){p {"No containers match these filters."}}
                            div {class:"grid",for container in visible {ContainerCard {key:"{container.server_id}:{container.id}",container,revision,message}}}
                        }
                    },
                    Some(Err(e))=>rsx!{p {class:"error","{e}"}},
                    None=>rsx!{p {"Loading containers…"}}
                }
            } else {
                h2 {"Traefik provider"}
                p {"Traefik polls /api/v1/traefikconfig. The endpoint is anonymous unless a provider token is configured."}
                ProviderStatus {revision,message}
                ProviderPreview {}
            }
        }
    }
}

#[derive(Deserialize, Clone)]
struct Status {
    refreshed_at: Option<i64>,
    auth: Option<AuthStatus>,
    api_error: Option<String>,
    last_error: Option<String>,
    route_warnings: Vec<String>,
}
#[derive(Deserialize, Clone)]
struct AuthStatus {
    reference: Option<String>,
    note: String,
    candidates: Vec<AuthCandidate>,
    protected: bool,
    api_reachable: bool,
    credentials_configured: bool,
}
#[derive(Deserialize, Clone)]
struct AuthCandidate {
    provider: String,
    container_id: String,
    server_id: String,
    name: String,
    eligible: bool,
    published_url: Option<String>,
}
#[derive(Deserialize, Clone)]
struct SnapshotInfo {
    id: i64,
    verified: bool,
    created_at: i64,
    redacted_count: usize,
}
#[derive(Deserialize)]
struct Snippet {
    yaml: String,
    warnings: Vec<String>,
}
#[component]
fn ProviderStatus(revision: Signal<u64>, message: Signal<String>) -> Element {
    let status = use_resource(move || {
        let _ = revision();
        async { request::<Status>("/api/v1/provider/status").await }
    });
    let snapshots = use_resource(move || {
        let _ = revision();
        async { request::<Vec<SnapshotInfo>>("/api/v1/provider/files").await }
    });
    let mut snippet = use_signal(String::new);
    rsx! {
        match &*status.read() {
            Some(Ok(s)) => rsx! {
                p { "Last successful refresh (Unix time): {s.refreshed_at:?}" }
                if let Some(a) = &s.auth {
                    p { "Traefik API reachable: {a.api_reachable} · credentials configured: {a.credentials_configured}" }
                    p { "Auth: {a.note}" }
                    if a.reference.is_none() {
                        p { class: "error", if a.protected { "Required auth unavailable: protected routes are omitted." } else { "Automatic ForwardAuth off; routes without explicit middleware are unprotected." } }
                    }
                    button {
                        onclick: move |_| async move {
                            match mutate("POST", "/api/v1/provider/auth/reset", None).await {
                                Ok(()) => { message.set("Auth selection reset".into()); revision += 1; },
                                Err(e) => message.set(e),
                            }
                        },
                        "Reset automatic auth selection"
                    }
                    h3 { "Detected auth containers" }
                    for c in &a.candidates {
                        article { class: "panel",
                            p { "{c.provider}: {c.name} · " if c.eligible { "On Traefik's network" } else { "On another Docker network/host" } }
                            if !c.eligible && a.reference.as_deref() != Some("default-forward-auth@file") {
                                if c.published_url.is_some() {
                                    button {
                                        onclick: {
                                            let server = c.server_id.clone();
                                            let id = c.container_id.clone();
                                            move |_| {
                                                let server = server.clone();
                                                let id = id.clone();
                                                async move {
                                                    let path = format!("/api/v1/provider/auth/snippet?server_id={server}&container_id={id}");
                                                    match request::<Snippet>(&path).await {
                                                        Ok(v) => snippet.set(format!("{}\n{}", v.warnings.join("\n"), v.yaml)),
                                                        Err(e) => message.set(e),
                                                    }
                                                }
                                            }
                                        },
                                        "Generate @file snippet"
                                    }
                                } else {
                                    p { "Publish the auth container port to a Traefik-reachable host or define its address in a Traefik file." }
                                }
                            }
                        }
                    }
                    if !snippet().is_empty() {
                        details { open: true, summary { "Snippet — copy into your Traefik dynamic directory" }
                            button { onclick: move |_| {if let Some(window)=web_sys::window(){let _=window.navigator().clipboard().write_text(&snippet());}}, "Copy snippet" }
                            a { href:"data:application/yaml,{url::form_urlencoded::byte_serialize(snippet().as_bytes()).collect::<String>()}",download:"default-forward-auth.yml","Download snippet" }
                            pre { "{snippet}" }
                        }
                    }
                } else {
                    p { "Waiting for first inventory refresh…" }
                }
                if let Some(error) = &s.api_error { p { class: "error", "Traefik API: {error}" } }
                if let Some(error) = &s.last_error { p { class: "error", "Last refresh failed: {error}. Serving last good snapshot." } }
                if !s.route_warnings.is_empty() {details {summary {"Route warnings ({s.route_warnings.len()})"} for warning in &s.route_warnings {p {class:"error","{warning}"}}}}
            },
            Some(Err(e)) => rsx! { p { class: "error", "{e}" } },
            None => rsx! { p { "Loading status…" } },
        }
        h3 { "File-provider snapshots" }
        p { "Effective config only; generated YAML is redacted and may need manual repair before use." }
        match &*snapshots.read() {
            Some(Ok(list)) => rsx! {
                div { class: "panel",
                    a { href: "/api/v1/provider/files/export", download: "traefik-file-snapshot.yml", "Generate config (latest verified)" }
                    for s in list {
                        p { "Snapshot #{s.id} · time {s.created_at} · verified: {s.verified} · redacted fields: {s.redacted_count} "
                            a { href: "/api/v1/provider/files/export?id={s.id}", download: "traefik-file-snapshot.yml", "Download" }
                        }
                    }
                }
            },
            Some(Err(e)) => rsx! { p { class: "error", "{e}" } },
            None => rsx! { p { "Loading file snapshots…" } },
        }
    }
}

#[component]
fn ProviderPreview() -> Element {
    let preview =
        use_resource(|| async { request::<serde_json::Value>("/api/v1/provider/preview").await });
    rsx! {details {summary {"Preview last published Traefik config"}
        match &*preview.read(){
            Some(Ok(doc))=>rsx!{pre {"{serde_json::to_string_pretty(doc).unwrap_or_default()}"}},
            Some(Err(e))=>rsx!{p {class:"error","{e}"}},
            None=>rsx!{p {"Loading preview…"}}
        }
    }}
}

/// Path of the app's own Docker networks, read from its local Docker socket.
const LOCAL_NETWORKS_PATH: &str = "/api/v1/networks/local";

/// State of the local network list shared by the add and edit server forms.
///
/// `generation` moves forward whenever a new request starts, so only the newest
/// request may apply its result. Older responses are ignored and never block a
/// newer one. Server connection fields don't affect this list.
#[derive(Clone, Debug, Default, PartialEq)]
struct NetworkDiscovery {
    generation: u64,
    status: DiscoveryStatus,
}

#[derive(Clone, Debug, Default, PartialEq)]
enum DiscoveryStatus {
    #[default]
    NotLoaded,
    Loading,
    Failed(String),
    Loaded(Vec<String>),
}

impl NetworkDiscovery {
    /// Start a request and return the ticket its response must present.
    fn begin(&mut self) -> u64 {
        self.generation = self.generation.wrapping_add(1);
        self.status = DiscoveryStatus::Loading;
        self.generation
    }

    /// Apply a response if it belongs to the newest request. Returns whether it was applied.
    fn finish(&mut self, ticket: u64, result: Result<ServerDiscovery, String>) -> bool {
        if ticket != self.generation {
            return false;
        }
        self.status = match result {
            Ok(found) => DiscoveryStatus::Loaded(found.networks),
            Err(error) => DiscoveryStatus::Failed(error),
        };
        true
    }

    /// Checking the box only fetches when nothing is loaded or loading yet.
    fn should_auto_load(&self) -> bool {
        matches!(
            self.status,
            DiscoveryStatus::NotLoaded | DiscoveryStatus::Failed(_)
        )
    }
}

#[derive(Debug, PartialEq)]
enum PickerState<'a> {
    NotLoaded,
    Loading,
    Failed(&'a str),
    Empty,
    Loaded(usize),
}

fn picker_state(status: &DiscoveryStatus) -> PickerState<'_> {
    match status {
        DiscoveryStatus::NotLoaded => PickerState::NotLoaded,
        DiscoveryStatus::Loading => PickerState::Loading,
        DiscoveryStatus::Failed(error) => PickerState::Failed(error),
        DiscoveryStatus::Loaded(list) if list.is_empty() => PickerState::Empty,
        DiscoveryStatus::Loaded(list) => PickerState::Loaded(list.len()),
    }
}

/// A selection is reported missing only after the list loaded successfully.
fn saved_network_missing(status: &DiscoveryStatus, selected: &str) -> bool {
    matches!(status, DiscoveryStatus::Loaded(list) if !selected.is_empty() && !list.iter().any(|n| n == selected))
}

/// Dropdown options as (value, label). The current selection is always kept as an
/// option so loading, errors, or missing results never drop it.
fn network_options(status: &DiscoveryStatus, selected: &str) -> Vec<(String, String)> {
    let loaded = match status {
        DiscoveryStatus::Loaded(list) => list.as_slice(),
        _ => &[],
    };
    let mut options = Vec::with_capacity(loaded.len() + 1);
    if !selected.is_empty() && !loaded.iter().any(|n| n == selected) {
        let label = if saved_network_missing(status, selected) {
            format!("{selected} (not attached to this app)")
        } else {
            selected.to_owned()
        };
        options.push((selected.to_owned(), label));
    }
    options.extend(loaded.iter().map(|n| (n.clone(), n.clone())));
    options
}

/// Short, single-line error for the local network lookup. HTML is stripped,
/// whitespace collapsed and the text capped; a denied request is described as
/// the app's local Docker socket refusing access.
fn local_network_error(error: &str) -> String {
    const LIMIT: usize = 160;
    let mut text = String::with_capacity(error.len());
    let mut in_tag = false;
    for ch in error.chars() {
        match ch {
            '<' => {
                in_tag = true;
                text.push(' ');
            }
            '>' if in_tag => in_tag = false,
            _ if !in_tag => text.push(ch),
            _ => {}
        }
    }
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let lower = text.to_ascii_lowercase();
    if lower.contains("forbidden") || lower.contains("status code 403") {
        return "the local Docker socket denied access (403 Forbidden).".into();
    }
    if text.is_empty() {
        return "request failed".into();
    }
    if text.chars().count() > LIMIT {
        let cut: String = text.chars().take(LIMIT).collect();
        return format!("{}…", cut.trim_end());
    }
    text
}

/// Blocks saving "prefer network" without a network, matching the backend rule.
fn network_submit_error(prefer_network: bool, selected: &str) -> Option<&'static str> {
    (prefer_network && selected.trim().is_empty())
        .then_some("Select a Docker network, or turn off Prefer Docker network.")
}

async fn run_local_networks(ticket: u64, mut discovery: Signal<NetworkDiscovery>) {
    let result = request::<ServerDiscovery>(LOCAL_NETWORKS_PATH).await;
    discovery.write().finish(ticket, result);
}

fn load_local_networks(mut discovery: Signal<NetworkDiscovery>) {
    let ticket = discovery.write().begin();
    spawn(run_local_networks(ticket, discovery));
}

#[component]
fn DockerNetworkField(
    discovery: Signal<NetworkDiscovery>,
    docker_network: Signal<String>,
) -> Element {
    let status = discovery.read().status.clone();
    let selected = docker_network();
    let options = network_options(&status, &selected);
    let missing = saved_network_missing(&status, &selected);
    rsx! {div {class:"network-picker",
        label {"Docker network"
            select {required:true,value:"{selected}",onchange:move |e|docker_network.set(e.value()),
                option {value:"",disabled:true,selected:selected.is_empty(),"Select a network"}
                for (value,label) in options {option {key:"{value}",value:"{value}",selected:value==selected,"{label}"}}
            }
        }
        small {class:"network-help","Lists the custom bridge networks this app's container is attached to. Traefik must also be on the network you choose."}
        div {class:"network-status",role:"status","aria-live":"polite",
            match picker_state(&status) {
                PickerState::NotLoaded=>rsx!{
                    small {"Networks aren't loaded yet."}
                    button {r#type:"button",class:"secondary",onclick:move |_|load_local_networks(discovery),"Load networks"}
                },
                PickerState::Loading=>rsx!{span {class:"spinner","aria-hidden":"true"} small {"Loading networks…"}},
                PickerState::Failed(error)=>rsx!{
                    small {class:"error","Couldn't read this app's networks from its local Docker socket: {local_network_error(error)}"}
                    button {r#type:"button",class:"secondary",onclick:move |_|load_local_networks(discovery),"Retry"}
                },
                PickerState::Empty=>rsx!{
                    small {"This app's container isn't attached to any custom bridge networks. Attach it to one Traefik also uses, then refresh."}
                    button {r#type:"button",class:"secondary",onclick:move |_|load_local_networks(discovery),"Refresh networks"}
                },
                PickerState::Loaded(count)=>rsx!{
                    small {if count==1 {"1 network found."} else {"{count} networks found."}}
                    button {r#type:"button",class:"secondary",onclick:move |_|load_local_networks(discovery),"Refresh networks"}
                },
            }
        }
        if missing {p {class:"error network-warning","“{selected}” isn't attached to this app's container. Pick another network or refresh."}}
    }}
}

#[component]
fn ServerForm(revision: Signal<u64>, message: Signal<String>) -> Element {
    let mut name = use_signal(String::new);
    let mut endpoint = use_signal(String::new);
    let mut transport = use_signal(|| "http".to_owned());
    let mut upstream = use_signal(String::new);
    let mut browser = use_signal(String::new);
    let mut key = use_signal(String::new);
    let mut cert = use_signal(String::new);
    let mut ca = use_signal(String::new);
    let mut enabled = use_signal(|| true);
    let mut prefer_network = use_signal(|| false);
    let docker_network = use_signal(String::new);
    let mut webui_same_as_server = use_signal(|| true);
    let discovery = use_signal(NetworkDiscovery::default);
    let make_input = move || ServerInput {
        name: name(),
        transport: transport(),
        endpoint: endpoint(),
        tls_key: (!key().is_empty()).then(|| key()),
        tls_cert: (!cert().is_empty()).then(|| cert()),
        tls_ca: (!ca().is_empty()).then(|| ca()),
        upstream_host: upstream(),
        webui_host: if webui_same_as_server() {
            upstream()
        } else {
            browser()
        },
        enabled: enabled(),
        prefer_network: prefer_network(),
        docker_network: prefer_network()
            .then(|| docker_network())
            .filter(|v| !v.is_empty()),
        webui_same_as_server: webui_same_as_server(),
    };
    rsx! {section {class:"panel",h2 {"Add Docker host"}
        p {"Docker API access, Traefik upstream, and browser WebUI address are separate."}
        form {onsubmit:move |e|{e.prevent_default();let input=make_input();async move{
            if let Some(error)=network_submit_error(input.prefer_network,input.docker_network.as_deref().unwrap_or_default()){message.set(error.into());return;}
            let result=serde_json::to_string(&input).map_err(|e|e.to_string()).and_then(|v|Ok(v));
            let result=match result {Ok(v)=>mutate("POST","/api/v1/servers",Some(v)).await,Err(e)=>Err(e)};
            match result {Ok(())=>{message.set("Server added".into());revision+=1;},Err(e)=>message.set(e)}
        }},
            label {"Name" input {required:true,value:"{name}",oninput:move |e|name.set(e.value())}}
            label {"Transport" select {value:"{transport}",onchange:move |e|transport.set(e.value()),
                option {value:"http","HTTP socket proxy"} option {value:"tls","Mutual TLS"} option {value:"ssh","SSH"} option {value:"unix","Unix socket"}
            }}
            label {"Docker API endpoint (http://host:2375, ssh://user@host, or /var/run/docker.sock)" input {required:true,value:"{endpoint}",oninput:move |e|endpoint.set(e.value())}}
            if transport()=="tls" {
                label {"Client key path" input {value:"{key}",oninput:move |e|key.set(e.value())}}
                label {"Client cert path" input {value:"{cert}",oninput:move |e|cert.set(e.value())}}
                label {"CA path" input {value:"{ca}",oninput:move |e|ca.set(e.value())}}
            }
            label {"Traefik-reachable host" input {required:true,value:"{upstream}",oninput:move |e|upstream.set(e.value())}
                small {"Server host used when connecting through published ports."}
            }
            label {input {r#type:"checkbox",checked:prefer_network(),onchange:move |e|{
                let checked=e.checked();prefer_network.set(checked);
                if checked && discovery.peek().should_auto_load(){load_local_networks(discovery);}
            }} "Prefer Docker network"}
            if prefer_network(){
                DockerNetworkField {discovery,docker_network}
            }
            label {input {r#type:"checkbox",checked:webui_same_as_server(),onchange:move |e|webui_same_as_server.set(e.checked())} "Same as server host"}
            label {"Browser-reachable host" input {required:!webui_same_as_server(),disabled:webui_same_as_server(),value:if webui_same_as_server(){upstream()}else{browser()},oninput:move |e|browser.set(e.value())}
                small {"Used with the container's published external WebUI port."}
            }
            label {"Enabled" input {r#type:"checkbox",checked:enabled(),onchange:move |e|enabled.set(e.checked())}}
            div {class:"actions",button {r#type:"submit","Add server"}
                button {r#type:"button",onclick:move |_|{let input=make_input();async move{
                    let body=serde_json::to_string(&input).unwrap_or_default();
                    match mutate("POST","/api/v1/servers/test",Some(body)).await{Ok(())=>message.set("Connection succeeded".into()),Err(e)=>message.set(e)}
                }},"Test connection"}
            }
        }
    }}
}

#[component]
fn ServerCard(server: DockerServer, revision: Signal<u64>, message: Signal<String>) -> Element {
    let id = server.id.clone();
    let mut editing = use_signal(|| false);
    let network_summary = if server.prefer_network {
        format!(
            "Docker network preferred: {}",
            server.docker_network.as_deref().unwrap_or("not selected")
        )
    } else {
        "Published ports preferred".to_string()
    };
    rsx! {article {class:"panel",h3 {"{server.name}"} p {"{server.transport} · {server.endpoint}"}
        p {"Server host: {server.upstream_host} · Browser host: " if server.webui_same_as_server {"{server.upstream_host} (same as server)"} else {"{server.webui_host}"}}
        p {"{network_summary}"}
        p {"Enabled: {server.enabled}"}
        button {onclick:move |_|editing.set(!editing()),if editing(){"Cancel edit"}else{"Edit"}}
        if editing(){ EditServerForm {server:server.clone(),revision,message} }
        button {onclick:move |_|{let id=id.clone();async move{
            match mutate("DELETE",&format!("/api/v1/servers/{id}"),None).await{Ok(())=>{message.set("Server deleted".into());revision+=1;},Err(e)=>message.set(e)}
        }},"Delete"}
    }}
}

#[component]
fn EditServerForm(server: DockerServer, revision: Signal<u64>, message: Signal<String>) -> Element {
    let mut name = use_signal(|| server.name.clone());
    let mut endpoint = use_signal(|| server.endpoint.clone());
    let mut transport = use_signal(|| server.transport.clone());
    let mut upstream = use_signal(|| server.upstream_host.clone());
    let mut browser = use_signal(|| server.webui_host.clone());
    let mut enabled = use_signal(|| server.enabled);
    let mut key = use_signal(|| server.tls_key.clone().unwrap_or_default());
    let mut cert = use_signal(|| server.tls_cert.clone().unwrap_or_default());
    let mut ca = use_signal(|| server.tls_ca.clone().unwrap_or_default());
    let mut prefer_network = use_signal(|| server.prefer_network);
    let docker_network = use_signal(|| server.docker_network.clone().unwrap_or_default());
    let mut webui_same_as_server = use_signal(|| server.webui_same_as_server);
    // Opening edit with the preference already on loads the app's networks once.
    // The loading state is set up front so nothing writes to a signal during render.
    let auto_load = server.prefer_network;
    let discovery = use_signal(|| {
        let mut state = NetworkDiscovery::default();
        if auto_load {
            state.begin();
        }
        state
    });
    let id = server.id.clone();
    let make_input = move || ServerInput {
        name: name(),
        endpoint: endpoint(),
        transport: transport(),
        upstream_host: upstream(),
        webui_host: if webui_same_as_server() {
            upstream()
        } else {
            browser()
        },
        enabled: enabled(),
        tls_key: (!key().is_empty()).then(|| key()),
        tls_cert: (!cert().is_empty()).then(|| cert()),
        tls_ca: (!ca().is_empty()).then(|| ca()),
        prefer_network: prefer_network(),
        docker_network: prefer_network()
            .then(|| docker_network())
            .filter(|v| !v.is_empty()),
        webui_same_as_server: webui_same_as_server(),
    };
    use_hook(move || {
        if auto_load {
            let ticket = discovery.peek().generation;
            spawn(run_local_networks(ticket, discovery));
        }
    });
    rsx! {form {onsubmit:move |e|{e.prevent_default();let id=id.clone();let input=make_input();
        async move{
            if let Some(error)=network_submit_error(input.prefer_network,input.docker_network.as_deref().unwrap_or_default()){message.set(error.into());return;}
            let body=serde_json::to_string(&input).unwrap_or_default();
            match mutate("PUT",&format!("/api/v1/servers/{id}"),Some(body)).await{Ok(())=>{message.set("Server updated".into());revision+=1;},Err(e)=>message.set(e)}
        }},
        label {"Name" input {value:"{name}",oninput:move |e|name.set(e.value())}}
        label {"Transport" select {value:"{transport}",onchange:move |e|transport.set(e.value()),
            option {value:"http","HTTP proxy"} option {value:"tls","Mutual TLS"} option {value:"ssh","SSH"} option {value:"unix","Unix socket"}
        }}
        label {"Docker API endpoint" input {value:"{endpoint}",oninput:move |e|endpoint.set(e.value())}}
        if transport()=="tls"{
            label {"Client key path" input {value:"{key}",oninput:move |e|key.set(e.value())}}
            label {"Client cert path" input {value:"{cert}",oninput:move |e|cert.set(e.value())}}
            label {"CA path" input {value:"{ca}",oninput:move |e|ca.set(e.value())}}
        }
        label {"Traefik-reachable host" input {value:"{upstream}",oninput:move |e|upstream.set(e.value())} small {"Server host used when connecting through published ports."}}
        label {input {r#type:"checkbox",checked:prefer_network(),onchange:move |e|{
            let checked=e.checked();prefer_network.set(checked);
            if checked && discovery.peek().should_auto_load(){load_local_networks(discovery);}
        }} "Prefer Docker network"}
        if prefer_network(){
            DockerNetworkField {discovery,docker_network}
        }
        label {input {r#type:"checkbox",checked:webui_same_as_server(),onchange:move |e|webui_same_as_server.set(e.checked())} "Same as server host"}
        label {"Browser-reachable host" input {disabled:webui_same_as_server(),value:if webui_same_as_server(){upstream()}else{browser()},oninput:move |e|browser.set(e.value())} small {"Used with the container's published external WebUI port."}}
        label {"Enabled" input {r#type:"checkbox",checked:enabled(),onchange:move |e|enabled.set(e.checked())}}
        button {r#type:"submit","Save changes"}
    }}
}

fn validate_browser_url(value: &str) -> Result<String, String> {
    let value = value.trim();
    if value.is_empty() {
        return Err("Enter a browser URL or choose Reset to automatic.".into());
    }
    let has_authority = value
        .get(..7)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("http://"))
        || value
            .get(..8)
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case("https://"));
    if !has_authority {
        return Err("Enter a valid absolute HTTP or HTTPS URL.".into());
    }
    let parsed = url::Url::parse(value)
        .map_err(|_| "Enter a valid absolute HTTP or HTTPS URL.".to_string())?;
    if !matches!(parsed.scheme(), "http" | "https") || parsed.host_str().is_none() {
        return Err("Enter a valid absolute HTTP or HTTPS URL.".into());
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err("Browser URLs cannot include a username or password.".into());
    }
    Ok(value.to_owned())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct RoutingPresentation {
    label: &'static str,
    class: &'static str,
    locked: bool,
    sr_text: Option<&'static str>,
    title: Option<&'static str>,
}

/// Card presentation for a routing state. Only a confirmed route with
/// confirmed forward auth is green with a lock; known and unknown missing
/// auth are amber but remain distinguishable to assistive technology.
fn routing_presentation(state: Option<&str>, forward_auth: Option<bool>) -> RoutingPresentation {
    let plain = |label, class| RoutingPresentation {
        label,
        class,
        locked: false,
        sr_text: None,
        title: None,
    };
    match state {
        Some("routed") => match forward_auth {
            Some(true) => RoutingPresentation {
                label: "Routed",
                class: "routed",
                locked: true,
                sr_text: Some(" · protected by forward auth"),
                title: Some("Routed through Traefik · protected by forward auth"),
            },
            Some(false) => RoutingPresentation {
                label: "Routed",
                class: "routed-unprotected",
                locked: false,
                sr_text: Some(" · no forward auth"),
                title: Some("Routed through Traefik · no forward auth"),
            },
            None => RoutingPresentation {
                label: "Routed",
                class: "routed-unprotected",
                locked: false,
                sr_text: Some(" · forward auth not confirmed"),
                title: Some("Routed through Traefik · forward auth not confirmed"),
            },
        },
        Some("unconfirmed") => plain("Generated · unconfirmed", "unconfirmed"),
        Some("not_routed") => plain("Not routed", "not-routed"),
        _ => plain("Status unavailable", "unavailable"),
    }
}

/// Builds the clickable route URL from the route hostname and observed TLS.
/// Plain-text fallback (`None`) for anything that is not a DNS hostname;
/// IDN input is normalised to its punycode (ASCII) form. Never adds a port
/// or path and never uses the browser URL.
fn route_href(hostname: &str, tls: Option<bool>) -> Option<String> {
    // Only letters (including Unicode, for IDN), digits, dots and hyphens:
    // rejects ports, paths, credentials, IPv6 literals and quoting up front.
    if hostname.is_empty()
        || !hostname
            .chars()
            .all(|c| c.is_alphanumeric() || c == '.' || c == '-')
    {
        return None;
    }
    let scheme = if tls == Some(false) { "http" } else { "https" };
    let parsed = url::Url::parse(&format!("{scheme}://{hostname}/")).ok()?;
    let host = match parsed.host()? {
        url::Host::Domain(host) => host.to_owned(),
        _ => return None,
    };
    let valid = host.len() <= 253
        && host.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        });
    valid.then(|| format!("{scheme}://{host}"))
}

fn format_checked_at(timestamp: i64) -> String {
    let days = timestamp.div_euclid(86_400);
    let seconds = timestamp.rem_euclid(86_400);
    let shifted = days + 719_468;
    let era = shifted.div_euclid(146_097);
    let day_of_era = shifted - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let mut year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = month_prime + if month_prime < 10 { 3 } else { -9 };
    if month <= 2 {
        year += 1;
    }
    let hour = seconds / 3_600;
    let minute = (seconds % 3_600) / 60;
    format!("{year:04}-{month:02}-{day:02} {hour:02}:{minute:02} UTC")
}

#[component]
fn ContainerCard(container: Container, revision: Signal<u64>, message: Signal<String>) -> Element {
    let icon = container_icon_url(&container.labels).map(str::to_owned);
    let fallback = if container.name.is_empty() {
        "?".to_string()
    } else {
        container
            .name
            .chars()
            .take(2)
            .collect::<String>()
            .to_uppercase()
    };
    let state_class = if container.state == "running" {
        "running"
    } else {
        "non-running"
    };
    let id = container.id.clone();
    let server_id = container.server_id.clone();
    let routing = container.routing.clone();
    let presentation = routing_presentation(
        routing.as_ref().map(|value| value.state.as_str()),
        routing.as_ref().and_then(|value| value.forward_auth),
    );
    let routing_reason = routing.as_ref().and_then(|value| value.reason.clone());
    let route_hostname = routing.as_ref().and_then(|value| value.hostname.clone());
    let route_tls = routing.as_ref().and_then(|value| value.tls);
    let route_link = route_hostname
        .as_deref()
        .and_then(|hostname| route_href(hostname, route_tls));
    let checked_at = routing
        .as_ref()
        .and_then(|value| value.checked_at)
        .map(format_checked_at);
    let custom_url = container.webui_url_override.clone();
    let has_custom_url = custom_url.is_some();
    let url_source = if has_custom_url {
        "Custom"
    } else {
        "Automatic"
    };
    let effective_url = container.webui_url.clone();
    let edit_start_url = custom_url
        .or_else(|| effective_url.clone())
        .or_else(|| container.webui_url_default.clone())
        .unwrap_or_default();
    let mut details = use_signal(|| None::<String>);
    let mut editing_url = use_signal(|| false);
    let mut url_input = use_signal(String::new);
    let mut url_error = use_signal(|| None::<String>);
    let mut saving_url = use_signal(|| false);
    let mut published_ports = container
        .ports
        .iter()
        .filter_map(|port| {
            port.public_port
                .map(|host| (port.private_port, host, port.protocol.clone()))
        })
        .collect::<Vec<_>>();
    published_ports.sort_unstable();
    published_ports.dedup();
    rsx! {article {class:"panel",
        if let Some(icon)=icon {img {class:"icon",src:"{icon}",alt:""}}
        else {span {class:"icon-fallback",aria_label:"No icon available","{fallback}"}}
        h3 {"{container.name}"}
        p {"{container.image}"}
        p {"{container.server_name} · " span {class:"container-state {state_class}","{container.state}"}}
        if let Some(health)=container.health {p {"Health: {health}"}}
        p {for (private_port,host,protocol) in published_ports {span {class:"port","{private_port} → {host}/{protocol} "}}}
        section {class:"container-routing",aria_label:"Traefik routing status",
            p {class:"routing-line",
                span {class:"routing-label","Traefik"}
                span {class:"routing-status {presentation.class}",title:presentation.title,
                    if presentation.locked {
                        svg {class:"routing-lock",view_box:"0 0 16 16",fill:"currentColor","aria-hidden":"true","focusable":"false",
                            path {fill_rule:"evenodd",d:"M4 7V5a4 4 0 1 1 8 0v2h.5A1.5 1.5 0 0 1 14 8.5v5a1.5 1.5 0 0 1-1.5 1.5h-9A1.5 1.5 0 0 1 2 13.5v-5A1.5 1.5 0 0 1 3.5 7H4Zm2 0h4V5a2 2 0 1 0-4 0v2Z"}
                        }
                    }
                    "{presentation.label}"
                    if let Some(sr_text)=presentation.sr_text {span {class:"visually-hidden","{sr_text}"}}
                }
            }
            if let Some(link)=route_link {
                p {class:"routing-detail","Route: " a {href:"{link}",target:"_blank",rel:"noopener noreferrer","{link}"}}
            } else if let Some(hostname)=route_hostname {
                p {class:"routing-detail","Route: {hostname}"}
            }
            if let Some(reason)=routing_reason {p {class:"routing-detail","{reason}"}}
            if let Some(checked_at)=checked_at {small {class:"routing-checked","Last checked: {checked_at}"}}
        }
        section {class:"browser-section",aria_label:"Browser URL",
            div {class:"browser-heading",
                span {class:"browser-label","Browser URL"}
                span {class:"url-source","{url_source}"}
            }
            if let Some(link)=effective_url {
                p {class:"browser-url",a {href:"{link}",target:"_blank",rel:"noopener noreferrer","{link}"}}
            } else {
                p {class:"browser-url browser-url-empty","No browser URL configured."}
            }
            if editing_url() {
                form {class:"browser-edit-form",aria_busy:saving_url(),onsubmit:{let id=id.clone();let server_id=server_id.clone();move |e|{
                    e.prevent_default();
                    let id=id.clone();let server_id=server_id.clone();let value=url_input();
                    async move {
                        if saving_url(){return;}
                        let value=match validate_browser_url(&value){Ok(value)=>value,Err(error)=>{url_error.set(Some(error));return;}};
                        saving_url.set(true);url_error.set(None);
                        let body=serde_json::json!({"url":value}).to_string();
                        match mutate("PUT",&format!("/api/v1/containers/{server_id}/{id}/webui-url"),Some(body)).await {
                            Ok(())=>{editing_url.set(false);message.set("Browser URL updated".into());revision+=1;},
                            Err(error)=>url_error.set(Some(error)),
                        }
                        saving_url.set(false);
                    }
                }},
                    label {"Browser URL"
                        input {r#type:"url",required:true,autofocus:true,disabled:saving_url(),placeholder:"https://app.example.com/path",value:"{url_input}",oninput:move |e|{url_input.set(e.value());url_error.set(None);}}
                    }
                    small {"Use an absolute HTTP or HTTPS URL. This changes only the browser link; Traefik routing is not affected."}
                    if let Some(error)=url_error(){p {class:"error",role:"alert","{error}"}}
                    div {class:"actions",
                        button {r#type:"submit",disabled:saving_url(),if saving_url(){"Saving…"}else{"Save"}}
                        button {r#type:"button",class:"secondary",disabled:saving_url(),onclick:move |_|{editing_url.set(false);url_error.set(None);},"Cancel"}
                        if has_custom_url {
                            button {r#type:"button",class:"secondary",disabled:saving_url(),onclick:{let id=id.clone();let server_id=server_id.clone();move |_|{let id=id.clone();let server_id=server_id.clone();async move{
                                if saving_url(){return;}
                                saving_url.set(true);url_error.set(None);
                                let body=serde_json::json!({"url":null}).to_string();
                                match mutate("PUT",&format!("/api/v1/containers/{server_id}/{id}/webui-url"),Some(body)).await {
                                    Ok(())=>{editing_url.set(false);message.set("Browser URL reset to automatic".into());revision+=1;},
                                    Err(error)=>url_error.set(Some(error)),
                                }
                                saving_url.set(false);
                            }}},"Reset to automatic"}
                        }
                    }
                }
            } else {
                button {r#type:"button",class:"secondary edit-url",onclick:move |_|{url_input.set(edit_start_url.clone());url_error.set(None);editing_url.set(true);},"Edit URL"}
            }
        }
        button {onclick:{let id=id.clone();let server_id=server_id.clone();move |_|{let id=id.clone();let server_id=server_id.clone();async move{
            match request::<serde_json::Value>(&format!("/api/v1/containers/{server_id}/{id}")).await{
                Ok(value)=>details.set(Some(serde_json::to_string_pretty(&value).unwrap_or_default())),Err(e)=>message.set(e)
            }
        }}},"Inspect"}
        if let Some(info)=details(){details {summary {"Container details"}pre {"{info}"}}}
        div {class:"actions lifecycle-actions",
            for command in match container.state.as_str(){"running"=>vec!["stop","restart","pause"],"paused"=>vec!["unpause","stop"],"exited"|"created"=>vec!["start"],_=>vec![]} {
                button {onclick:{let id=id.clone();let server_id=server_id.clone();move |_|{let id=id.clone();let server_id=server_id.clone();async move{
                    let body=serde_json::json!({"command":command}).to_string();
                    match mutate("POST",&format!("/api/v1/containers/{server_id}/{id}/actions"),Some(body)).await{Ok(())=>{message.set(format!("{command} sent"));revision+=1;},Err(e)=>message.set(e)}
                }}},"{command}"}
            }
        }
    }}
}

#[cfg(test)]
mod container_card_tests {
    use super::{format_checked_at, route_href, routing_presentation, validate_browser_url};

    #[test]
    fn routed_presentation_distinguishes_protected_unprotected_and_unknown() {
        let protected = routing_presentation(Some("routed"), Some(true));
        assert_eq!(
            (protected.label, protected.class, protected.locked),
            ("Routed", "routed", true)
        );
        assert_eq!(protected.sr_text, Some(" · protected by forward auth"));
        assert_eq!(
            protected.title,
            Some("Routed through Traefik · protected by forward auth")
        );

        let unprotected = routing_presentation(Some("routed"), Some(false));
        assert_eq!(
            (unprotected.label, unprotected.class, unprotected.locked),
            ("Routed", "routed-unprotected", false)
        );
        assert_eq!(unprotected.sr_text, Some(" · no forward auth"));
        assert_eq!(
            unprotected.title,
            Some("Routed through Traefik · no forward auth")
        );

        let unknown = routing_presentation(Some("routed"), None);
        assert_eq!(
            (unknown.label, unknown.class, unknown.locked),
            ("Routed", "routed-unprotected", false)
        );
        assert_eq!(unknown.sr_text, Some(" · forward auth not confirmed"));
        assert_eq!(
            unknown.title,
            Some("Routed through Traefik · forward auth not confirmed")
        );
        assert_ne!(unknown.sr_text, unprotected.sr_text);
        assert_ne!(unknown.title, unprotected.title);
    }

    #[test]
    fn non_routed_presentation_is_unchanged_and_never_locked() {
        for auth in [Some(true), Some(false), None] {
            for (state, label, class) in [
                (
                    Some("unconfirmed"),
                    "Generated · unconfirmed",
                    "unconfirmed",
                ),
                (Some("not_routed"), "Not routed", "not-routed"),
                (Some("unavailable"), "Status unavailable", "unavailable"),
                (Some("something-else"), "Status unavailable", "unavailable"),
                (None, "Status unavailable", "unavailable"),
            ] {
                let p = routing_presentation(state, auth);
                assert_eq!((p.label, p.class), (label, class), "{state:?} {auth:?}");
                assert!(!p.locked, "{state:?} {auth:?} locked");
                assert_eq!(p.sr_text, None);
                assert_eq!(p.title, None);
            }
        }
    }

    #[test]
    fn route_href_scheme_follows_observed_tls_with_https_default() {
        assert_eq!(
            route_href("app.example.com", Some(false)).as_deref(),
            Some("http://app.example.com")
        );
        assert_eq!(
            route_href("app.example.com", Some(true)).as_deref(),
            Some("https://app.example.com")
        );
        assert_eq!(
            route_href("app.example.com", None).as_deref(),
            Some("https://app.example.com")
        );
    }

    #[test]
    fn route_href_accepts_dns_hostnames_including_punycode() {
        assert_eq!(
            route_href("App.Example.COM", None).as_deref(),
            Some("https://app.example.com")
        );
        assert_eq!(
            route_href("xn--bcher-kva.example", None).as_deref(),
            Some("https://xn--bcher-kva.example")
        );
        assert_eq!(
            route_href("bücher.example", None).as_deref(),
            Some("https://xn--bcher-kva.example")
        );
        assert_eq!(
            route_href("my-app.lan", Some(false)).as_deref(),
            Some("http://my-app.lan")
        );
        assert_eq!(
            route_href("localhost", None).as_deref(),
            Some("https://localhost")
        );
    }

    #[test]
    fn route_href_rejects_non_hostnames() {
        let long_label = format!("{}.example.com", "a".repeat(64));
        for value in [
            "",
            " ",
            "app example.com",
            "app.example.com/path",
            "app.example.com:8443",
            "user@app.example.com",
            "app.example.com?x=1",
            "app.example.com#x",
            "`app.example.com`",
            "javascript:alert(1)",
            "[::1]",
            "192.168.1.20",
            "-app.example.com",
            "app-.example.com",
            "app..example.com",
            "app_1.example.com",
            "app.example.com%2F",
            "app\\example.com",
            "\"app.example.com\"",
            long_label.as_str(),
        ] {
            assert_eq!(route_href(value, None), None, "accepted {value:?}");
        }
    }

    #[test]
    fn browser_url_accepts_absolute_http_urls_with_query_and_fragment() {
        assert_eq!(
            validate_browser_url(" https://example.com:8443/admin?tab=1#settings ").unwrap(),
            "https://example.com:8443/admin?tab=1#settings"
        );
        assert!(validate_browser_url("http://192.168.1.20:8080/").is_ok());
        assert_eq!(
            validate_browser_url("HTTP://[2001:db8::1]:8080/admin?tab=1#settings").unwrap(),
            "HTTP://[2001:db8::1]:8080/admin?tab=1#settings"
        );
    }

    #[test]
    fn browser_url_rejects_relative_unsafe_and_credential_urls() {
        for value in [
            "",
            "/admin",
            "https:example.com",
            "http:foo",
            "javascript:alert(1)",
            "file:///tmp/admin",
            "https://user:password@example.com/",
        ] {
            assert!(validate_browser_url(value).is_err(), "accepted {value}");
        }
    }

    #[test]
    fn checked_at_is_readable_utc() {
        assert_eq!(format_checked_at(0), "1970-01-01 00:00 UTC");
        assert_eq!(format_checked_at(-1), "1969-12-31 23:59 UTC");
        assert_eq!(format_checked_at(1_700_000_000), "2023-11-14 22:13 UTC");
    }
}

#[cfg(test)]
mod network_discovery_tests {
    use super::{
        DiscoveryStatus, LOCAL_NETWORKS_PATH, NetworkDiscovery, PickerState, ServerDiscovery,
        local_network_error, network_options, network_submit_error, picker_state,
        saved_network_missing,
    };

    fn found(networks: &[&str]) -> Result<ServerDiscovery, String> {
        Ok(ServerDiscovery {
            suggested_host: None,
            networks: networks.iter().map(|n| n.to_string()).collect(),
        })
    }

    #[test]
    fn options_come_from_the_app_local_endpoint() {
        assert_eq!(LOCAL_NETWORKS_PATH, "/api/v1/networks/local");
    }

    #[test]
    fn newest_request_wins_and_stale_responses_are_ignored() {
        let mut state = NetworkDiscovery::default();
        let first = state.begin();
        let second = state.begin();
        assert!(state.finish(second, found(&["apps"])));
        // The older request answers late and must not replace it or block anything.
        assert!(!state.finish(first, found(&["old"])));
        assert!(!state.finish(first, Err("boom".into())));
        assert_eq!(state.status, DiscoveryStatus::Loaded(vec!["apps".into()]));
        // A refresh after that is accepted normally.
        let third = state.begin();
        assert!(state.finish(third, Err("socket unavailable".into())));
        assert_eq!(
            state.status,
            DiscoveryStatus::Failed("socket unavailable".into())
        );
    }

    #[test]
    fn auto_load_only_when_nothing_is_loaded_or_loading() {
        let mut state = NetworkDiscovery::default();
        assert!(state.should_auto_load());
        let ticket = state.begin();
        assert!(!state.should_auto_load());
        state.finish(ticket, found(&[]));
        assert!(!state.should_auto_load());
        let ticket = state.begin();
        state.finish(ticket, Err("x".into()));
        assert!(state.should_auto_load());
    }

    #[test]
    fn picker_states_are_distinct() {
        assert_eq!(
            picker_state(&DiscoveryStatus::NotLoaded),
            PickerState::NotLoaded
        );
        assert_eq!(
            picker_state(&DiscoveryStatus::Loading),
            PickerState::Loading
        );
        assert_eq!(
            picker_state(&DiscoveryStatus::Failed("e".into())),
            PickerState::Failed("e")
        );
        assert_eq!(
            picker_state(&DiscoveryStatus::Loaded(vec![])),
            PickerState::Empty
        );
        assert_eq!(
            picker_state(&DiscoveryStatus::Loaded(vec!["a".into(), "b".into()])),
            PickerState::Loaded(2)
        );
    }

    #[test]
    fn selection_is_kept_and_missing_only_after_successful_load() {
        for status in [
            DiscoveryStatus::NotLoaded,
            DiscoveryStatus::Loading,
            DiscoveryStatus::Failed("e".into()),
        ] {
            assert!(!saved_network_missing(&status, "apps"));
            assert_eq!(
                network_options(&status, "apps"),
                vec![("apps".into(), "apps".into())]
            );
        }
        let loaded = DiscoveryStatus::Loaded(vec!["web".into()]);
        assert!(saved_network_missing(&loaded, "apps"));
        assert_eq!(
            network_options(&loaded, "apps"),
            vec![
                ("apps".into(), "apps (not attached to this app)".into()),
                ("web".into(), "web".into())
            ]
        );
        assert!(!saved_network_missing(&loaded, "web"));
        assert!(!saved_network_missing(&loaded, ""));
        assert_eq!(
            network_options(&loaded, "web"),
            vec![("web".into(), "web".into())]
        );
        assert!(saved_network_missing(
            &DiscoveryStatus::Loaded(vec![]),
            "apps"
        ));
    }

    #[test]
    fn local_errors_are_short_and_never_mention_a_proxy() {
        let html = "<html><body><h1>403 Forbidden</h1>\nRequest forbidden by administrative rules.\n</body></html>";
        let summary = local_network_error(html);
        assert_eq!(
            summary,
            "the local Docker socket denied access (403 Forbidden)."
        );
        assert!(!summary.to_ascii_lowercase().contains("proxy"));
        assert_eq!(
            local_network_error("  connection\n refused  "),
            "connection refused"
        );
        assert_eq!(local_network_error("<p></p>"), "request failed");
        let long = local_network_error(&"x".repeat(400));
        assert!(long.ends_with('…') && long.chars().count() <= 161);
    }

    #[test]
    fn submit_requires_a_network_when_preferred() {
        assert!(network_submit_error(true, "").is_some());
        assert!(network_submit_error(true, "  ").is_some());
        assert_eq!(network_submit_error(true, "apps"), None);
        assert_eq!(network_submit_error(false, ""), None);
    }
}
