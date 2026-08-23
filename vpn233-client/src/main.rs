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
use std::{
    collections::BTreeMap,
    net::{IpAddr, SocketAddr},
    path::PathBuf,
    sync::Arc,
};
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
    #[serde(default = "default_stats_file")]
    stats_file: PathBuf,
    #[serde(default = "default_stats_timezone_offset_hours")]
    stats_timezone_offset_hours: i32,
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
fn default_stats_file() -> PathBuf {
    "vpn233-client/data/stats.json".into()
}
fn default_stats_timezone_offset_hours() -> i32 {
    8
}

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[derive(Clone)]
struct AppState {
    config: Arc<RwLock<Config>>,
    stats: Arc<RwLock<Stats>>,
    stats_file: PathBuf,
    stats_timezone_offset_hours: i32,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
struct Stats {
    active_connections: usize,
    total_connections: u64,
    bytes_up: u64,
    bytes_down: u64,
    #[serde(default)]
    daily: BTreeMap<String, TrafficBucket>,
    #[serde(default)]
    domains: BTreeMap<String, DomainBucket>,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
struct TrafficBucket {
    upload_bytes: u64,
    download_bytes: u64,
    requests: u64,
    connections: u64,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
struct DomainBucket {
    upload_bytes: u64,
    download_bytes: u64,
    requests: u64,
    last_seen: u64,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt().with_env_filter("info").init();
    let args = Args::parse();
    if matches!(args.command, Some(Command::Update)) {
        return run_update();
    }
    let config = Arc::new(RwLock::new(load_config(&args.config).await?));
    let stats_file = config.read().await.stats_file.clone();
    let mut loaded_stats = load_stats(&stats_file).await.unwrap_or_default();
    loaded_stats.active_connections = 0;
    let state = AppState {
        config: config.clone(),
        stats: Arc::new(RwLock::new(loaded_stats)),
        stats_file,
        stats_timezone_offset_hours: config.read().await.stats_timezone_offset_hours,
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
            stats_file: default_stats_file(),
            stats_timezone_offset_hours: default_stats_timezone_offset_hours(),
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
    if let Ok(value) = std::env::var("VPN233_CLIENT_STATS_FILE") {
        config.stats_file = PathBuf::from(value);
    }
    if let Ok(value) = std::env::var("VPN233_CLIENT_STATS_TIMEZONE_OFFSET_HOURS") {
        config.stats_timezone_offset_hours = value
            .parse()
            .context("invalid VPN233_CLIENT_STATS_TIMEZONE_OFFSET_HOURS")?;
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
    let result = copy_counted(&mut stream, &mut remote, &state, &host).await;
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
    let result = copy_counted(&mut stream, &mut remote, &state, &host).await;
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

async fn load_stats(path: &PathBuf) -> Result<Stats> {
    if !fs::try_exists(path).await? {
        return Ok(Stats::default());
    }
    Ok(serde_json::from_slice(&fs::read(path).await?)?)
}

async fn save_stats(state: &AppState) -> Result<()> {
    if let Some(parent) = state.stats_file.parent() {
        fs::create_dir_all(parent).await?;
    }
    let tmp = state.stats_file.with_extension("json.tmp");
    fs::write(&tmp, serde_json::to_vec_pretty(&*state.stats.read().await)?).await?;
    fs::rename(tmp, &state.stats_file).await?;
    Ok(())
}

fn date_key(timestamp: u64, offset_hours: i32) -> String {
    let shifted = timestamp as i64 + i64::from(offset_hours) * 3_600;
    let days = shifted.div_euclid(86_400);
    let z = days + 719_468;
    let era = (if z >= 0 { z } else { z - 146_096 }) / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let mut year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let month_part = (5 * doy + 2) / 153;
    let day = doy - (153 * month_part + 2) / 5 + 1;
    let month = month_part + if month_part < 10 { 3 } else { -9 };
    year += i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}")
}

fn normalize_destination(host: &str) -> Option<String> {
    let value = host.trim().trim_end_matches('.').to_ascii_lowercase();
    if value.is_empty() || value == "localhost" {
        return None;
    }
    if value.parse::<IpAddr>().is_ok() {
        return Some(format!("[IP] {value}"));
    }
    Some(value.chars().take(253).collect())
}

async fn record_usage(state: &AppState, host: &str, upload_bytes: u64, download_bytes: u64) {
    let now = unix_now();
    let mut stats = state.stats.write().await;
    stats.bytes_up = stats.bytes_up.saturating_add(upload_bytes);
    stats.bytes_down = stats.bytes_down.saturating_add(download_bytes);
    stats.total_connections = stats.total_connections.saturating_add(1);
    let day = stats
        .daily
        .entry(date_key(now, state.stats_timezone_offset_hours))
        .or_default();
    day.upload_bytes = day.upload_bytes.saturating_add(upload_bytes);
    day.download_bytes = day.download_bytes.saturating_add(download_bytes);
    day.requests = day.requests.saturating_add(1);
    day.connections = day.connections.saturating_add(1);
    while stats.daily.len() > 400 {
        let Some(first) = stats.daily.keys().next().cloned() else {
            break;
        };
        stats.daily.remove(&first);
    }
    if let Some(domain) = normalize_destination(host) {
        let bucket = stats.domains.entry(domain).or_default();
        bucket.upload_bytes = bucket.upload_bytes.saturating_add(upload_bytes);
        bucket.download_bytes = bucket.download_bytes.saturating_add(download_bytes);
        bucket.requests = bucket.requests.saturating_add(1);
        bucket.last_seen = now;
    }
    while stats.domains.len() > 2_000 {
        let Some(first) = stats.domains.keys().next().cloned() else {
            break;
        };
        stats.domains.remove(&first);
    }
    drop(stats);
    if let Err(error) = save_stats(state).await {
        warn!(%error, "traffic stats persistence failed");
    }
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

async fn copy_counted<A, B>(left: &mut A, right: &mut B, state: &AppState, host: &str) -> Result<()>
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
    record_usage(state, host, up, down).await;
    Ok(())
}
