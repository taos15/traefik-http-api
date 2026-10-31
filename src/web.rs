use dioxus::prelude::*;
use serde::{Deserialize, Serialize};
use traefik_http_api::{Container, DockerServer, ServerInput};

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
                button {onclick:move |_|revision+=1,"Refresh"}
                match &*containers.read() {
                    Some(Ok(list))=>rsx! {div {class:"grid",for container in list.iter() { ContainerCard {container:container.clone(),revision,message} }}},
                    Some(Err(e))=>rsx!{p {class:"error","{e}"}},
                    None=>rsx!{p {"Loading containers…"}}
                }
            } else {
                h2 {"Traefik provider"}
                p {"Traefik polls /api/v1/traefikconfig. The endpoint is anonymous unless a provider token is configured."}
                ProviderStatus {}
                ProviderPreview {}
            }
        }
    }
}

#[derive(Serialize, Deserialize, Clone)]
struct Status {
    refreshed_at: Option<i64>,
    default_auth_enabled: bool,
    last_error: Option<String>,
}
#[component]
fn ProviderStatus() -> Element {
    let status = use_resource(|| async { request::<Status>("/api/v1/provider/status").await });
    rsx! {match &*status.read(){
        Some(Ok(s))=>rsx!{ p {"Last successful refresh (Unix time): {s.refreshed_at:?}"}
            p {"Default ForwardAuth configured: {s.default_auth_enabled}"}
            if !s.default_auth_enabled {p {class:"error","Automatic ForwardAuth is OFF. Generated routes are not protected by this app."}}
            if let Some(error)=&s.last_error {p {class:"error","Last refresh failed: {error}. Serving last good snapshot."}}
        },
        Some(Err(e))=>rsx!{p {class:"error","{e}"}},
        None=>rsx!{p {"Loading status…"}}
    }}
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
    let make_input = move || ServerInput {
        name: name(),
        transport: transport(),
        endpoint: endpoint(),
        tls_key: (!key().is_empty()).then(|| key()),
        tls_cert: (!cert().is_empty()).then(|| cert()),
        tls_ca: (!ca().is_empty()).then(|| ca()),
        upstream_host: upstream(),
        webui_host: browser(),
        enabled: enabled(),
    };
    rsx! {section {class:"panel",h2 {"Add Docker host"}
        p {"Docker API access, Traefik upstream, and browser WebUI address are separate."}
        form {onsubmit:move |e|{e.prevent_default();let input=make_input();async move{
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
            label {"Traefik-reachable host" input {required:true,value:"{upstream}",oninput:move |e|upstream.set(e.value())}}
            label {"Browser-reachable host" input {required:true,value:"{browser}",oninput:move |e|browser.set(e.value())}}
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
    rsx! {article {class:"panel",h3 {"{server.name}"} p {"{server.transport} · {server.endpoint}"}
        p {"Upstream: {server.upstream_host} · Browser: {server.webui_host}"}
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
    let id = server.id.clone();
    rsx! {form {onsubmit:move |e|{e.prevent_default();let id=id.clone();let input=ServerInput{
        name:name(),endpoint:endpoint(),transport:transport(),upstream_host:upstream(),webui_host:browser(),enabled:enabled(),
        tls_key:(!key().is_empty()).then(||key()),tls_cert:(!cert().is_empty()).then(||cert()),tls_ca:(!ca().is_empty()).then(||ca())};
        async move{let body=serde_json::to_string(&input).unwrap_or_default();
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
        label {"Traefik-reachable host" input {value:"{upstream}",oninput:move |e|upstream.set(e.value())}}
        label {"Browser-reachable host" input {value:"{browser}",oninput:move |e|browser.set(e.value())}}
        label {"Enabled" input {r#type:"checkbox",checked:enabled(),onchange:move |e|enabled.set(e.checked())}}
        button {r#type:"submit","Save changes"}
    }}
}

#[component]
fn ContainerCard(container: Container, revision: Signal<u64>, message: Signal<String>) -> Element {
    let icon = container
        .labels
        .get("net.unraid.docker.icon")
        .filter(|v| v.starts_with("https://"))
        .cloned();
    let id = container.id.clone();
    let server_id = container.server_id.clone();
    let mut details = use_signal(|| None::<String>);
    rsx! {article {class:"panel",
        if let Some(icon)=icon {img {class:"icon",src:"{icon}",alt:""}}
        h3 {"{container.name}"}
        p {"{container.image}"}
        p {"{container.server_name} · {container.state}"}
        if let Some(health)=container.health {p {"Health: {health}"}}
        p {for port in container.ports.iter(){if let Some(host)=port.public_port{span {class:"port","{port.private_port} → {host}/{port.protocol} "}}}}
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
