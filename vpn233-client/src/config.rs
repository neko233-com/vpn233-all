//! Client configuration, environment overrides and validation.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::net::{IpAddr, SocketAddr};
use std::path::{Path, PathBuf};

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
fn default_pac_proxy_host() -> String {
    "127.0.0.1".into()
}

/// Client configuration. Secrets are only read from an untracked `config.yaml`
/// or environment variables, never hardcoded.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Config {
    #[serde(default = "default_bind")]
    pub listen_addr: String,
    #[serde(default = "default_socks_port")]
    pub socks_port: u16,
    #[serde(default = "default_http_port")]
    pub http_port: u16,
    #[serde(default = "default_web_addr")]
    pub web_addr: String,
    #[serde(default = "default_admin_user")]
    pub admin_user: String,
    #[serde(default = "default_admin_password")]
    pub admin_password: String,
    /// `direct`, `socks5` or `auto`.
    #[serde(default = "default_upstream_mode")]
    pub upstream_mode: String,
    #[serde(default)]
    pub subscription_url: String,
    /// Optional upstream SOCKS5 proxy, `host:port` or `user:pass@host:port`.
    #[serde(default)]
    pub upstream_socks5: String,
    #[serde(default)]
    pub upstream_socks5_user: String,
    #[serde(default)]
    pub upstream_socks5_password: String,
    #[serde(default = "default_stats_file")]
    pub stats_file: PathBuf,
    /// Daily boundaries use this fixed offset; 8 means Asia/Shanghai.
    #[serde(default = "default_stats_timezone_offset_hours")]
    pub stats_timezone_offset_hours: i32,
    /// Local SOCKS5 username/password authentication (RFC 1929).
    #[serde(default)]
    pub socks_user: String,
    #[serde(default)]
    pub socks_password: String,
    /// Local HTTP proxy Basic authentication.
    #[serde(default)]
    pub http_user: String,
    #[serde(default)]
    pub http_password: String,
    /// Record destination domains in the traffic statistics. Disabled by
    /// default: VPN233 does not build browsing profiles unless explicitly asked.
    #[serde(default)]
    pub record_domains: bool,
    /// Optional PAC script returned by `/proxy.pac` verbatim.
    #[serde(default)]
    pub pac: String,
    /// Host advertised inside the generated PAC script.
    #[serde(default = "default_pac_proxy_host")]
    pub pac_proxy_host: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            listen_addr: default_bind(),
            socks_port: default_socks_port(),
            http_port: default_http_port(),
            web_addr: default_web_addr(),
            admin_user: default_admin_user(),
            admin_password: default_admin_password(),
            upstream_mode: default_upstream_mode(),
            subscription_url: String::new(),
            upstream_socks5: String::new(),
            upstream_socks5_user: String::new(),
            upstream_socks5_password: String::new(),
            stats_file: default_stats_file(),
            stats_timezone_offset_hours: default_stats_timezone_offset_hours(),
            socks_user: String::new(),
            socks_password: String::new(),
            http_user: String::new(),
            http_password: String::new(),
            record_domains: false,
            pac: String::new(),
            pac_proxy_host: default_pac_proxy_host(),
        }
    }
}

impl Config {
    /// Load a configuration file (when present) and apply environment overrides.
    pub async fn load(path: &Path) -> Result<Self> {
        let mut config = if tokio::fs::try_exists(path)
            .await
            .context("cannot access client config")?
        {
            let raw = tokio::fs::read_to_string(path)
                .await
                .with_context(|| format!("cannot read client config {}", path.display()))?;
            serde_yaml::from_str::<Config>(&raw).context("invalid client YAML")?
        } else {
            Config::default()
        };
        config.apply_env()?;
        Ok(config)
    }

