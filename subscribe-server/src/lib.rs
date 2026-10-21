//! vpn233 subscription aggregation service.
//!
//! The crate exposes a small library surface (`Config`, `AppState`,
//! [`init_state`], [`build_app`], [`run`]) so the Axum router can be exercised
//! by integration tests without binding a TCP port or touching real runtime
//! data.

mod admin;
mod auth;
mod config;
mod formats;
mod jwt_crypto;
mod models;
mod quota;
mod registry;
mod state;
mod subscription;
mod traffic;
mod update;
mod util;

use std::net::SocketAddr;
use std::path::PathBuf;

use anyhow::{Context, Result};
use axum::{
    http::{header, HeaderName, HeaderValue, Method},
    routing::{get, patch, post},
    Router,
};
use tower_http::{cors::CorsLayer, services::ServeDir, set_header::SetResponseHeaderLayer};
use tracing::info;

pub use config::{Config, ServerConfig, SubscriptionSource};
pub use state::AppState;

/// Security headers applied to every response (API, subscriptions, admin UI).
const CONTENT_SECURITY_POLICY: &str = "default-src 'self'; style-src 'self' 'unsafe-inline'; \
     img-src 'self' data:; connect-src 'self'; frame-ancestors 'none'";

/// Loads every persisted store and builds the shared application state.
pub async fn init_state(config: Config) -> Result<AppState> {
    jwt_crypto::install();
    state::init_state(config).await
}

/// Builds the Axum router. Tests use this directly with [`init_state`].
pub fn build_app(state: AppState) -> Router {
    let web_dir = state.web_dir.as_ref().clone();
    let cors = CorsLayer::new()
        .allow_origin(state.cors_origins.as_ref().clone())
        .allow_methods([Method::GET, Method::POST, Method::PATCH, Method::DELETE])
        .allow_headers([header::AUTHORIZATION, header::CONTENT_TYPE]);
    Router::new()
        .route("/sub", get(subscription::subscription_default))
        .route("/sub/c", get(subscription::subscription_clash))
        .route("/sub/c/{name}", get(subscription::subscription_clash_named))
        // Explicit per-format routes (aliases included) to avoid wildcard
        // ambiguity with /sub/c/{name}. The handler resolves the format from
        // the request path, so all of them share one implementation.
        .route("/sub/clash", get(subscription::subscription_format_route))
        .route("/sub/mihomo", get(subscription::subscription_format_route))
        .route("/sub/singbox", get(subscription::subscription_format_route))
        .route(
            "/sub/sing-box",
            get(subscription::subscription_format_route),
        )
        .route("/sub/v2ray", get(subscription::subscription_format_route))
        .route("/sub/v2rayn", get(subscription::subscription_format_route))
        .route("/sub/surge", get(subscription::subscription_format_route))
        .route(
            "/sub/quantumultx",
            get(subscription::subscription_format_route),
        )
        .route("/sub/qx", get(subscription::subscription_format_route))
        .route("/sub/loon", get(subscription::subscription_format_route))
        .route(
            "/sub/surfboard",
            get(subscription::subscription_format_route),
        )
        .route("/sub/stash", get(subscription::subscription_format_route))
        .route("/sub/hiddify", get(subscription::subscription_format_route))
        .route(
            "/sub/shadowrocket",
            get(subscription::subscription_format_route),
        )
        .route("/sub/nekobox", get(subscription::subscription_format_route))
        // Kept as stable legacy paths; /sub/base64 is the v2ray format.
        .route("/sub/links", get(subscription::subscription_format_route))
        .route("/sub/base64", get(subscription::subscription_format_route))
        // Unknown /sub/<format> ids are a client error instead of a 404.
        .route("/sub/{format}", get(subscription::subscription_any_format))
        .route("/api/v1/formats", get(formats::formats))
        .route("/api/auth/login", post(admin::admin_login))
        .route("/api/status", get(admin::status))
        .route("/api/nodes", get(admin::nodes))
        .route("/api/refresh", get(admin::refresh))
        .route(
            "/api/v1/registry/register",
            post(registry::registry_register),
        )
        .route(
            "/api/v1/registry/heartbeat",
            post(registry::registry_heartbeat),
        )
        .route(
            "/api/v1/registry/unregister",
            post(registry::registry_unregister),
        )
        .route("/api/admin/providers", get(admin::admin_providers))
        .route(
            "/api/admin/providers/{provider_id}/disable",
            post(admin::admin_disable_provider),
        )
        .route(
            "/api/admin/tokens",
            get(admin::admin_tokens).post(admin::admin_create_token),
        )
        .route("/api/admin/traffic", get(traffic::admin_traffic))
        .route(
            "/api/admin/accounts",
            get(admin::admin_accounts).post(admin::admin_create_account),
        )
        .route(
            "/api/admin/accounts/{account_id}",
            patch(admin::admin_update_account).delete(admin::admin_delete_account),
        )
        .route(
            "/api/admin/accounts/{account_id}/links",
            get(admin::admin_account_links).post(admin::admin_create_account_link),
        )
        .route(
            "/api/admin/accounts/{account_id}/links/{token}",
            patch(admin::admin_update_account_link).delete(admin::admin_delete_account_link),
        )
        .route(
            "/api/admin/accounts/{account_id}/links/{token}/rotate",
            post(admin::admin_rotate_account_link),
        )
        .route(
            "/api/admin/groups",
            get(admin::admin_groups).post(admin::admin_create_group),
        )
        .route(
            "/api/admin/groups/{group_id}",
            patch(admin::admin_update_group).delete(admin::admin_delete_group),
        )
        .route(
            "/api/admin/tokens/{token}",
            post(admin::admin_disable_token).delete(admin::admin_delete_token),
        )
        .route(
            "/api/admin/tokens/{token}/renew",
            post(admin::admin_renew_token),
        )
        .route(
            "/api/admin/tokens/{token}/reset",
            post(admin::admin_reset_token),
        )
        .route(
            "/api/admin/tokens/{token}/rotate",
            post(admin::admin_rotate_token),
        )
        .route(
            "/api/admin/tokens/{token}/link",
            post(admin::admin_subscription_link),
        )
        .fallback_service(ServeDir::new(web_dir))
        .layer(cors)
        .layer(SetResponseHeaderLayer::overriding(
            header::X_CONTENT_TYPE_OPTIONS,
            HeaderValue::from_static("nosniff"),
        ))
        .layer(SetResponseHeaderLayer::overriding(
            header::X_FRAME_OPTIONS,
            HeaderValue::from_static("DENY"),
        ))
        .layer(SetResponseHeaderLayer::overriding(
            header::REFERRER_POLICY,
            HeaderValue::from_static("no-referrer"),
        ))
        .layer(SetResponseHeaderLayer::overriding(
            HeaderName::from_static("permissions-policy"),
            HeaderValue::from_static("interest-cohort=()"),
        ))
        .layer(SetResponseHeaderLayer::overriding(
            header::CONTENT_SECURITY_POLICY,
            HeaderValue::from_static(CONTENT_SECURITY_POLICY),
        ))
        .with_state(state)
}

