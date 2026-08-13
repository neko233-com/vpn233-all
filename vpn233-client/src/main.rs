use anyhow::{Context, Result};
use axum::{
    extract::State,
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
    Json, Router,
};
use base64::Engine;
use clap::{Parser, Subcommand};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::process::Command as ProcessCommand;
use std::{net::SocketAddr, path::PathBuf, sync::Arc};
use tokio::{
    fs,
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::RwLock,
};
use tracing::{info, warn};

#[derive(Parser, Debug)]
#[command(name = "vpn233-client", version)]
struct Args {
    #[arg(
        short,
        long,
        env = "VPN233_CLIENT_CONFIG",
        default_value = "vpn233-client/config.yaml"
    )]
    config: PathBuf,
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand, Debug)]
enum Command {
    Update,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct Config {
    #[serde(default = "default_bind")]
    listen_addr: String,
    #[serde(default = "default_socks_port")]
    socks_port: u16,
    #[serde(default = "default_http_port")]
    http_port: u16,
    #[serde(default = "default_web_addr")]
    web_addr: String,
    #[serde(default = "default_admin_user")]
    admin_user: String,
    #[serde(default = "default_admin_password")]
    admin_password: String,
    #[serde(default = "default_upstream_mode")]
    upstream_mode: String,
    #[serde(default)]
    subscription_url: String,
    #[serde(default)]
    upstream_socks5: String,
}

fn default_bind() -> String {
    "127.0.0.1".into()
}
fn default_socks_port() -> u16 {
    1080
}
fn default_http_port() -> u16 {
    1081
}
fn default_web_addr() -> String {
    "127.0.0.1:8082".into()
}
fn default_admin_user() -> String {
    "root".into()
}
fn default_admin_password() -> String {
    "root".into()
}
fn default_upstream_mode() -> String {
    "direct".into()
}

#[derive(Clone)]
struct AppState {
    config: Arc<RwLock<Config>>,
    stats: Arc<RwLock<Stats>>,
}

#[derive(Debug, Default, Clone, Serialize)]
struct Stats {
    active_connections: usize,
    total_connections: u64,
    bytes_up: u64,
    bytes_down: u64,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt().with_env_filter("info").init();
    let args = Args::parse();
    if matches!(args.command, Some(Command::Update)) {
        return run_update();
    }
    let config = Arc::new(RwLock::new(load_config(&args.config).await?));
    let state = AppState {
        config: config.clone(),
        stats: Arc::new(RwLock::new(Stats::default())),
    };
    let snapshot = config.read().await.clone();
    if snapshot.upstream_mode != "direct" {
        warn!(mode = %snapshot.upstream_mode, "remote protocol routing belongs to the external Mihomo core");
    }

    let socks_addr: SocketAddr = format!("{}:{}", snapshot.listen_addr, snapshot.socks_port)
        .parse()
        .context("invalid SOCKS5 listen address")?;
    let http_addr: SocketAddr = format!("{}:{}", snapshot.listen_addr, snapshot.http_port)
        .parse()
        .context("invalid HTTP listen address")?;
    let socks_listener = TcpListener::bind(socks_addr).await?;
    let http_listener = TcpListener::bind(http_addr).await?;
    info!(%socks_addr, %http_addr, "vpn233 client proxy listeners ready");

    let socks_state = state.clone();
    tokio::spawn(async move {
        loop {
            match socks_listener.accept().await {
                Ok((stream, peer)) => {
                    let state = socks_state.clone();
                    tokio::spawn(async move {
                        if let Err(error) = handle_socks(stream, state).await {
                            warn!(%peer, %error, "SOCKS5 connection failed");
                        }
                    });
                }
                Err(error) => warn!(%error, "SOCKS5 accept failed"),
            }
        }
    });
    let http_state = state.clone();
    tokio::spawn(async move {
        loop {
            match http_listener.accept().await {
                Ok((stream, peer)) => {
                    let state = http_state.clone();
                    tokio::spawn(async move {
                        if let Err(error) = handle_http(stream, state).await {
                            warn!(%peer, %error, "HTTP proxy connection failed");
                        }
                    });
                }
                Err(error) => warn!(%error, "HTTP accept failed"),
            }
        }
    });

