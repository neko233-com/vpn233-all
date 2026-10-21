//! Service configuration and the YAML loader with environment overrides.

use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use tokio::fs;

/// Top-level configuration for the subscription server.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct Config {
    pub server: ServerConfig,
    #[serde(default)]
    pub subscriptions: Vec<SubscriptionSource>,
}

/// HTTP listener, admin credentials, and upstream relay settings.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ServerConfig {
    #[serde(default = "default_host")]
    pub host: String,
    #[serde(default = "default_port")]
    pub http_port: u16,
    #[serde(default = "default_format")]
    pub default_format: String,
    #[serde(default = "default_data_dir")]
    pub data_dir: String,
    #[serde(default = "default_admin_user")]
    pub admin_user: String,
    #[serde(default = "default_admin_password")]
    pub admin_password: String,
    #[serde(default = "default_admin_origins")]
    pub admin_origins: String,
    #[serde(default = "default_web_dir")]
    pub web_dir: String,
    #[serde(default)]
    pub provider_registry_token: String,
    #[serde(default = "default_provider_stale_after_secs")]
    pub provider_stale_after_secs: u64,
    #[serde(default)]
    pub client_stats_url: String,
    #[serde(default)]
    pub client_stats_user: String,
    #[serde(default)]
    pub client_stats_password: String,
    #[serde(default = "default_traffic_timezone_offset_hours")]
    pub traffic_timezone_offset_hours: i32,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            host: default_host(),
            http_port: default_port(),
            default_format: default_format(),
            data_dir: default_data_dir(),
            admin_user: default_admin_user(),
            admin_password: default_admin_password(),
            admin_origins: default_admin_origins(),
            web_dir: default_web_dir(),
            provider_registry_token: String::new(),
            provider_stale_after_secs: default_provider_stale_after_secs(),
            client_stats_url: String::new(),
            client_stats_user: String::new(),
            client_stats_password: String::new(),
            traffic_timezone_offset_hours: default_traffic_timezone_offset_hours(),
        }
    }
}

/// One upstream subscription source: inline links and/or a remote feed URL.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SubscriptionSource {
    pub name: String,
    #[serde(default)]
    pub links: Vec<String>,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub access_token: String,
}

fn default_host() -> String {
    "0.0.0.0".into()
}
fn default_port() -> u16 {
    23444
}
fn default_format() -> String {
    "clash".into()
}
fn default_data_dir() -> String {
    "./subscribe-server/data".into()
}
fn default_admin_user() -> String {
    "root".into()
}
fn default_admin_password() -> String {
    "root".into()
}
fn default_admin_origins() -> String {
    "http://127.0.0.1:4173,http://localhost:4173".into()
}
fn default_web_dir() -> String {
    "./admin-web/dist".into()
}
fn default_provider_stale_after_secs() -> u64 {
    120
}
fn default_traffic_timezone_offset_hours() -> i32 {
    8
}

/// Loads the YAML configuration, falling back to defaults when the file is absent.
///
/// Environment variables always win over the file so deployments can keep
/// secrets out of tracked configuration.
pub async fn load_config(path: &Path) -> Result<Config> {
    let mut config = if fs::try_exists(path).await? {
        serde_yaml::from_str::<Config>(&fs::read_to_string(path).await?)
            .context("invalid subscribe YAML")?
    } else {
        Config::default()
    };
    if let Ok(user) = std::env::var("VPN233_ADMIN_USER") {
        config.server.admin_user = user;
    }
    if let Ok(password) = std::env::var("VPN233_ADMIN_PASSWORD") {
        config.server.admin_password = password;
    }
    if let Ok(origins) = std::env::var("VPN233_ADMIN_ORIGINS") {
        config.server.admin_origins = origins;
    }
    if let Ok(port) = std::env::var("VPN233_SUBSCRIBE_PORT") {
        config.server.http_port = port.parse().context("invalid VPN233_SUBSCRIBE_PORT")?;
    }
    if let Ok(dir) = std::env::var("VPN233_SUBSCRIBE_DATA_DIR") {
        config.server.data_dir = dir;
    }
    if let Ok(token) = std::env::var("VPN233_PROVIDER_REGISTRY_TOKEN") {
        config.server.provider_registry_token = token;
    }
    if let Ok(stale) = std::env::var("VPN233_PROVIDER_STALE_AFTER_SECS") {
        config.server.provider_stale_after_secs = stale
            .parse()
            .context("invalid VPN233_PROVIDER_STALE_AFTER_SECS")?;
    }
    if let Ok(url) = std::env::var("VPN233_CLIENT_STATS_URL") {
        config.server.client_stats_url = url;
    }
    if let Ok(user) = std::env::var("VPN233_CLIENT_STATS_USER") {
        config.server.client_stats_user = user;
    }
    if let Ok(password) = std::env::var("VPN233_CLIENT_STATS_PASSWORD") {
        config.server.client_stats_password = password;
    }
    if let Ok(offset) = std::env::var("VPN233_TRAFFIC_TIMEZONE_OFFSET_HOURS") {
        config.server.traffic_timezone_offset_hours = offset
            .parse()
            .context("invalid VPN233_TRAFFIC_TIMEZONE_OFFSET_HOURS")?;
    }
    Ok(config)
}