/// Loads config, binds the listener, and serves with graceful shutdown.
pub async fn run(config_path: PathBuf) -> Result<()> {
    let config = config::load_config(&config_path).await?;
    let addr: SocketAddr = format!("{}:{}", config.server.host, config.server.http_port)
        .parse()
        .context("invalid subscribe listen address")?;
    let state = state::init_state(config).await?;
    state::spawn_usage_reaper(state.clone());
    let app = build_app(state);
    let listener = tokio::net::TcpListener::bind(addr).await?;
    info!(%addr, "vpn233 subscribe server listening");
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal())
    .await?;
    Ok(())
}

/// Self-update entry point used by the `update` CLI subcommand.
pub async fn update_self() -> Result<()> {
    update::run_update()
}

async fn shutdown_signal() {
    let ctrl_c = async {
        if let Err(error) = tokio::signal::ctrl_c().await {
            tracing::warn!(%error, "failed to install Ctrl+C handler");
        }
    };
    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut signal) => {
                signal.recv().await;
            }
            Err(error) => {
                tracing::warn!(%error, "failed to install SIGTERM handler");
                std::future::pending::<()>().await;
            }
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::select! {
        () = ctrl_c => {},
        () = terminate => {},
    }
    info!("shutdown signal received, stopping subscribe server");
}

#[cfg(test)]
mod tests {
    use super::CONTENT_SECURITY_POLICY;

    #[test]
    fn csp_locks_down_frames_and_sources() {
        assert!(CONTENT_SECURITY_POLICY.contains("frame-ancestors 'none'"));
        assert!(CONTENT_SECURITY_POLICY.contains("default-src 'self'"));
        assert!(CONTENT_SECURITY_POLICY.contains("connect-src 'self'"));
        assert!(CONTENT_SECURITY_POLICY.contains("'unsafe-inline'"));
    }
}
