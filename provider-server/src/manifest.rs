//! Protocol catalogue, generated node manifests and subscription link builders.

use crate::config::Config;
use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use serde::{Deserialize, Serialize};
use serde_json::json;
use uuid::Uuid;

/// One protocol the provider can expose on the node.
#[derive(Debug, Clone, Copy)]
pub struct ProtocolDef {
    pub id: &'static str,
    pub name: &'static str,
    /// Offset added to the configured base port.
    pub offset: u16,
    /// Link name suffix used when printing node links.
    pub suffix: &'static str,
    /// Whether the generated Mihomo config enables TLS for this inbound.
    pub tls: bool,
}

/// All protocols exposed by the generated manifest, in port order.
pub const PROTOCOLS: [ProtocolDef; 8] = [
    ProtocolDef {
        id: "vless",
        name: "VLESS",
        offset: 0,
        suffix: "VLESS",
        tls: true,
    },
    ProtocolDef {
        id: "vmess",
        name: "VMess",
        offset: 1,
        suffix: "VMess",
        tls: false,
    },
    ProtocolDef {
        id: "trojan",
        name: "Trojan",
        offset: 2,
        suffix: "Trojan",
        tls: true,
    },
    ProtocolDef {
        id: "ss",
        name: "Shadowsocks",
        offset: 3,
        suffix: "SS",
        tls: false,
    },
    ProtocolDef {
        id: "hysteria2",
        name: "Hysteria2",
        offset: 4,
        suffix: "Hysteria2",
        tls: true,
    },
    ProtocolDef {
        id: "tuic",
        name: "TUIC",
        offset: 5,
        suffix: "TUIC",
        tls: true,
    },
    ProtocolDef {
        id: "anytls",
        name: "AnyTLS",
        offset: 6,
        suffix: "AnyTLS",
        tls: true,
    },
    ProtocolDef {
        id: "socks5",
        name: "SOCKS5",
        offset: 7,
        suffix: "SOCKS5",
        tls: false,
    },
];

/// Supported protocol identifiers, in manifest order.
pub fn supported_protocols() -> Vec<&'static str> {
    PROTOCOLS.iter().map(|protocol| protocol.id).collect()
}

/// Format capability entry (same JSON shape as the subscribe-server API).
#[derive(Debug, Clone, Serialize)]
pub struct FormatCapability {
    pub id: String,
    pub name: String,
    pub content_type: String,
    pub extension: String,
    pub protocols: Vec<String>,
    pub recommended: bool,
}

fn capability(
    id: &str,
    name: &str,
    content_type: &str,
    extension: &str,
    protocols: &[&str],
    recommended: bool,
) -> FormatCapability {
    FormatCapability {
        id: id.to_owned(),
        name: name.to_owned(),
        content_type: content_type.to_owned(),
        extension: extension.to_owned(),
        protocols: protocols.iter().map(|value| (*value).to_owned()).collect(),
        recommended,
    }
}

/// Capability matrix used by `/api/v1/protocols` and `/api/v1/formats`.
///
/// The identifiers and JSON shape mirror the core library contract so the
/// admin web can consume either service with the same code.
pub fn format_capabilities() -> Vec<FormatCapability> {
    vec![
        capability(
            "clash",
            "Clash / Mihomo",
            "text/yaml; charset=utf-8",
            "yaml",
            &[
                "vless",
                "vmess",
                "trojan",
                "ss",
                "hysteria2",
                "tuic",
                "anytls",
                "socks5",
            ],
            true,
        ),
        capability(
            "singbox",
            "sing-box",
            "application/json; charset=utf-8",
            "json",
            &[
                "vless",
                "vmess",
                "trojan",
                "ss",
                "hysteria2",
                "tuic",
                "anytls",
                "socks5",
            ],
            true,
        ),
        capability(
            "v2ray",
            "v2rayN / v2rayNG",
            "text/plain; charset=utf-8",
            "txt",
            &[
                "vless",
                "vmess",
                "trojan",
                "ss",
                "hysteria2",
                "tuic",
                "socks5",
            ],
            false,
        ),
        capability(
            "links",
            "Raw Links",
            "text/plain; charset=utf-8",
            "txt",
            &[
                "vless",
                "vmess",
                "trojan",
                "ss",
                "hysteria2",
                "tuic",
                "anytls",
                "socks5",
            ],
            false,
        ),
        capability(
            "surge",
            "Surge",
            "text/plain; charset=utf-8",
            "conf",
            &["ss", "vmess", "trojan", "socks5"],
            false,
        ),
        capability(
            "quantumultx",
            "Quantumult X",
            "text/plain; charset=utf-8",
            "conf",
            &["ss", "vmess", "trojan", "vless", "socks5"],
            false,
        ),
        capability(
            "loon",
            "Loon",
            "text/plain; charset=utf-8",
            "conf",
            &["ss", "vmess", "trojan", "vless", "hysteria2", "socks5"],
            false,
        ),
        capability(
            "surfboard",
            "Surfboard",
            "text/plain; charset=utf-8",
            "conf",
            &["ss", "vmess", "trojan", "socks5"],
            false,
        ),
        capability(
            "stash",
            "Stash",
            "text/yaml; charset=utf-8",
            "yaml",
            &[
                "vless",
                "vmess",
                "trojan",
                "ss",
                "hysteria2",
                "tuic",
                "anytls",
                "socks5",
            ],
            false,
        ),
    ]
}

