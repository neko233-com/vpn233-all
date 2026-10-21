//! vpn233 provider-server library: state, HTTP API, registry heartbeat and the
//! generated one-shot node deployment scripts.

pub mod auth;
pub mod config;
pub mod manifest;
pub mod registry;
pub mod scripts;

pub use config::Config;
pub use manifest::{
    format_capabilities, supported_protocols, GenerateQuery, NodeManifest, NodePort,
};

use anyhow::{Context, Result};
use auth::{constant_time_eq, credentials_match, AuthState};
use axum::{
    extract::{Query, State},
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use registry::RegistryReporter;
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    path::Path,
    process::Command as ProcessCommand,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::{
    net::TcpListener,
    sync::RwLock,
    time::{interval, MissedTickBehavior},
};
use tracing::{info, warn};

/// Shared provider state.
#[derive(Clone)]
pub struct AppState {
    pub config: Arc<RwLock<Config>>,
    pub auth: Arc<RwLock<AuthState>>,
}

/// Build the shared state from a validated configuration.
pub async fn init_state(config: Config) -> AppState {
    AppState {
        config: Arc::new(RwLock::new(config)),
        auth: Arc::new(RwLock::new(AuthState::default())),
    }
}

/// Assemble the admin/provider HTTP router.
pub fn build_app(state: AppState) -> Router {
    Router::new()
        .route("/api/v1/health", get(health))
        .route("/api/v1/login", post(login))
        .route("/api/v1/logout", post(logout))
        .route("/api/v1/protocols", get(protocols))
        .route("/api/v1/formats", get(formats))
        .route("/api/v1/config", get(config_view))
        .route("/api/v1/generate", get(generate))
        .route("/api/v1/generate.sh", get(generate_sh))
        .route("/api/v1/generate.ps1", get(generate_ps1))
        .route("/api/v1/generate.links", get(generate_links))
        .route("/api/v1/nodes", get(nodes))
        .route("/api/v1/subscribe/verify", get(subscribe_verify))
        .with_state(state)
}

/// Load the configuration, initialise state, start the registry reporter and
/// serve the API until the process is asked to stop.
pub async fn run(config_path: &Path) -> Result<()> {
    let config = Config::load(config_path).await?;
    config.validate()?;
    warn_insecure_defaults(&config);
    let addr = config.socket_addr()?;
    let state = init_state(config.clone()).await;
    let app = build_app(state.clone());

    match RegistryReporter::new(&config) {
        Ok(Some(reporter)) => {
            tokio::spawn(async move { reporter.run().await });
        }
        Ok(None) => {}
        Err(error) => warn!(%error, "provider registry registration disabled"),
    }
    tokio::spawn(cleanup_sessions(state.clone()));

    let listener = TcpListener::bind(addr)
        .await
        .with_context(|| format!("cannot bind provider server to {addr}"))?;
    info!(%addr, "vpn233 provider server listening");
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .context("provider server terminated unexpectedly")?;
    Ok(())
}

/// Write the example configuration when `--init` is used.
pub async fn init_config(path: &Path) -> Result<()> {
    if tokio::fs::try_exists(path).await.unwrap_or(false) {
        anyhow::bail!("refusing to overwrite existing config: {}", path.display());
    }
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            tokio::fs::create_dir_all(parent).await?;
        }
    }
    tokio::fs::write(path, include_str!("../config.example.yaml")).await?;
    println!("created {}", path.display());
    Ok(())
}

/// Run the platform update script shipped next to the executable.
pub fn run_update() -> Result<()> {
    let script = if cfg!(windows) {
        "update.ps1"
    } else {
        "update.sh"
    };
    let path = if let Ok(path) = std::env::var("VPN233_UPDATE_SCRIPT") {
        std::path::PathBuf::from(path)
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
            .arg(&path)
            .status()?
    } else {
        ProcessCommand::new("sh").arg(&path).status()?
    };
    anyhow::ensure!(status.success(), "update script failed with {status}");
    Ok(())
}

pub fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn warn_insecure_defaults(config: &Config) {
    if config.admin_password == "root" {
        warn!(
            "provider admin password is the insecure default 'root'; set VPN233_ADMIN_PASSWORD or edit the untracked config.yaml before exposing the service"
        );
    }
    if config.provider_access_token.trim().is_empty() {
        warn!("provider_access_token is empty; the /api/v1/nodes feed is disabled");
    }
    if config.registry_url.trim().is_empty() {
        info!("no registry_url configured; registry heartbeat disabled");
    }
}