    /// Apply every supported `VPN233_CLIENT_*` environment override.
    pub fn apply_env(&mut self) -> Result<()> {
        if let Ok(value) = std::env::var("VPN233_CLIENT_ADMIN_USER") {
            self.admin_user = value;
        }
        if let Ok(value) = std::env::var("VPN233_CLIENT_ADMIN_PASSWORD") {
            self.admin_password = value;
        }
        if let Ok(value) = std::env::var("VPN233_CLIENT_UPSTREAM_SOCKS5") {
            self.upstream_socks5 = value;
        }
        if let Ok(value) = std::env::var("VPN233_CLIENT_UPSTREAM_SOCKS5_USER") {
            self.upstream_socks5_user = value;
        }
        if let Ok(value) = std::env::var("VPN233_CLIENT_UPSTREAM_SOCKS5_PASSWORD") {
            self.upstream_socks5_password = value;
        }
        if let Ok(value) = std::env::var("VPN233_CLIENT_UPSTREAM_MODE") {
            self.upstream_mode = value;
        }
        if let Ok(value) = std::env::var("VPN233_CLIENT_STATS_FILE") {
            self.stats_file = PathBuf::from(value);
        }
        if let Ok(value) = std::env::var("VPN233_CLIENT_STATS_TIMEZONE_OFFSET_HOURS") {
            self.stats_timezone_offset_hours = value
                .parse()
                .context("invalid VPN233_CLIENT_STATS_TIMEZONE_OFFSET_HOURS")?;
        }
        if let Ok(value) = std::env::var("VPN233_CLIENT_LISTEN_ADDR") {
            self.listen_addr = value;
        }
        if let Ok(value) = std::env::var("VPN233_CLIENT_SOCKS_PORT") {
            self.socks_port = value.parse().context("invalid VPN233_CLIENT_SOCKS_PORT")?;
        }
        if let Ok(value) = std::env::var("VPN233_CLIENT_HTTP_PORT") {
            self.http_port = value.parse().context("invalid VPN233_CLIENT_HTTP_PORT")?;
        }
        if let Ok(value) = std::env::var("VPN233_CLIENT_WEB_ADDR") {
            self.web_addr = value;
        }
        if let Ok(value) = std::env::var("VPN233_CLIENT_SOCKS_USER") {
            self.socks_user = value;
        }
        if let Ok(value) = std::env::var("VPN233_CLIENT_SOCKS_PASSWORD") {
            self.socks_password = value;
        }
        if let Ok(value) = std::env::var("VPN233_CLIENT_HTTP_USER") {
            self.http_user = value;
        }
        if let Ok(value) = std::env::var("VPN233_CLIENT_HTTP_PASSWORD") {
            self.http_password = value;
        }
        if let Ok(value) = std::env::var("VPN233_CLIENT_RECORD_DOMAINS") {
            self.record_domains = parse_bool(&value, "VPN233_CLIENT_RECORD_DOMAINS")?;
        }
        if let Ok(value) = std::env::var("VPN233_CLIENT_PAC") {
            self.pac = value;
        }
        if let Ok(value) = std::env::var("VPN233_CLIENT_PAC_PROXY_HOST") {
            self.pac_proxy_host = value;
        }
        Ok(())
    }