/// Query parameters accepted by the generate endpoints.
#[derive(Debug, Default, Clone, Deserialize)]
pub struct GenerateQuery {
    pub format: Option<String>,
    pub node_name: Option<String>,
    pub node_ip: Option<String>,
    pub port_base: Option<u16>,
    pub uuid: Option<String>,
    pub password: Option<String>,
}

/// One port/fork of the node.
#[derive(Debug, Clone, Serialize)]
pub struct NodePort {
    pub id: String,
    pub name: String,
    pub protocol: String,
    pub core: String,
    pub port: u16,
}

/// Deterministic node description used by the generated scripts and links.
#[derive(Debug, Clone, Serialize)]
pub struct NodeManifest {
    pub node_name: String,
    pub node_ip: String,
    pub uuid: String,
    pub password: String,
    pub ports: Vec<NodePort>,
}

impl NodeManifest {
    /// Build a manifest from the configuration and optional query overrides.
    pub fn build(config: &Config, query: &GenerateQuery) -> Self {
        let base = query.port_base.unwrap_or(config.default_port_base);
        let ports = PROTOCOLS
            .iter()
            .filter(|protocol| config.is_protocol_enabled(protocol.id))
            .map(|protocol| NodePort {
                id: protocol.id.to_owned(),
                name: protocol.name.to_owned(),
                protocol: protocol.id.to_owned(),
                core: "mihomo".to_owned(),
                port: base.saturating_add(protocol.offset),
            })
            .collect::<Vec<_>>();
        Self {
            node_name: query
                .node_name
                .as_deref()
                .map(sanitize_node_name)
                .filter(|name| !name.is_empty())
                .unwrap_or_else(|| "vpn233-node".to_owned()),
            node_ip: resolve_node_ip(config, query.node_ip.as_deref()),
            uuid: query
                .uuid
                .as_deref()
                .and_then(sanitize_secret)
                .unwrap_or_else(|| Uuid::new_v4().to_string()),
            password: query
                .password
                .as_deref()
                .and_then(sanitize_secret)
                .unwrap_or_else(|| Uuid::new_v4().to_string()),
            ports,
        }
    }

    /// Manifest built from defaults only (used by the registry reporter).
    pub fn from_config(config: &Config) -> Self {
        Self::build(config, &GenerateQuery::default())
    }

    pub fn port_for(&self, protocol: &str) -> Option<u16> {
        self.ports
            .iter()
            .find(|port| port.protocol == protocol)
            .map(|port| port.port)
    }

    /// Subscription links for every enabled protocol.
    pub fn link_lines(&self) -> Vec<String> {
        let mut links = Vec::new();
        for port in &self.ports {
            links.push(self.link_for(port));
        }
        links
    }

    pub fn links_text(&self) -> String {
        let mut text = self.link_lines().join("\n");
        text.push('\n');
        text
    }

    fn link_for(&self, port: &NodePort) -> String {
        let name = &self.node_name;
        let ip = &self.node_ip;
        let uuid = &self.uuid;
        let password = &self.password;
        let suffix = suffix_for(&port.protocol);
        match port.protocol.as_str() {
            "vless" => format!(
                "vless://{uuid}@{ip}:{port}?encryption=none&security=tls&type=tcp&sni={ip}#{name}-{suffix}",
                port = port.port
            ),
            "vmess" => {
                let payload = json!({
                    "v": "2",
                    "ps": format!("{name}-{suffix}"),
                    "add": ip,
                    "port": port.port.to_string(),
                    "id": uuid,
                    "aid": "0",
                    "scy": "auto",
                    "net": "tcp",
                    "type": "none",
                    "host": "",
                    "path": "",
                    "tls": "",
                });
                format!("vmess://{}", BASE64.encode(payload.to_string()))
            }
            "trojan" => format!(
                "trojan://{password}@{ip}:{port}?security=tls&type=tcp&sni={ip}#{name}-{suffix}",
                port = port.port
            ),
            "ss" => {
                let userinfo = BASE64.encode(format!("aes-256-gcm:{password}"));
                format!(
                    "ss://{userinfo}@{ip}:{port}#{name}-{suffix}",
                    port = port.port
                )
            }
            "hysteria2" => format!(
                "hysteria2://{password}@{ip}:{port}/?sni={ip}&insecure=1#{name}-{suffix}",
                port = port.port
            ),
            "tuic" => format!(
                "tuic://{uuid}:{password}@{ip}:{port}?congestion_control=bbr&alpn=h3&sni={ip}&insecure=1#{name}-{suffix}",
                port = port.port
            ),
            "anytls" => format!(
                "anytls://{password}@{ip}:{port}/?sni={ip}&insecure=1#{name}-{suffix}",
                port = port.port
            ),
            "socks5" => format!(
                "socks5://{uuid}:{password}@{ip}:{port}#{name}-{suffix}",
                port = port.port
            ),
            _ => String::new(),
        }
    }
}

