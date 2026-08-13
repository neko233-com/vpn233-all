use anyhow::{Context, Result};
use axum::{
    extract::{Query, State},
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use clap::{Parser, Subcommand};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::process::Command as ProcessCommand;
use std::{collections::HashMap, net::SocketAddr, path::PathBuf, sync::Arc};
use tokio::{fs, sync::RwLock};
use tracing::info;
use uuid::Uuid;

#[derive(Parser, Debug)]
#[command(name = "vpn233-provider-server", version)]
struct Args {
    #[arg(
        short,
        long,
        env = "VPN233_CONFIG",
        default_value = "provider-server/config.yaml"
    )]
    config: PathBuf,
    #[arg(long)]
    init: bool,
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand, Debug)]
enum Command {
    Update,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct Config {
    #[serde(default = "default_listen_addr")]
    listen_addr: String,
    #[serde(default = "default_provider_port")]
    listen_port: u16,
    #[serde(default = "default_admin_user")]
    admin_user: String,
    #[serde(default = "default_admin_password")]
    admin_password: String,
    #[serde(default = "default_node_ip")]
    default_node_ip: String,
    #[serde(default)]
    provider_access_token: String,
    #[serde(default = "default_mihomo_url")]
    mihomo_url: String,
    #[serde(default)]
    mihomo_sha256: String,
    #[serde(default = "default_port_base")]
    default_port_base: u16,
    #[serde(default)]
    subscribe_verify_token: String,
    #[serde(default)]
    node_links: Vec<String>,
    #[serde(default)]
    registry_url: String,
    #[serde(default)]
    registry_token: String,
    #[serde(default)]
    registry_provider_id: String,
    #[serde(default)]
    registry_name: String,
    #[serde(default = "default_heartbeat_interval_secs")]
    heartbeat_interval_secs: u64,
    #[serde(default)]
    advertise_addresses: Vec<AdvertiseAddress>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct AdvertiseAddress {
    kind: String,
    value: String,
}

fn default_listen_addr() -> String {
    "0.0.0.0".into()
}
fn default_provider_port() -> u16 {
    8080
}
fn default_admin_user() -> String {
    "root".into()
}
fn default_admin_password() -> String {
    "root".into()
}
fn default_node_ip() -> String {
    "auto".into()
}
fn default_mihomo_url() -> String {
    "https://github.com/MetaCubeX/mihomo/releases/latest/download/mihomo-linux-amd64-compatible-v1.gz".into()
}
fn default_port_base() -> u16 {
    10000
}
fn default_heartbeat_interval_secs() -> u64 {
    30
}

#[derive(Clone)]
struct AppState {
    config: Arc<RwLock<Config>>,
    sessions: Arc<RwLock<HashMap<String, String>>>,
}

#[derive(Debug, Deserialize)]
struct RegistryRegisterResponse {
    heartbeat_token: String,
    heartbeat_interval_secs: u64,
}

#[derive(Debug, Serialize)]
struct RegistryRegisterRequest {
    provider_id: String,
    name: String,
    version: String,
    addresses: Vec<AdvertiseAddress>,
    node_links: Vec<String>,
    heartbeat_interval_secs: u64,
}

#[derive(Debug, Serialize)]
struct RegistryHeartbeatRequest {
    provider_id: String,
    heartbeat_token: String,
    version: String,
    addresses: Vec<AdvertiseAddress>,
    node_links: Vec<String>,
}

async fn run_registry_heartbeat(config: Config) -> Result<()> {
    if config.registry_url.trim().is_empty() {
        return Ok(());
    }
    if config.registry_token.trim().is_empty() {
        anyhow::bail!("registry_url is configured but registry_token is empty");
    }
    let provider_id = if config.registry_provider_id.trim().is_empty() {
        format!("provider-{}", Uuid::new_v4())
    } else {
        config.registry_provider_id.clone()
    };
    let name = if config.registry_name.trim().is_empty() {
        provider_id.clone()
    } else {
        config.registry_name.clone()
    };
    let client = reqwest::Client::builder()
        .user_agent(format!(
            "vpn233-provider-server/{}",
            env!("CARGO_PKG_VERSION")
        ))
        .build()?;
    let mut heartbeat_token: Option<String> = None;
    let mut interval_secs = config.heartbeat_interval_secs.clamp(10, 3600);
    loop {
        let url = format!(
            "{}/api/v1/registry/{}",
            config.registry_url.trim_end_matches('/'),
            if heartbeat_token.is_some() {
                "heartbeat"
            } else {
                "register"
            }
        );
        if let Some(token) = heartbeat_token.clone() {
            let request = RegistryHeartbeatRequest {
                provider_id: provider_id.clone(),
                heartbeat_token: token,
                version: env!("CARGO_PKG_VERSION").to_owned(),
                addresses: config.advertise_addresses.clone(),
                node_links: config.node_links.clone(),
            };
            match client
                .post(&url)
                .header("x-provider-registry-token", &config.registry_token)
                .json(&request)
                .send()
                .await
            {
                Ok(response) if response.status().is_success() => {
                    tokio::time::sleep(std::time::Duration::from_secs(interval_secs)).await;
                    continue;
                }
                Ok(response) => {
                    tracing::warn!(status = %response.status(), "provider registry heartbeat rejected; registering again");
                    heartbeat_token = None;
                }
                Err(error) => tracing::warn!(%error, "provider registry heartbeat request failed"),
            }
        } else {
            let request = RegistryRegisterRequest {
                provider_id: provider_id.clone(),
                name: name.clone(),
                version: env!("CARGO_PKG_VERSION").to_owned(),
                addresses: config.advertise_addresses.clone(),
                node_links: config.node_links.clone(),
                heartbeat_interval_secs: interval_secs,
            };
            match client
                .post(&url)
                .header("x-provider-registry-token", &config.registry_token)
                .json(&request)
                .send()
                .await
            {
                Ok(response) if response.status().is_success() => {
                    let payload: RegistryRegisterResponse = response.json().await?;
                    heartbeat_token = Some(payload.heartbeat_token);
                    interval_secs = payload.heartbeat_interval_secs.clamp(10, 3600);
                    tracing::info!(provider_id = %provider_id, "provider registered with subscribe server");
                }
                Ok(response) => {
                    tracing::warn!(status = %response.status(), "provider registry registration rejected")
                }
                Err(error) => {
                    tracing::warn!(%error, "provider registry registration request failed")
                }
            }
        }
        tokio::time::sleep(std::time::Duration::from_secs(interval_secs)).await;
    }
}

#[derive(Debug, Deserialize)]
struct LoginRequest {
    username: String,
    password: String,
}

#[derive(Debug, Deserialize)]
struct GenerateQuery {
    format: Option<String>,
    node_name: Option<String>,
    node_ip: Option<String>,
    port_base: Option<u16>,
    uuid: Option<String>,
    password: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
struct NodeManifest {
    node_name: String,
    node_ip: String,
    uuid: String,
    password: String,
    ports: Vec<NodePort>,
}

#[derive(Debug, Clone, Serialize)]
struct NodePort {
    id: String,
    name: String,
    core: String,
    port: u16,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt().with_env_filter("info").init();
    let args = Args::parse();
    if matches!(args.command, Some(Command::Update)) {
        return run_update();
    }
    if args.init {
        let path = args.config;
        if fs::try_exists(&path).await.unwrap_or(false) {
            anyhow::bail!("refusing to overwrite existing config: {}", path.display());
        }
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).await?;
        }
        fs::write(&path, include_str!("../config.example.yaml")).await?;
        println!("created {}", path.display());
        return Ok(());
    }

    let config = load_config(&args.config).await?;
    let addr: SocketAddr = format!("{}:{}", config.listen_addr, config.listen_port)
        .parse()
        .context("invalid provider listen address")?;
    let registry_config = config.clone();
    let state = AppState {
        config: Arc::new(RwLock::new(config)),
        sessions: Arc::new(RwLock::new(HashMap::new())),
    };
    let app = Router::new()
        .route("/api/v1/health", get(health))
        .route("/api/v1/login", post(login))
        .route("/api/v1/protocols", get(protocols))
        .route("/api/v1/config", get(config_view))
        .route("/api/v1/generate", get(generate))
        .route("/api/v1/generate.sh", get(generate_sh))
        .route("/api/v1/generate.ps1", get(generate_ps1))
        .route("/api/v1/nodes", get(nodes))
        .route("/api/v1/subscribe/verify", get(subscribe_verify))
        .with_state(state);
    let listener = tokio::net::TcpListener::bind(addr).await?;
    info!(%addr, "vpn233 provider server listening");
    tokio::spawn(async move {
        if let Err(error) = run_registry_heartbeat(registry_config).await {
            tracing::warn!(%error, "provider registry heartbeat stopped");
        }
    });
    axum::serve(listener, app).await?;
    Ok(())
}

fn run_update() -> Result<()> {
    let script = if cfg!(windows) {
        "update.ps1"
    } else {
        "update.sh"
    };
    if let Ok(path) = std::env::var("VPN233_UPDATE_SCRIPT") {
        let status = if cfg!(windows) {
            ProcessCommand::new("powershell")
                .args(["-NoProfile", "-File", &path])
                .status()?
        } else {
            ProcessCommand::new("sh").arg(path).status()?
        };
        anyhow::ensure!(status.success(), "update script failed with {status}");
        return Ok(());
    }
    let executable = std::env::current_exe()?;
    let root = executable
        .parent()
        .and_then(|path| path.parent())
        .context("cannot locate installation root")?;
    let path = root.join(script);
    anyhow::ensure!(path.exists(), "update script not found: {}", path.display());
    let status = if cfg!(windows) {
        ProcessCommand::new("powershell")
            .args(["-NoProfile", "-File"])
            .arg(path)
            .status()?
    } else {
        ProcessCommand::new("sh").arg(path).status()?
    };
    anyhow::ensure!(status.success(), "update script failed with {status}");
    Ok(())
}

async fn load_config(path: &PathBuf) -> Result<Config> {
    let mut config = if fs::try_exists(path).await? {
        let raw = fs::read_to_string(path).await?;
        serde_yaml::from_str::<Config>(&raw).context("invalid provider YAML")?
    } else {
        Config {
            listen_addr: default_listen_addr(),
            listen_port: default_provider_port(),
            admin_user: default_admin_user(),
            admin_password: default_admin_password(),
            default_node_ip: default_node_ip(),
            provider_access_token: String::new(),
            mihomo_url: default_mihomo_url(),
            mihomo_sha256: String::new(),
            default_port_base: default_port_base(),
            subscribe_verify_token: String::new(),
            node_links: Vec::new(),
            registry_url: String::new(),
            registry_token: String::new(),
            registry_provider_id: String::new(),
            registry_name: String::new(),
            heartbeat_interval_secs: default_heartbeat_interval_secs(),
            advertise_addresses: Vec::new(),
        }
    };
    if let Ok(v) = std::env::var("VPN233_ADMIN_USER") {
        config.admin_user = v;
    }
    if let Ok(v) = std::env::var("VPN233_ADMIN_PASSWORD") {
        config.admin_password = v;
    }
    if let Ok(v) = std::env::var("VPN233_PROVIDER_ACCESS_TOKEN") {
        config.provider_access_token = v;
    }
    if let Ok(v) = std::env::var("VPN233_MIHOMO_URL") {
        config.mihomo_url = v;
    }
    if let Ok(v) = std::env::var("VPN233_MIHOMO_SHA256") {
        config.mihomo_sha256 = v;
    }
    if let Ok(v) = std::env::var("VPN233_PROVIDER_ADDR") {
        let (host, port) = v
            .rsplit_once(':')
            .context("VPN233_PROVIDER_ADDR must be host:port")?;
        config.listen_addr = host.to_owned();
        config.listen_port = port.parse().context("invalid VPN233_PROVIDER_ADDR port")?;
    }
    if let Ok(v) = std::env::var("VPN233_REGISTRY_URL") {
        config.registry_url = v;
    }
    if let Ok(v) = std::env::var("VPN233_REGISTRY_TOKEN") {
        config.registry_token = v;
    }
    if let Ok(v) = std::env::var("VPN233_PROVIDER_ID") {
        config.registry_provider_id = v;
    }
    if let Ok(v) = std::env::var("VPN233_PROVIDER_NAME") {
        config.registry_name = v;
    }
    if let Ok(v) = std::env::var("VPN233_HEARTBEAT_INTERVAL") {
        config.heartbeat_interval_secs = v.parse().context("invalid VPN233_HEARTBEAT_INTERVAL")?;
    }
    if let Ok(v) = std::env::var("VPN233_ADVERTISE_ADDRESSES") {
        config.advertise_addresses = v
            .split(',')
            .filter_map(|entry| entry.split_once('='))
            .map(|(kind, value)| AdvertiseAddress {
                kind: kind.trim().to_owned(),
                value: value.trim().to_owned(),
            })
            .collect();
    }
    Ok(config)
}

async fn health() -> Json<Value> {
    Json(
        json!({"ok": true, "service": "vpn233-provider-server", "version": env!("CARGO_PKG_VERSION")}),
    )
}

async fn login(State(state): State<AppState>, Json(input): Json<LoginRequest>) -> Response {
    let config = state.config.read().await;
    if input.username != config.admin_user || input.password != config.admin_password {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({"error": "invalid credentials"})),
        )
            .into_response();
    }
    let token = Uuid::new_v4().to_string();
    state
        .sessions
        .write()
        .await
        .insert(token.clone(), input.username);
    Json(json!({"token": token, "token_type": "Bearer"})).into_response()
}

