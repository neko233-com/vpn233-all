//! Local HTTP/SOCKS5 proxy. Encrypted protocols and TUN require an external core.

pub mod auth;
pub mod config;
pub mod http_proxy;
pub mod socks;
pub mod state;
pub mod upstream;

use anyhow::Result;
use axum::{
    extract::State,
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
    Json, Router,
};
use serde_json::json;
use std::sync::Arc;
use tokio::net::TcpListener;

pub use config::Config;
pub use state::AppState;

/// Builds the local management API; statistics always require Basic auth.
pub fn build_app(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/api/v1/health", get(health))
        .route("/api/v1/stats", get(stats))
        .route("/api/v1/config", get(config_view))
        .route("/proxy.pac", get(pac))
        .with_state(state)
}

/// Validates configuration and starts all three local listeners.
pub async fn run(config: Config) -> Result<()> {
    config.validate()?;
    let socks_listener = TcpListener::bind(config.socks_addr()?).await?;
    let http_listener = TcpListener::bind(config.http_addr()?).await?;
    let web_listener = TcpListener::bind(config.web_socket_addr()?).await?;
    let state = Arc::new(AppState::new(config).await?);
    let socks_state = state.clone();
    let socks_task = tokio::spawn(async move {
        loop {
            let (stream, peer) = match socks_listener.accept().await {
                Ok(connection) => connection,
                Err(_) => {
                    tracing::warn!("SOCKS5 accept failed");
                    continue;
                }
            };
            let state = socks_state.clone();
            tokio::spawn(async move {
                let _ = socks::handle_socks(stream, peer, state).await;
            });
        }
    });
    let http_state = state.clone();
    let http_task = tokio::spawn(async move {
        loop {
            let (stream, peer) = match http_listener.accept().await {
                Ok(connection) => connection,
                Err(_) => {
                    tracing::warn!("HTTP accept failed");
                    continue;
                }
            };
            let state = http_state.clone();
            tokio::spawn(async move {
                let _ = http_proxy::handle_http(stream, peer, state).await;
            });
        }
    });
    tracing::info!("vpn233 client HTTP, SOCKS5 and API listeners ready");
    let result = axum::serve(web_listener, build_app(state.clone()))
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await;
    socks_task.abort();
    http_task.abort();
    state.flush().await?;
    result?;
    Ok(())
}

async fn health(State(state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    let config = state.config_snapshot().await;
    Json(json!({
        "ok": true, "service": "vpn233-client", "version": env!("CARGO_PKG_VERSION"),
        "upstream_mode": config.upstream_mode, "tun": "external-mihomo",
        "record_domains": config.record_domains,
        "capabilities": ["socks5-connect", "http-connect", "http-forward", "socks5-upstream", "pac"],
    }))
}

async fn stats(State(state): State<Arc<AppState>>, headers: HeaderMap) -> Response {
    let config = state.config_snapshot().await;
    if !auth::basic_auth_matches(&headers, &config.admin_user, &config.admin_password) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let mut response = Json(state.stats_snapshot().await).into_response();
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("no-store, private"),
    );
    response
}

async fn pac(State(state): State<Arc<AppState>>) -> Response {
    let config = state.config_snapshot().await;
    let body = if config.pac.is_empty() {
        let proxy = serde_json::to_string(&format!(
            "SOCKS5 {}:{}; PROXY {}:{}",
            config.pac_proxy_host, config.socks_port, config.pac_proxy_host, config.http_port
        ))
        .unwrap_or_default();
        format!("function FindProxyForURL(url, host) {{\n  if (isPlainHostName(host) || host === 'localhost' || shExpMatch(host, '*.local')) return 'DIRECT';\n  return {proxy};\n}}\n")
    } else {
        config.pac
    };
    (
        [
            (
                header::CONTENT_TYPE,
                "application/x-ns-proxy-autoconfig; charset=utf-8",
            ),
            (header::CACHE_CONTROL, "no-store"),
        ],
        body,
    )
        .into_response()
}

async fn config_view(State(state): State<Arc<AppState>>, headers: HeaderMap) -> Response {
    let config = state.config_snapshot().await;
    if !auth::basic_auth_matches(&headers, &config.admin_user, &config.admin_password) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let mut response = Json(json!({
        "listen_addr": config.listen_addr, "socks_port": config.socks_port,
        "http_port": config.http_port, "web_addr": config.web_addr,
        "upstream_mode": config.upstream_mode, "upstream_configured": !config.upstream_socks5.is_empty(),
        "socks_auth_enabled": config.socks_auth_enabled(), "http_auth_enabled": config.http_auth_enabled(),
        "record_domains": config.record_domains, "stats_timezone_offset_hours": config.stats_timezone_offset_hours,
    })).into_response();
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("no-store, private"),
    );
    response
}