    let web_addr: SocketAddr = snapshot
        .web_addr
        .parse()
        .context("invalid client web address")?;
    let app = Router::new()
        .route("/api/v1/health", get(health))
        .route("/api/v1/stats", get(stats))
        .with_state(state);
    let web_listener = TcpListener::bind(web_addr).await?;
    info!(%web_addr, "vpn233 client API ready");
    axum::serve(web_listener, app).await?;
    Ok(())
}

async fn load_config(path: &PathBuf) -> Result<Config> {
    let mut config = if fs::try_exists(path).await? {
        serde_yaml::from_str(&fs::read_to_string(path).await?).context("invalid client YAML")?
    } else {
        Config {
            listen_addr: default_bind(),
            socks_port: default_socks_port(),
            http_port: default_http_port(),
            web_addr: default_web_addr(),
            admin_user: default_admin_user(),
            admin_password: default_admin_password(),
            upstream_mode: default_upstream_mode(),
            subscription_url: String::new(),
            upstream_socks5: String::new(),
        }
    };
    if let Ok(value) = std::env::var("VPN233_CLIENT_ADMIN_USER") {
        config.admin_user = value;
    }
    if let Ok(value) = std::env::var("VPN233_CLIENT_ADMIN_PASSWORD") {
        config.admin_password = value;
    }
    if let Ok(value) = std::env::var("VPN233_CLIENT_UPSTREAM_SOCKS5") {
        config.upstream_socks5 = value;
    }
    if let Ok(value) = std::env::var("VPN233_CLIENT_LISTEN_ADDR") {
        config.listen_addr = value;
    }
    if let Ok(value) = std::env::var("VPN233_CLIENT_SOCKS_PORT") {
        config.socks_port = value.parse().context("invalid VPN233_CLIENT_SOCKS_PORT")?;
    }
    if let Ok(value) = std::env::var("VPN233_CLIENT_HTTP_PORT") {
        config.http_port = value.parse().context("invalid VPN233_CLIENT_HTTP_PORT")?;
    }
    if let Ok(value) = std::env::var("VPN233_CLIENT_WEB_ADDR") {
        config.web_addr = value;
    }
    Ok(config)
}

async fn health(State(state): State<AppState>) -> Json<serde_json::Value> {
    let config = state.config.read().await;
    Json(
        json!({ "ok": true, "service": "vpn233-client", "version": env!("CARGO_PKG_VERSION"), "upstream_mode": config.upstream_mode, "tun": "external-mihomo" }),
    )
}

async fn stats(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if !authorized(&state, &headers).await {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    Json(state.stats.read().await.clone()).into_response()
}

async fn authorized(state: &AppState, headers: &HeaderMap) -> bool {
    let config = state.config.read().await;
    let Some(value) = headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
    else {
        return false;
    };
    let Some(encoded) = value.strip_prefix("Basic ") else {
        return false;
    };
    let Ok(decoded) = base64::engine::general_purpose::STANDARD.decode(encoded) else {
        return false;
    };
    String::from_utf8(decoded).ok().is_some_and(|credentials| {
        credentials == format!("{}:{}", config.admin_user, config.admin_password)
    })
}

fn run_update() -> Result<()> {
    let script = if cfg!(windows) {
        "update.ps1"
    } else {
        "update.sh"
    };
    let path = if let Ok(path) = std::env::var("VPN233_UPDATE_SCRIPT") {
        PathBuf::from(path)
    } else {
        let executable = std::env::current_exe()?;
        executable
            .parent()
            .and_then(|path| path.parent())
            .context("cannot locate installation root")?
            .join(script)
    };
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

async fn handle_http(mut stream: TcpStream, state: AppState) -> Result<()> {
    let mut buffer = vec![0_u8; 16 * 1024];
    let length = stream.read(&mut buffer).await?;
    if length == 0 {
        return Ok(());
    }
    let request = String::from_utf8_lossy(&buffer[..length]);
    let first = request
        .lines()
        .next()
        .context("missing HTTP request line")?;
    let mut parts = first.split_whitespace();
    let method = parts.next().context("missing HTTP method")?;
    let target = parts.next().context("missing HTTP target")?;
    let (host, port, is_connect) = if method.eq_ignore_ascii_case("CONNECT") {
        let (host, port) = split_host_port(target, 443);
        (host, port, true)
    } else {
        let parsed = url::Url::parse(target).context("HTTP proxy requires an absolute URL")?;
        (
            parsed.host_str().context("URL has no host")?.to_owned(),
            parsed.port_or_known_default().unwrap_or(80),
            false,
        )
    };
    let mut remote = connect_target(&state, &host, port).await?;
    mark_open(&state).await;
    if is_connect {
        stream
            .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
            .await?;
    } else {
        remote.write_all(&buffer[..length]).await?;
    }
    let result = copy_counted(&mut stream, &mut remote, &state).await;
    mark_close(&state).await;
    result
}

async fn handle_socks(mut stream: TcpStream, state: AppState) -> Result<()> {
    let mut header = [0_u8; 2];
    stream.read_exact(&mut header).await?;
    if header[0] != 5 {
        anyhow::bail!("unsupported SOCKS version")
    }
    let mut methods = vec![0_u8; header[1] as usize];
    stream.read_exact(&mut methods).await?;
    stream.write_all(&[5, 0]).await?;
    let mut request = [0_u8; 4];
    stream.read_exact(&mut request).await?;
    if request[0] != 5 || request[1] != 1 {
        anyhow::bail!("only SOCKS5 CONNECT is supported")
    }
    let host = match request[3] {
        1 => {
            let mut ip = [0_u8; 4];
            stream.read_exact(&mut ip).await?;
            std::net::Ipv4Addr::from(ip).to_string()
        }
        3 => {
            let mut len = [0_u8; 1];
            stream.read_exact(&mut len).await?;
            let mut name = vec![0_u8; len[0] as usize];
            stream.read_exact(&mut name).await?;
            String::from_utf8(name)?
        }
        4 => {
            let mut ip = [0_u8; 16];
            stream.read_exact(&mut ip).await?;
            std::net::Ipv6Addr::from(ip).to_string()
        }
        _ => anyhow::bail!("unsupported SOCKS address type"),
    };
    let mut port_bytes = [0_u8; 2];
    stream.read_exact(&mut port_bytes).await?;
    let port = u16::from_be_bytes(port_bytes);
    let mut remote = connect_target(&state, &host, port).await?;
    stream.write_all(&[5, 0, 0, 1, 0, 0, 0, 0, 0, 0]).await?;
    mark_open(&state).await;
    let result = copy_counted(&mut stream, &mut remote, &state).await;
    mark_close(&state).await;
    result
}

async fn mark_open(state: &AppState) {
    let mut stats = state.stats.write().await;
    stats.active_connections += 1;
    stats.total_connections += 1;
}
async fn mark_close(state: &AppState) {
    let mut stats = state.stats.write().await;
    stats.active_connections = stats.active_connections.saturating_sub(1);
}
fn split_host_port(value: &str, default_port: u16) -> (String, u16) {
    value
        .rsplit_once(':')
        .and_then(|(host, port)| port.parse().ok().map(|port| (host.to_owned(), port)))
        .unwrap_or_else(|| (value.to_owned(), default_port))
}

async fn connect_target(state: &AppState, host: &str, port: u16) -> Result<TcpStream> {
    let upstream = state.config.read().await.upstream_socks5.clone();
    if upstream.trim().is_empty() {
        return TcpStream::connect((host, port))
            .await
            .context("connect target failed");
    }
    let (proxy_host, proxy_port) = split_host_port(upstream.trim(), 1080);
    let mut stream = TcpStream::connect((proxy_host.as_str(), proxy_port))
        .await
        .context("connect upstream SOCKS5 failed")?;
    stream.write_all(&[5, 1, 0]).await?;
    let mut greeting = [0_u8; 2];
    stream.read_exact(&mut greeting).await?;
    if greeting != [5, 0] {
        anyhow::bail!("upstream SOCKS5 requires no-auth mode")
    }
    let host_bytes = host.as_bytes();
    if host_bytes.len() > u8::MAX as usize {
        anyhow::bail!("target hostname is too long")
    }
    let mut request = vec![5, 1, 0, 3, host_bytes.len() as u8];
    request.extend_from_slice(host_bytes);
    request.extend_from_slice(&port.to_be_bytes());
    stream.write_all(&request).await?;
    let mut response = [0_u8; 4];
    stream.read_exact(&mut response).await?;
    if response[1] != 0 {
        anyhow::bail!("upstream SOCKS5 rejected target with code {}", response[1])
    }
    match response[3] {
        1 => {
            let mut address = [0_u8; 4];
            stream.read_exact(&mut address).await?;
        }
        3 => {
            let mut length = [0_u8; 1];
            stream.read_exact(&mut length).await?;
            let mut address = vec![0_u8; length[0] as usize];
            stream.read_exact(&mut address).await?;
        }
        4 => {
            let mut address = [0_u8; 16];
            stream.read_exact(&mut address).await?;
        }
        _ => anyhow::bail!("upstream SOCKS5 returned an invalid address type"),
    }
    let mut ignored_port = [0_u8; 2];
    stream.read_exact(&mut ignored_port).await?;
    Ok(stream)
}

async fn copy_counted<A, B>(left: &mut A, right: &mut B, state: &AppState) -> Result<()>
where
    A: AsyncRead + AsyncWrite + Unpin,
    B: AsyncRead + AsyncWrite + Unpin,
{
    let (mut left_read, mut left_write) = tokio::io::split(left);
    let (mut right_read, mut right_write) = tokio::io::split(right);
    let up = async {
        let bytes = tokio::io::copy(&mut left_read, &mut right_write).await?;
        right_write.shutdown().await?;
        Ok::<u64, std::io::Error>(bytes)
    };
    let down = async {
        let bytes = tokio::io::copy(&mut right_read, &mut left_write).await?;
        left_write.shutdown().await?;
        Ok::<u64, std::io::Error>(bytes)
    };
    let (up, down) = tokio::try_join!(up, down)?;
    let mut stats = state.stats.write().await;
    stats.bytes_up += up;
    stats.bytes_down += down;
    Ok(())
}