    /// Validate values that would otherwise fail at runtime.
    pub fn validate(&self) -> Result<()> {
        self.socks_addr()?;
        self.http_addr()?;
        self.web_socket_addr()?;
        if self.stats_file.as_os_str().is_empty() {
            anyhow::bail!("stats_file must not be empty");
        }
        if self.socks_port != 0 && self.socks_port == self.http_port {
            anyhow::bail!("socks_port and http_port must differ");
        }
        match self.upstream_mode.trim().to_ascii_lowercase().as_str() {
            "direct" | "auto" => {}
            "socks5" => {
                if self.upstream_socks5.trim().is_empty() {
                    anyhow::bail!(
                        "upstream_mode is socks5 but upstream_socks5 is empty (use host:port or user:pass@host:port)"
                    );
                }
            }
            _ => anyhow::bail!("upstream_mode must be direct, socks5 or auto"),
        }
        if self.use_upstream() {
            crate::upstream::parse_upstream_with_credentials(
                &self.upstream_socks5,
                Some(&self.upstream_socks5_user),
                Some(&self.upstream_socks5_password),
            )?;
        }
        if !(-12..=14).contains(&self.stats_timezone_offset_hours) {
            anyhow::bail!("stats_timezone_offset_hours must be between -12 and 14");
        }
        if self.socks_auth_enabled()
            && (self.socks_user.is_empty() || self.socks_password.is_empty())
        {
            anyhow::bail!("both socks_user and socks_password are required");
        }
        if self.http_auth_enabled() && (self.http_user.is_empty() || self.http_password.is_empty())
        {
            anyhow::bail!("both http_user and http_password are required");
        }
        if !self.web_socket_addr()?.ip().is_loopback()
            && matches!(self.admin_password.as_str(), "" | "root" | "change-me")
        {
            anyhow::bail!("a non-default admin password is required for a public client API");
        }
        Ok(())
    }

    pub fn socks_addr(&self) -> Result<SocketAddr> {
        let address = self
            .listen_addr
            .trim_matches(['[', ']'])
            .parse::<IpAddr>()
            .context("invalid SOCKS5 listen address")?;
        Ok(SocketAddr::new(address, self.socks_port))
    }

    pub fn http_addr(&self) -> Result<SocketAddr> {
        let address = self
            .listen_addr
            .trim_matches(['[', ']'])
            .parse::<IpAddr>()
            .context("invalid HTTP listen address")?;
        Ok(SocketAddr::new(address, self.http_port))
    }

    pub fn web_socket_addr(&self) -> Result<SocketAddr> {
        self.web_addr.parse().context("invalid client web address")
    }

    /// True when SOCKS5 username/password authentication is required.
    pub fn socks_auth_enabled(&self) -> bool {
        !self.socks_user.is_empty() || !self.socks_password.is_empty()
    }

    /// True when local HTTP proxy Basic authentication is required.
    pub fn http_auth_enabled(&self) -> bool {
        !self.http_user.is_empty() || !self.http_password.is_empty()
    }

    /// Whether a configured upstream SOCKS5 proxy should be used.
    pub fn use_upstream(&self) -> bool {
        match self.upstream_mode.trim().to_ascii_lowercase().as_str() {
            "socks5" => true,
            "auto" => !self.upstream_socks5.trim().is_empty(),
            _ => false,
        }
    }
}

fn parse_bool(value: &str, name: &str) -> Result<bool> {
    match value.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Ok(true),
        "0" | "false" | "no" | "off" => Ok(false),
        _ => anyhow::bail!("{name} must be a boolean value"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_is_private_and_has_no_auth() {
        let config = Config::default();
        assert_eq!(config.listen_addr, "127.0.0.1");
        assert!(!config.record_domains);
        assert!(!config.socks_auth_enabled());
        assert!(!config.http_auth_enabled());
        assert!(config.validate().is_ok());
    }

    #[test]
    fn upstream_mode_is_validated() {
        let config = Config {
            upstream_mode: "exotic".into(),
            ..Config::default()
        };
        assert!(config.validate().is_err());
        let config = Config {
            upstream_mode: "socks5".into(),
            upstream_socks5: String::new(),
            ..Config::default()
        };
        assert!(config.validate().is_err());
        let config = Config {
            upstream_mode: "socks5".into(),
            upstream_socks5: "127.0.0.1:7891".into(),
            ..Config::default()
        };
        assert!(config.validate().is_ok());
        assert!(config.use_upstream());
    }

    #[test]
    fn auto_mode_uses_upstream_only_when_configured() {
        let mut config = Config {
            upstream_mode: "auto".into(),
            ..Config::default()
        };
        assert!(!config.use_upstream());
        config.upstream_socks5 = "127.0.0.1:7891".into();
        assert!(config.use_upstream());
    }
}