fn suffix_for(protocol: &str) -> &'static str {
    PROTOCOLS
        .iter()
        .find(|candidate| candidate.id == protocol)
        .map(|candidate| candidate.suffix)
        .unwrap_or("Node")
}

/// Keep generated names shell- and URL-safe.
pub fn sanitize_node_name(raw: &str) -> String {
    let sanitized = raw
        .trim()
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | '-') {
                character
            } else {
                '-'
            }
        })
        .collect::<String>();
    let trimmed = sanitized.trim_matches('-').to_owned();
    trimmed.chars().take(64).collect()
}

/// Keep caller-supplied uuid/password values shell-safe.
pub fn sanitize_secret(raw: &str) -> Option<String> {
    let sanitized = raw
        .trim()
        .chars()
        .filter(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '=' | '+' | '/')
        })
        .collect::<String>();
    if sanitized.is_empty() {
        None
    } else {
        Some(sanitized.chars().take(128).collect())
    }
}

/// Resolve the advertised node address without contacting third parties.
pub fn resolve_node_ip(config: &Config, requested: Option<&str>) -> String {
    if let Some(requested) = requested.map(str::trim).filter(|value| !value.is_empty()) {
        if !requested.eq_ignore_ascii_case("auto") {
            return sanitize_ip_like(requested);
        }
    }
    let configured = config.default_node_ip.trim();
    if !configured.is_empty() && !configured.eq_ignore_ascii_case("auto") {
        return sanitize_ip_like(configured);
    }
    if config.auto_detect_public_ip {
        detect_local_ip().unwrap_or_else(|| "127.0.0.1".to_owned())
    } else {
        "127.0.0.1".to_owned()
    }
}

fn sanitize_ip_like(value: &str) -> String {
    let cleaned = value
        .chars()
        .filter(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '.' | '-' | ':' | '[' | ']')
        })
        .collect::<String>();
    if cleaned.is_empty() {
        "127.0.0.1".to_owned()
    } else {
        cleaned
    }
}

/// Local-only address detection: `UdpSocket::connect` performs no network I/O,
/// it only selects the outbound interface and returns its local address.
pub fn detect_local_ip() -> Option<String> {
    let socket = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.connect("1.1.1.1:80").ok()?;
    let address = socket.local_addr().ok()?.ip();
    if address.is_unspecified() {
        None
    } else {
        Some(address.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config() -> Config {
        Config {
            default_node_ip: "203.0.113.10".into(),
            ..Config::default()
        }
    }

    #[test]
    fn manifest_assigns_one_port_per_protocol() {
        let manifest = NodeManifest::build(
            &Config {
                default_port_base: 20000,
                ..test_config()
            },
            &GenerateQuery::default(),
        );
        assert_eq!(manifest.ports.len(), 8);
        assert_eq!(manifest.port_for("vless"), Some(20000));
        assert_eq!(manifest.port_for("vmess"), Some(20001));
        assert_eq!(manifest.port_for("trojan"), Some(20002));
        assert_eq!(manifest.port_for("ss"), Some(20003));
        assert_eq!(manifest.port_for("hysteria2"), Some(20004));
        assert_eq!(manifest.port_for("tuic"), Some(20005));
        assert_eq!(manifest.port_for("anytls"), Some(20006));
        assert_eq!(manifest.port_for("socks5"), Some(20007));
    }

    #[test]
    fn manifest_filters_disabled_protocols() {
        let manifest = NodeManifest::build(
            &Config {
                enabled_protocols: vec!["vless".into(), "socks5".into()],
                ..test_config()
            },
            &GenerateQuery::default(),
        );
        let protocols = manifest
            .ports
            .iter()
            .map(|port| port.protocol.as_str())
            .collect::<Vec<_>>();
        assert_eq!(protocols, vec!["vless", "socks5"]);
    }

    #[test]
    fn links_cover_every_protocol() {
        let links = NodeManifest::build(&test_config(), &GenerateQuery::default()).link_lines();
        assert_eq!(links.len(), 8);
        for prefix in [
            "vless://",
            "vmess://",
            "trojan://",
            "ss://",
            "hysteria2://",
            "tuic://",
            "anytls://",
            "socks5://",
        ] {
            assert!(
                links.iter().any(|link| link.starts_with(prefix)),
                "missing {prefix}"
            );
        }
    }

    #[test]
    fn node_name_and_secrets_are_sanitized() {
        let manifest = NodeManifest::build(
            &test_config(),
            &GenerateQuery {
                node_name: Some("bad name'; rm -rf /".into()),
                uuid: Some("uuid with spaces".into()),
                password: Some("secret'quote".into()),
                ..GenerateQuery::default()
            },
        );
        assert!(!manifest.node_name.contains(' '));
        assert!(!manifest.node_name.contains('\''));
        assert!(!manifest.uuid.contains(' '));
        assert!(!manifest.password.contains('\''));
    }
}
