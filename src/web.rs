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

async fn post_json<T: serde::de::DeserializeOwned>(path: &str, body: String) -> Result<T, String> {
    let response = gloo_net::http::Request::post(path)
        .header("X-Requested-With", "traefik-web")
        .header("Content-Type", "application/json")
        .body(body)
        .map_err(|e| e.to_string())?
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
    let mut docker_network = use_signal(String::new);
    let mut webui_same_as_server = use_signal(|| true);
    let mut networks = use_signal(Vec::<String>::new);
    let mut discovery_loading = use_signal(|| false);
    let mut discovery_error = use_signal(|| None::<String>);
    let mut connection_revision = use_signal(|| 0_u64);
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
            let result=serde_json::to_string(&input).map_err(|e|e.to_string()).and_then(|v|Ok(v));
            let result=match result {Ok(v)=>mutate("POST","/api/v1/servers",Some(v)).await,Err(e)=>Err(e)};
            match result {Ok(())=>{message.set("Server added".into());revision+=1;},Err(e)=>message.set(e)}
        }},
            label {"Name" input {required:true,value:"{name}",oninput:move |e|name.set(e.value())}}
            label {"Transport" select {value:"{transport}",onchange:move |e|{transport.set(e.value());connection_revision+=1;},
                option {value:"http","HTTP socket proxy"} option {value:"tls","Mutual TLS"} option {value:"ssh","SSH"} option {value:"unix","Unix socket"}
            }}
            label {"Docker API endpoint (http://host:2375, ssh://user@host, or /var/run/docker.sock)" input {required:true,value:"{endpoint}",oninput:move |e|{endpoint.set(e.value());connection_revision+=1;}}}
            if transport()=="tls" {
                label {"Client key path" input {value:"{key}",oninput:move |e|{key.set(e.value());connection_revision+=1;}}}
                label {"Client cert path" input {value:"{cert}",oninput:move |e|{cert.set(e.value());connection_revision+=1;}}}
                label {"CA path" input {value:"{ca}",oninput:move |e|{ca.set(e.value());connection_revision+=1;}}}
            }
            button {r#type:"button",disabled:discovery_loading(),onclick:move |_|{let input=make_input();let request_revision=connection_revision();async move{
                discovery_loading.set(true);discovery_error.set(None);
                let result=serde_json::to_string(&input).map_err(|e|e.to_string());
                let result=match result {Ok(body)=>post_json::<ServerDiscovery>("/api/v1/servers/discover",body).await,Err(e)=>Err(e)};
                if connection_revision()==request_revision {match result {
                    Ok(found)=>{if upstream().trim().is_empty(){if let Some(host)=found.suggested_host{upstream.set(host);}}networks.set(found.networks);}
                    Err(e)=>discovery_error.set(Some(e)),
                }}
                discovery_loading.set(false);
            }},if discovery_loading(){"Discovering…"}else{"Discover host and networks"}}
            if let Some(error)=discovery_error(){p {class:"error","Discovery failed: {error}. You can still enter host settings manually."}}
            label {"Traefik-reachable host" input {required:true,value:"{upstream}",oninput:move |e|upstream.set(e.value())}
                small {"Server host used when connecting through published ports."}
            }
            label {input {r#type:"checkbox",checked:prefer_network(),onchange:move |e|prefer_network.set(e.checked())} "Prefer Docker network"}
            if prefer_network(){
                label {"Docker network" select {required:true,value:"{docker_network}",onchange:move |e|docker_network.set(e.value()),
                    option {value:"",disabled:true,"Select a network"}
                    if !docker_network().is_empty() && !networks().contains(&docker_network()){option {value:"{docker_network}","{docker_network} (saved)"}}
                    for network in networks(){option {value:"{network}","{network}"}}
                }
                if discovery_loading(){small {"Loading networks…"}}
                else if networks().is_empty(){small {"No eligible networks found. Check the connection settings and run discovery."}}
                }
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
    let mut docker_network = use_signal(|| server.docker_network.clone().unwrap_or_default());
    let mut webui_same_as_server = use_signal(|| server.webui_same_as_server);
    let mut networks = use_signal(Vec::<String>::new);
    let mut discovery_loading = use_signal(|| false);
    let mut discovery_error = use_signal(|| None::<String>);
    let mut connection_revision = use_signal(|| 0_u64);
    let id = server.id.clone();
    rsx! {form {onsubmit:move |e|{e.prevent_default();let id=id.clone();let input=ServerInput{
        name:name(),endpoint:endpoint(),transport:transport(),upstream_host:upstream(),webui_host:if webui_same_as_server(){upstream()}else{browser()},enabled:enabled(),
        tls_key:(!key().is_empty()).then(||key()),tls_cert:(!cert().is_empty()).then(||cert()),tls_ca:(!ca().is_empty()).then(||ca()),
        prefer_network:prefer_network(),docker_network:prefer_network().then(||docker_network()).filter(|v|!v.is_empty()),webui_same_as_server:webui_same_as_server()};
        async move{let body=serde_json::to_string(&input).unwrap_or_default();
            match mutate("PUT",&format!("/api/v1/servers/{id}"),Some(body)).await{Ok(())=>{message.set("Server updated".into());revision+=1;},Err(e)=>message.set(e)}
        }},
        label {"Name" input {value:"{name}",oninput:move |e|name.set(e.value())}}
        label {"Transport" select {value:"{transport}",onchange:move |e|{transport.set(e.value());connection_revision+=1;},
            option {value:"http","HTTP proxy"} option {value:"tls","Mutual TLS"} option {value:"ssh","SSH"} option {value:"unix","Unix socket"}
        }}
        label {"Docker API endpoint" input {value:"{endpoint}",oninput:move |e|{endpoint.set(e.value());connection_revision+=1;}}}
        if transport()=="tls"{
            label {"Client key path" input {value:"{key}",oninput:move |e|{key.set(e.value());connection_revision+=1;}}}
            label {"Client cert path" input {value:"{cert}",oninput:move |e|{cert.set(e.value());connection_revision+=1;}}}
            label {"CA path" input {value:"{ca}",oninput:move |e|{ca.set(e.value());connection_revision+=1;}}}
        }
        button {r#type:"button",disabled:discovery_loading(),onclick:move |_|{let input=ServerInput{
            name:name(),endpoint:endpoint(),transport:transport(),upstream_host:upstream(),webui_host:if webui_same_as_server(){upstream()}else{browser()},enabled:enabled(),
            tls_key:(!key().is_empty()).then(||key()),tls_cert:(!cert().is_empty()).then(||cert()),tls_ca:(!ca().is_empty()).then(||ca()),
            prefer_network:prefer_network(),docker_network:prefer_network().then(||docker_network()).filter(|v|!v.is_empty()),webui_same_as_server:webui_same_as_server()};
            let request_revision=connection_revision();async move{discovery_loading.set(true);discovery_error.set(None);
                let result=serde_json::to_string(&input).map_err(|e|e.to_string());
                let result=match result{Ok(body)=>post_json::<ServerDiscovery>("/api/v1/servers/discover",body).await,Err(e)=>Err(e)};
                if connection_revision()==request_revision{match result{Ok(found)=>{if upstream().trim().is_empty(){if let Some(host)=found.suggested_host{upstream.set(host);}}networks.set(found.networks);},Err(e)=>discovery_error.set(Some(e))}}
                discovery_loading.set(false);
            }
        },if discovery_loading(){"Discovering…"}else{"Discover host and networks"}}
        if let Some(error)=discovery_error(){p {class:"error","Discovery failed: {error}. You can still enter host settings manually."}}
        label {"Traefik-reachable host" input {value:"{upstream}",oninput:move |e|upstream.set(e.value())} small {"Server host used when connecting through published ports."}}
        label {input {r#type:"checkbox",checked:prefer_network(),onchange:move |e|prefer_network.set(e.checked())} "Prefer Docker network"}
        if prefer_network(){label {"Docker network" select {required:true,value:"{docker_network}",onchange:move |e|docker_network.set(e.value()),
            option {value:"",disabled:true,"Select a network"}
            if !docker_network().is_empty() && !networks().contains(&docker_network()){option {value:"{docker_network}","{docker_network} (saved)"}}
            for network in networks(){option {value:"{network}","{network}"}}
        } if discovery_loading(){small {"Loading networks…"}} else if networks().is_empty(){small {"No networks discovered yet. Refresh discovery or keep the saved selection."}}}}
        label {input {r#type:"checkbox",checked:webui_same_as_server(),onchange:move |e|webui_same_as_server.set(e.checked())} "Same as server host"}
        label {"Browser-reachable host" input {disabled:webui_same_as_server(),value:if webui_same_as_server(){upstream()}else{browser()},oninput:move |e|browser.set(e.value())} small {"Used with the container's published external WebUI port."}}
        label {"Enabled" input {r#type:"checkbox",checked:enabled(),onchange:move |e|enabled.set(e.checked())}}
        button {r#type:"submit","Save changes"}
    }}
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
    let mut details = use_signal(|| None::<String>);
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
        button {onclick:{let id=id.clone();let server_id=server_id.clone();move |_|{let id=id.clone();let server_id=server_id.clone();async move{
            match request::<serde_json::Value>(&format!("/api/v1/containers/{server_id}/{id}")).await{
                Ok(value)=>details.set(Some(serde_json::to_string_pretty(&value).unwrap_or_default())),Err(e)=>message.set(e)
            }
        }}},"Inspect"}
        if let Some(info)=details(){details {summary {"Container details"}pre {"{info}"}}}
        div {class:"actions",
            if let Some(link)=container.webui_url {a {href:"{link}",target:"_blank",rel:"noopener noreferrer","Open WebUI"}}
            for command in match container.state.as_str(){"running"=>vec!["stop","restart","pause"],"paused"=>vec!["unpause","stop"],"exited"|"created"=>vec!["start"],_=>vec![]} {
                button {onclick:{let id=id.clone();let server_id=server_id.clone();move |_|{let id=id.clone();let server_id=server_id.clone();async move{
                    let body=serde_json::json!({"command":command}).to_string();
                    match mutate("POST",&format!("/api/v1/containers/{server_id}/{id}/actions"),Some(body)).await{Ok(())=>{message.set(format!("{command} sent"));revision+=1;},Err(e)=>message.set(e)}
                }}},"{command}"}
            }
        }
    }}
}
