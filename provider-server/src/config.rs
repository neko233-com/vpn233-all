//! Provider server configuration, environment overrides and validation.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use std::path::Path;

/// GitHub mirror prefixes used by the generated node deployment scripts.
///
/// This list is a repository-wide contract: it must stay in the same order as
/// `scripts/lib/mirrors.sh`, `scripts/lib/mirrors.ps1` and the documentation.
/// The last entry is the direct (unmirrored) connection.
pub const MIRROR_PREFIXES: [&str; 5] = [
    "https://ghproxy.net/",
    "https://gh.llkk.cc/",
    "https://ghfast.top/",
    "https://gh-proxy.com/",
    "",
];

/// Default pinned Mihomo release used by the generated node scripts.
pub const DEFAULT_MIHOMO_VERSION: &str = "v1.19.33";
/// Default admin session TTL: 12 hours.
pub const DEFAULT_SESSION_TTL_SECS: u64 = 12 * 60 * 60;
/// Default number of failed logins allowed inside [`LOGIN_RATE_WINDOW_SECS`].
pub const DEFAULT_LOGIN_RATE_LIMIT: u64 = 10;
/// Failed login window in seconds.
pub const LOGIN_RATE_WINDOW_SECS: u64 = 60;
/// Default node root used by the generated bash deployment script.
pub const DEFAULT_NODE_ROOT: &str = "/etc/vpn233";
/// Local mixed port written to the generated Mihomo configuration.
pub const DEFAULT_MIXED_PORT: u16 = 7890;

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
fn default_port_base() -> u16 {
    10000
}
fn default_heartbeat_interval_secs() -> u64 {
    30
}
fn default_mihomo_version() -> String {
    DEFAULT_MIHOMO_VERSION.into()
}
fn default_node_root() -> String {
    DEFAULT_NODE_ROOT.into()
}
fn default_session_ttl_secs() -> u64 {
    DEFAULT_SESSION_TTL_SECS
}
fn default_login_rate_limit() -> u64 {
    DEFAULT_LOGIN_RATE_LIMIT
}
fn default_auto_detect_public_ip() -> bool {
    true
}
fn default_download_mirrors() -> Vec<String> {
    MIRROR_PREFIXES
        .iter()
        .map(|prefix| (*prefix).to_owned())
        .collect()
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AdvertiseAddress {
    pub kind: String,
    pub value: String,
}

/// Persisted provider configuration. Secrets are only ever read from an
/// untracked `config.yaml`, an environment variable or a password file.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Config {
    #[serde(default = "default_listen_addr")]
    pub listen_addr: String,
    #[serde(default = "default_provider_port")]
    pub listen_port: u16,
    #[serde(default = "default_admin_user")]
    pub admin_user: String,
    #[serde(default = "default_admin_password")]
    pub admin_password: String,
    #[serde(default = "default_node_ip")]
    pub default_node_ip: String,
    #[serde(default)]
    pub provider_access_token: String,
    /// Optional explicit Mihomo release asset URL. When empty the generated
    /// scripts build the URL from `mihomo_version` and the target platform.
    #[serde(default)]
    pub mihomo_url: String,
    #[serde(default)]
    pub mihomo_sha256: String,
    #[serde(default = "default_mihomo_version")]
    pub mihomo_version: String,
    #[serde(default = "default_port_base")]
    pub default_port_base: u16,
    #[serde(default)]
    pub subscribe_verify_token: String,
    #[serde(default)]
    pub node_links: Vec<String>,
    #[serde(default)]
    pub registry_url: String,
    #[serde(default)]
    pub registry_token: String,
    #[serde(default)]
    pub registry_provider_id: String,
    #[serde(default)]
    pub registry_name: String,
    #[serde(default = "default_heartbeat_interval_secs")]
    pub heartbeat_interval_secs: u64,
    #[serde(default)]
    pub advertise_addresses: Vec<AdvertiseAddress>,
    /// Protocols exposed by the generated manifests. Empty means "all".
    #[serde(default)]
    pub enabled_protocols: Vec<String>,
    /// Download mirror prefixes for the generated scripts (direct last).
    #[serde(default = "default_download_mirrors")]
    pub download_mirrors: Vec<String>,
    /// Default installation root of the generated bash script.
    #[serde(default = "default_node_root")]
    pub node_root: String,
    /// Detect the node address locally (no third-party IP lookup service).
    #[serde(default = "default_auto_detect_public_ip")]
    pub auto_detect_public_ip: bool,
    #[serde(default = "default_session_ttl_secs")]
    pub session_ttl_secs: u64,
    #[serde(default = "default_login_rate_limit")]
    pub login_rate_limit: u64,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            listen_addr: default_listen_addr(),
            listen_port: default_provider_port(),
            admin_user: default_admin_user(),
            admin_password: default_admin_password(),
            default_node_ip: default_node_ip(),
            provider_access_token: String::new(),
            mihomo_url: String::new(),
            mihomo_sha256: String::new(),
            mihomo_version: default_mihomo_version(),
            default_port_base: default_port_base(),
            subscribe_verify_token: String::new(),
            node_links: Vec::new(),
            registry_url: String::new(),
            registry_token: String::new(),
            registry_provider_id: String::new(),
            registry_name: String::new(),
            heartbeat_interval_secs: default_heartbeat_interval_secs(),
            advertise_addresses: Vec::new(),
            enabled_protocols: Vec::new(),
            download_mirrors: default_download_mirrors(),
            node_root: default_node_root(),
            auto_detect_public_ip: default_auto_detect_public_ip(),
            session_ttl_secs: default_session_ttl_secs(),
            login_rate_limit: default_login_rate_limit(),
        }
    }
}