async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let terminate = async {
        if let Ok(mut signal) =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        {
            signal.recv().await;
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
    info!("provider server shutting down");
}

async fn cleanup_sessions(state: AppState) {
    let mut ticker = interval(Duration::from_secs(300));
    ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
    loop {
        ticker.tick().await;
        state.auth.write().await.cleanup(now_secs());
    }
}

async fn health() -> Json<Value> {
    Json(json!({
        "ok": true,
        "service": "vpn233-provider-server",
        "version": env!("CARGO_PKG_VERSION"),
    }))
}

#[derive(Debug, Deserialize)]
struct LoginRequest {
    username: String,
    password: String,
}

async fn login(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<LoginRequest>,
) -> Response {
    let now = now_secs();
    let client = auth::client_key(&headers);
    let (rate_limit, ttl) = {
        let config = state.config.read().await;
        (config.login_rate_limit, config.session_ttl_secs)
    };
    {
        let mut auth = state.auth.write().await;
        auth.cleanup(now);
        if auth.is_rate_limited(&client, now, rate_limit) {
            return (
                StatusCode::TOO_MANY_REQUESTS,
                Json(json!({"error": "too many failed login attempts"})),
            )
                .into_response();
        }
    }
    let config = state.config.read().await.clone();
    if !credentials_match(&input.username, &input.password, &config) {
        state
            .auth
            .write()
            .await
            .record_failure(&client, now, rate_limit);
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({"error": "invalid credentials"})),
        )
            .into_response();
    }
    let token = state
        .auth
        .write()
        .await
        .create_session(&config.admin_user, ttl, now);
    Json(json!({
        "token": token,
        "token_type": "Bearer",
        "expires_in": ttl,
    }))
    .into_response()
}

async fn logout(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let Some(token) = bearer_token(&headers) else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    let mut auth = state.auth.write().await;
    if auth.validate(token, now_secs()).is_none() {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    auth.remove(token);
    Json(json!({"ok": true})).into_response()
}

async fn protocols() -> Json<Value> {
    let capabilities = format_capabilities();
    Json(json!({
        "protocols": supported_protocols(),
        "mihomo": supported_protocols(),
        "sing_box": supported_protocols(),
        "formats": capabilities,
        "capability_matrix": capability_matrix(&capabilities),
        "note": "The generated node config targets Mihomo; running the core remains the boundary of the external mihomo binary.",
    }))
}

async fn formats() -> Response {
    let capabilities = format_capabilities();
    let payload = json!({
        "formats": capabilities,
        "protocols": supported_protocols(),
        "capability_matrix": capability_matrix(&capabilities),
    });
    let mut response = Json(payload).into_response();
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("public, max-age=300"),
    );
    response
}

fn capability_matrix(capabilities: &[manifest::FormatCapability]) -> Value {
    let mut matrix = serde_json::Map::new();
    for capability in capabilities {
        matrix.insert(
            capability.id.clone(),
            Value::Array(
                capability
                    .protocols
                    .iter()
                    .map(|protocol| Value::String(protocol.clone()))
                    .collect(),
            ),
        );
    }
    Value::Object(matrix)
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
        "mihomo_url_configured": !config.mihomo_url.is_empty(),
        "mihomo_version": config.mihomo_version,
        "mihomo_sha256_configured": !config.mihomo_sha256.is_empty(),
        "default_port_base": config.default_port_base,
        "node_root": config.node_root,
        "enabled_protocols": manifest::supported_protocols().iter()
            .filter(|protocol| config.is_protocol_enabled(protocol))
            .map(|protocol| (*protocol).to_owned())
            .collect::<Vec<_>>(),
        "download_mirrors": config.mirrors(),
        "session_ttl_secs": config.session_ttl_secs,
        "login_rate_limit": config.login_rate_limit,
        "subscribe_verify_token_configured": !config.subscribe_verify_token.is_empty(),
        "registry_configured": !config.registry_url.is_empty(),
    }))
    .into_response()
}

async fn generate(
    headers: HeaderMap,
    State(state): State<AppState>,
    Query(query): Query<GenerateQuery>,
) -> Response {
    let format = query.format.clone().unwrap_or_else(|| "json".to_owned());
    generate_formatted(&state, &headers, &query, &format).await
}

async fn generate_sh(
    headers: HeaderMap,
    State(state): State<AppState>,
    Query(query): Query<GenerateQuery>,
) -> Response {
    generate_formatted(&state, &headers, &query, "sh").await
}

async fn generate_ps1(
    headers: HeaderMap,
    State(state): State<AppState>,
    Query(query): Query<GenerateQuery>,
) -> Response {
    generate_formatted(&state, &headers, &query, "ps1").await
}