async fn protocols() -> Json<Value> {
    Json(json!({
        "mihomo": ["vless", "vmess", "trojan", "ss", "hysteria2", "tuic", "wireguard"],
        "sing_box": ["vless", "vmess", "trojan", "shadowsocks", "anytls", "hysteria2", "tuic", "wireguard"],
        "note": "mihomo core integration is generated as a compatible config; full TUN/core execution remains an external mihomo binary boundary"
    }))
}

async fn config_view(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if !authorized(&state, &headers).await {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let config = state.config.read().await;
    Json(json!({
        "listen_addr": config.listen_addr,
        "listen_port": config.listen_port,
        "admin_user": config.admin_user,
        "default_node_ip": config.default_node_ip,
        "provider_access_token_configured": !config.provider_access_token.is_empty(),
        "mihomo_url": config.mihomo_url,
        "mihomo_sha256_configured": !config.mihomo_sha256.is_empty(),
        "default_port_base": config.default_port_base,
        "subscribe_verify_token_configured": !config.subscribe_verify_token.is_empty()
    }))
    .into_response()
}

async fn generate(
    headers: HeaderMap,
    State(state): State<AppState>,
    Query(query): Query<GenerateQuery>,
) -> Response {
    if !authorized(&state, &headers).await {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let manifest = make_manifest(&state, &query).await;
    let format = query.format.as_deref().unwrap_or("json");
    match format {
        "sh" => plain_script(render_sh(&manifest)),
        "ps1" => plain_script(render_ps1(&manifest)),
        "json" => Json(
            json!({"node": manifest, "sh": render_sh(&manifest), "ps1": render_ps1(&manifest)}),
        )
        .into_response(),
        _ => (StatusCode::BAD_REQUEST, "format must be json, sh, or ps1").into_response(),
    }
}

async fn generate_sh(
    headers: HeaderMap,
    state: State<AppState>,
    query: Query<GenerateQuery>,
) -> Response {
    generate(
        headers,
        state,
        Query(GenerateQuery {
            format: Some("sh".into()),
            ..query.0
        }),
    )
    .await
}

async fn generate_ps1(
    headers: HeaderMap,
    state: State<AppState>,
    query: Query<GenerateQuery>,
) -> Response {
    generate(
        headers,
        state,
        Query(GenerateQuery {
            format: Some("ps1".into()),
            ..query.0
        }),
    )
    .await
}

async fn nodes(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<HashMap<String, String>>,
) -> Response {
    let access_token = state.config.read().await.provider_access_token.clone();
    let supplied = query.get("token").cloned().or_else(|| {
        headers
            .get("x-provider-token")
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned)
    });
    if access_token.is_empty() || supplied.as_deref() != Some(access_token.as_str()) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let config = state.config.read().await;
    if !config.node_links.is_empty() {
        return plain_script(format!("{}\n", config.node_links.join("\n")));
    }
    drop(config);
    let manifest = make_manifest(
        &state,
        &GenerateQuery {
            format: None,
            node_name: None,
            node_ip: None,
            port_base: None,
            uuid: None,
            password: None,
        },
    )
    .await;
    let body = manifest
        .ports
        .iter()
        .map(|port| {
            format!(
                "vless://{}@{}:{}?encryption=none&security=tls&type=tcp#{}-{}",
                manifest.uuid, manifest.node_ip, port.port, manifest.node_name, port.name
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    plain_script(format!("{body}\n"))
}

async fn subscribe_verify(
    State(state): State<AppState>,
    Query(query): Query<HashMap<String, String>>,
) -> Response {
    let config = state.config.read().await;
    if !config.subscribe_verify_token.is_empty()
        && query.get("token") != Some(&config.subscribe_verify_token)
    {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    Json(json!({"ok": true, "service": "vpn233-provider-server", "version": env!("CARGO_PKG_VERSION"), "protocols": ["vless", "vmess", "trojan", "ss"]})).into_response()
}

async fn authorized(state: &AppState, headers: &HeaderMap) -> bool {
    let Some(value) = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
    else {
        return false;
    };
    let Some(token) = value.strip_prefix("Bearer ") else {
        return false;
    };
    state.sessions.read().await.contains_key(token)
}

async fn make_manifest(state: &AppState, query: &GenerateQuery) -> NodeManifest {
    let config = state.config.read().await;
    let node_ip = query
        .node_ip
        .clone()
        .unwrap_or_else(|| config.default_node_ip.clone());
    let node_ip = if node_ip == "auto" {
        "127.0.0.1".into()
    } else {
        node_ip
    };
    let base = query.port_base.unwrap_or(config.default_port_base);
    NodeManifest {
        node_name: query
            .node_name
            .clone()
            .unwrap_or_else(|| "vpn233-node".into()),
        node_ip,
        uuid: query
            .uuid
            .clone()
            .unwrap_or_else(|| Uuid::new_v4().to_string()),
        password: query
            .password
            .clone()
            .unwrap_or_else(|| Uuid::new_v4().to_string()),
        ports: vec![
            NodePort {
                id: "singbox-vless".into(),
                name: "VLESS".into(),
                core: "sing-box".into(),
                port: base,
            },
            NodePort {
                id: "mihomo-vless".into(),
                name: "Mihomo VLESS".into(),
                core: "mihomo".into(),
                port: base + 11,
            },
        ],
    }
}

fn plain_script(body: String) -> Response {
    let mut headers = HeaderMap::new();
    headers.insert(
        header::CONTENT_TYPE,
        "text/plain; charset=utf-8".parse().unwrap(),
    );
    (headers, body).into_response()
}

fn render_sh(node: &NodeManifest) -> String {
    let manifest = BASE64.encode(serde_json::to_vec(node).unwrap());
    format!(
        r##"#!/usr/bin/env bash
set -Eeuo pipefail

# Generated by vpn233-provider-server. Credentials are provided at runtime only.
if [[ "${{EUID}}" -ne 0 ]]; then echo "run as root" >&2; exit 1; fi
DATA_DIR="/etc/vpn233"
mkdir -p "$DATA_DIR/runtime"
printf '%s' '{manifest}' | base64 -d > "$DATA_DIR/runtime/node-manifest.json"
chmod 700 "$DATA_DIR" "$DATA_DIR/runtime"
command -v curl >/dev/null || {{ apt-get update -y && apt-get install -y curl; }}
echo "vpn233 node manifest installed at $DATA_DIR/runtime/node-manifest.json"
echo "Ports: {ports}"
if command -v mihomo >/dev/null 2>&1; then
  echo "Mihomo found at $(command -v mihomo)"
else
  echo "Mihomo is not installed. Use the signed project bundle or a pinned distribution package before enabling traffic."
fi
"##,
        manifest = manifest,
        ports = node
            .ports
            .iter()
            .map(|p| p.port.to_string())
            .collect::<Vec<_>>()
            .join(", ")
    )
}

fn render_ps1(node: &NodeManifest) -> String {
    let json = serde_json::to_string(node).unwrap();
    format!(
        r##"$ErrorActionPreference = 'Stop'
# Generated by vpn233-provider-server. Credentials are provided at runtime only.
$dataDir = Join-Path $env:ProgramData 'vpn233'
$runtime = Join-Path $dataDir 'runtime'
New-Item -ItemType Directory -Force -Path $runtime | Out-Null
@'
{json}
'@ | Set-Content -Encoding UTF8 (Join-Path $runtime 'node-manifest.json')
Write-Host "vpn233 node manifest installed at $runtime\node-manifest.json"
Write-Host "Ports: {ports}"
$mihomo = Join-Path $dataDir 'mihomo.exe'
if (-not (Test-Path $mihomo)) {{
  Write-Host 'Mihomo is not bundled in the generated installer. Set VPN233_MIHOMO_URL to a pinned release and install it with the signed project installer.'
}} else {{ Write-Host "Mihomo found at $mihomo" }}
"##,
        json = json,
        ports = node
            .ports
            .iter()
            .map(|p| p.port.to_string())
            .collect::<Vec<_>>()
            .join(", ")
    )
}