impl Config {
    /// Load a configuration file (when present) and apply environment overrides.
    pub async fn load(path: &Path) -> Result<Self> {
        let mut config = if tokio::fs::try_exists(path).await.unwrap_or(false) {
            let raw = tokio::fs::read_to_string(path)
                .await
                .with_context(|| format!("cannot read provider config {}", path.display()))?;
            serde_yaml::from_str::<Config>(&raw).context("invalid provider YAML")?
        } else {
            Config::default()
        };
        config.apply_env()?;
        Ok(config)
    }

    /// Apply every supported `VPN233_*` environment override.
    pub fn apply_env(&mut self) -> Result<()> {
        if let Ok(value) = std::env::var("VPN233_ADMIN_USER") {
            self.admin_user = value;
        }
        if let Ok(value) = std::env::var("VPN233_ADMIN_PASSWORD") {
            self.admin_password = value;
        }
        if let Ok(path) = std::env::var("VPN233_ADMIN_PASSWORD_FILE") {
            self.admin_password = read_password_file(Path::new(&path))?;
        }
        if let Ok(value) = std::env::var("VPN233_PROVIDER_ACCESS_TOKEN") {
            self.provider_access_token = value;
        }
        if let Ok(value) = std::env::var("VPN233_MIHOMO_URL") {
            self.mihomo_url = value;
        }
        if let Ok(value) = std::env::var("VPN233_MIHOMO_SHA256") {
            self.mihomo_sha256 = value.trim().to_owned();
        }
        if let Ok(value) = std::env::var("VPN233_MIHOMO_VERSION") {
            self.mihomo_version = value;
        }
        if let Ok(value) = std::env::var("VPN233_PROVIDER_ADDR") {
            let (host, port) = value
                .rsplit_once(':')
                .context("VPN233_PROVIDER_ADDR must be host:port")?;
            self.listen_addr = host.to_owned();
            self.listen_port = port.parse().context("invalid VPN233_PROVIDER_ADDR port")?;
        }
        if let Ok(value) = std::env::var("VPN233_DEFAULT_NODE_IP") {
            self.default_node_ip = value;
        }
        if let Ok(value) = std::env::var("VPN233_DEFAULT_PORT_BASE") {
            self.default_port_base = value.parse().context("invalid VPN233_DEFAULT_PORT_BASE")?;
        }
        if let Ok(value) = std::env::var("VPN233_REGISTRY_URL") {
            self.registry_url = value;
        }
        if let Ok(value) = std::env::var("VPN233_REGISTRY_TOKEN") {
            self.registry_token = value;
        }
        if let Ok(value) = std::env::var("VPN233_PROVIDER_ID") {
            self.registry_provider_id = value;
        }
        if let Ok(value) = std::env::var("VPN233_PROVIDER_NAME") {
            self.registry_name = value;
        }
        if let Ok(value) = std::env::var("VPN233_HEARTBEAT_INTERVAL") {
            self.heartbeat_interval_secs =
                value.parse().context("invalid VPN233_HEARTBEAT_INTERVAL")?;
        }
        if let Ok(value) = std::env::var("VPN233_ADVERTISE_ADDRESSES") {
            self.advertise_addresses = value
                .split(',')
                .filter_map(|entry| entry.split_once('='))
                .map(|(kind, value)| AdvertiseAddress {
                    kind: kind.trim().to_owned(),
                    value: value.trim().to_owned(),
                })
                .collect();
        }
        if let Ok(value) = std::env::var("VPN233_ENABLED_PROTOCOLS") {
            self.enabled_protocols = split_list(&value);
        }
        if let Ok(value) = std::env::var("VPN233_DOWNLOAD_MIRRORS") {
            self.download_mirrors = split_list(&value);
        }
        if let Ok(value) = std::env::var("VPN233_NODE_ROOT") {
            self.node_root = value;
        }
        if let Ok(value) = std::env::var("VPN233_AUTO_DETECT_PUBLIC_IP") {
            self.auto_detect_public_ip = parse_bool(&value, "VPN233_AUTO_DETECT_PUBLIC_IP")?;
        }
        if let Ok(value) = std::env::var("VPN233_SESSION_TTL_SECS") {
            self.session_ttl_secs = value.parse().context("invalid VPN233_SESSION_TTL_SECS")?;
        }
        if let Ok(value) = std::env::var("VPN233_LOGIN_RATE_LIMIT") {
            self.login_rate_limit = value.parse().context("invalid VPN233_LOGIN_RATE_LIMIT")?;
        }
        Ok(())
    }