async fn generate_links(
    headers: HeaderMap,
    State(state): State<AppState>,
    Query(query): Query<GenerateQuery>,
) -> Response {
    generate_formatted(&state, &headers, &query, "links").await
}

async fn generate_formatted(
    state: &AppState,
    headers: &HeaderMap,
    query: &GenerateQuery,
    format: &str,
) -> Response {
    if !authorized(state, headers).await {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let config = state.config.read().await.clone();
    let manifest = NodeManifest::build(&config, query);
    match format.to_ascii_lowercase().as_str() {
        "json" => Json(json!({
            "node": manifest,
            "links": manifest.link_lines(),
            "sh": scripts::render_sh(&manifest, &config),
            "ps1": scripts::render_ps1(&manifest, &config),
            "deployment": deployment_view(&config, &manifest),
        }))
        .into_response(),
        "links" => plain_script(manifest.links_text(), "vpn233-node.links.txt"),
        "sh" => plain_script(scripts::render_sh(&manifest, &config), "vpn233-node.sh"),
        "ps1" => plain_script(scripts::render_ps1(&manifest, &config), "vpn233-node.ps1"),
        _ => (
            StatusCode::BAD_REQUEST,
            "format must be one of json, links, sh or ps1",
        )
            .into_response(),
    }
}

fn deployment_view(config: &Config, manifest: &NodeManifest) -> Value {
    json!({
        "service_name": "vpn233-node",
        "systemd_unit": "/etc/systemd/system/vpn233-node.service",
        "scheduled_task": "VPN233-Node",
        "node_root": config.node_root,
        "windows_root": "%ProgramData%\\vpn233",
        "mihomo_version": config.mihomo_version,
        "mirrors": config.mirrors(),
        "protocols": manifest.ports.iter().map(|port| port.protocol.clone()).collect::<Vec<_>>(),
        "link_count": manifest.link_lines().len(),
    })
}

async fn nodes(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<HashMap<String, String>>,
) -> Response {
    let config = state.config.read().await.clone();
    let expected = config.provider_access_token.trim();
    if expected.is_empty() {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let supplied = query.get("token").cloned().or_else(|| {
        headers
            .get("x-provider-token")
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned)
    });
    let allowed = supplied
        .as_deref()
        .is_some_and(|token| constant_time_eq(token.trim(), expected));
    if !allowed {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    if !config.node_links.is_empty() {
        let mut body = config.node_links.join("\n");
        body.push('\n');
        return plain_script(body, "vpn233-nodes.txt");
    }
    let manifest = NodeManifest::build(&config, &GenerateQuery::default());
    plain_script(manifest.links_text(), "vpn233-nodes.txt")
}

async fn subscribe_verify(
    State(state): State<AppState>,
    Query(query): Query<HashMap<String, String>>,
) -> Response {
    let config = state.config.read().await.clone();
    if !config.subscribe_verify_token.is_empty()
        && query.get("token") != Some(&config.subscribe_verify_token)
    {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    Json(json!({
        "ok": true,
        "service": "vpn233-provider-server",
        "version": env!("CARGO_PKG_VERSION"),
        "protocols": supported_protocols(),
        "enabled_protocols": manifest::supported_protocols().iter()
            .filter(|protocol| config.is_protocol_enabled(protocol))
            .map(|protocol| (*protocol).to_owned())
            .collect::<Vec<_>>(),
    }))
    .into_response()
}

fn bearer_token(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
}

async fn authorized(state: &AppState, headers: &HeaderMap) -> bool {
    let Some(token) = bearer_token(headers) else {
        return false;
    };
    let now = now_secs();
    state.auth.write().await.validate(token, now).is_some()
}

fn plain_script(body: String, filename: &str) -> Response {
    let mut headers = HeaderMap::new();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/plain; charset=utf-8"),
    );
    headers.insert(
        header::CONTENT_DISPOSITION,
        HeaderValue::from_str(&format!("attachment; filename=\"{filename}\""))
            .unwrap_or_else(|_| HeaderValue::from_static("attachment")),
    );
    headers.insert(
        "x-content-type-options",
        HeaderValue::from_static("nosniff"),
    );
    (headers, body).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn now_secs_is_reasonable() {
        assert!(now_secs() > 1_600_000_000);
    }

    #[test]
    fn capability_matrix_covers_every_format() {
        let capabilities = format_capabilities();
        let matrix = capability_matrix(&capabilities);
        assert_eq!(
            matrix.as_object().map(serde_json::Map::len),
            Some(capabilities.len())
        );
        for capability in &capabilities {
            assert!(matrix.get(&capability.id).is_some());
        }
    }
}