    /// Validate that startup-critical values are usable.
    pub fn validate(&self) -> Result<()> {
        self.socket_addr()?;
        if self.admin_user.trim().is_empty() {
            anyhow::bail!("admin_user must not be empty");
        }
        if self.session_ttl_secs == 0 {
            anyhow::bail!("session_ttl_secs must be greater than zero");
        }
        if self.login_rate_limit == 0 {
            anyhow::bail!("login_rate_limit must be greater than zero");
        }
        Ok(())
    }

    pub fn socket_addr(&self) -> Result<SocketAddr> {
        format!("{}:{}", self.listen_addr, self.listen_port)
            .parse()
            .context("invalid provider listen address")
    }

    /// Mirror prefixes in contract order; the direct connection is always last.
    pub fn mirrors(&self) -> Vec<String> {
        let mut mirrors: Vec<String> = if self.download_mirrors.is_empty() {
            default_download_mirrors()
        } else {
            self.download_mirrors.clone()
        };
        if mirrors.last().map(String::as_str) != Some("") {
            mirrors.push(String::new());
        }
        mirrors
    }

    /// Canonical, unmirrored GitHub release directory for the pinned Mihomo version.
    pub fn mihomo_release_base(&self) -> String {
        format!(
            "https://github.com/MetaCubeX/mihomo/releases/download/{}",
            self.mihomo_version.trim()
        )
    }

    pub fn is_protocol_enabled(&self, protocol: &str) -> bool {
        if self.enabled_protocols.is_empty() {
            return true;
        }
        self.enabled_protocols
            .iter()
            .any(|enabled| enabled.eq_ignore_ascii_case(protocol))
    }
}

/// Read a password from a file, trimming trailing line endings.
pub fn read_password_file(path: &Path) -> Result<String> {
    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("cannot read password file {}", path.display()))?;
    let password = raw.trim_end_matches(['\r', '\n']).to_owned();
    if password.is_empty() {
        anyhow::bail!("password file {} is empty", path.display());
    }
    Ok(password)
}

fn split_list(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
        .map(str::to_owned)
        .collect()
}

fn parse_bool(value: &str, name: &str) -> Result<bool> {
    match value.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Ok(true),
        "0" | "false" | "no" | "off" => Ok(false),
        _ => anyhow::bail!("{name} must be a boolean value"),
    }
}
