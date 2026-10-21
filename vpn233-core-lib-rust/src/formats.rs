//! Subscription format registry, capabilities and rendering dispatch.
//!
//! [`SubscriptionFormat`] is the single source of truth for the formats VPN233
//! serves. Subscribe-server routes a client to a format through
//! [`SubscriptionFormat::from_alias`] and obtains the output through
//! [`render_subscription`].

use base64::engine::general_purpose::STANDARD;
use base64::Engine as _;
use serde::Serialize;

use crate::nodes;
use crate::{clash, link, singbox, text, ProxyNode};

/// Every protocol the compatibility layer can parse or render.
pub const SUPPORTED_PROTOCOLS: &[&str] = &[
    "vless",
    "vmess",
    "trojan",
    "ss",
    "ssr",
    "hysteria2",
    "tuic",
    "anytls",
    "socks5",
    "http",
    "hysteria",
    "wireguard",
    "snell",
    "ssh",
    "mieru",
];

/// Protocols with a supported URI representation. Mihomo-only structured
/// protocols are intentionally absent rather than emitted as invented links.
const LINK_PROTOCOLS: &[&str] = &[
    "vless",
    "vmess",
    "trojan",
    "ss",
    "ssr",
    "hysteria2",
    "tuic",
    "anytls",
    "socks5",
    "http",
];

const SINGBOX_PROTOCOLS: &[&str] = &[
    "vless",
    "vmess",
    "trojan",
    "ss",
    "hysteria2",
    "tuic",
    "anytls",
    "socks5",
    "http",
];
const SURGE_PROTOCOLS: &[&str] = &["vmess", "trojan", "ss", "socks5", "http"];
const QUANTUMULTX_PROTOCOLS: &[&str] = &["vmess", "trojan", "ss", "vless", "socks5", "http"];
const LOON_PROTOCOLS: &[&str] = &[
    "vmess",
    "trojan",
    "ss",
    "vless",
    "socks5",
    "http",
    "hysteria2",
];
const SURFBOARD_PROTOCOLS: &[&str] = &["vmess", "trojan", "ss", "ssr", "socks5", "http"];

/// Subscription formats VPN233 can render.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SubscriptionFormat {
    /// Clash / Mihomo YAML.
    Clash,
    /// sing-box JSON.
    SingBox,
    /// Base64 encoded share links (v2rayN, v2rayNG, Shadowrocket, ...).
    V2Ray,
    /// Plain share links, one per line.
    Links,
    /// Surge configuration.
    Surge,
    /// Quantumult X configuration.
    QuantumultX,
    /// Loon configuration.
    Loon,
    /// Surfboard configuration.
    Surfboard,
    /// Stash YAML (Clash compatible dialect).
    Stash,
}

impl SubscriptionFormat {
    /// All formats in display order (Clash first).
    pub fn all() -> &'static [SubscriptionFormat] {
        &[
            Self::Clash,
            Self::SingBox,
            Self::V2Ray,
            Self::Links,
            Self::Surge,
            Self::QuantumultX,
            Self::Loon,
            Self::Surfboard,
            Self::Stash,
        ]
    }

    /// Stable format id used in URLs and API responses.
    pub fn canonical_id(&self) -> &'static str {
        match self {
            Self::Clash => "clash",
            Self::SingBox => "singbox",
            Self::V2Ray => "v2ray",
            Self::Links => "links",
            Self::Surge => "surge",
            Self::QuantumultX => "quantumultx",
            Self::Loon => "loon",
            Self::Surfboard => "surfboard",
            Self::Stash => "stash",
        }
    }

    /// Human readable client name.
    pub fn display_name(&self) -> &'static str {
        match self {
            Self::Clash => "Clash / Mihomo",
            Self::SingBox => "sing-box",
            Self::V2Ray => "v2rayN / v2rayNG",
            Self::Links => "Raw Links",
            Self::Surge => "Surge",
            Self::QuantumultX => "Quantumult X",
            Self::Loon => "Loon",
            Self::Surfboard => "Surfboard",
            Self::Stash => "Stash",
        }
    }

    /// File extension used when the output is exported as a file.
    pub fn ext(&self) -> &'static str {
        match self {
            Self::Clash | Self::Stash => "yaml",
            Self::SingBox => "json",
            Self::V2Ray | Self::Links => "txt",
            Self::Surge | Self::QuantumultX | Self::Loon | Self::Surfboard => "conf",
        }
    }

    /// HTTP `Content-Type` header for the rendered output.
    pub fn content_type(&self) -> &'static str {
        match self {
            Self::Clash | Self::Stash => "text/yaml; charset=utf-8",
            Self::SingBox => "application/json; charset=utf-8",
            _ => "text/plain; charset=utf-8",
        }
    }

    /// Protocols this format can represent. Always a subset of
    /// [`SUPPORTED_PROTOCOLS`].
    pub fn protocols(&self) -> &'static [&'static str] {
        match self {
            Self::Clash => SUPPORTED_PROTOCOLS,
            Self::V2Ray | Self::Links => LINK_PROTOCOLS,
            Self::SingBox | Self::Stash => SINGBOX_PROTOCOLS,
            Self::Surge => SURGE_PROTOCOLS,
            Self::QuantumultX => QUANTUMULTX_PROTOCOLS,
            Self::Loon => LOON_PROTOCOLS,
            Self::Surfboard => SURFBOARD_PROTOCOLS,
        }
    }

    /// Returns true when the format can represent the given protocol.
    ///
    /// The protocol name is matched case-insensitively after trimming.
    pub fn supports_protocol(&self, protocol: &str) -> bool {
        let protocol = protocol.trim().to_ascii_lowercase();
        if protocol.is_empty() {
            return false;
        }
        self.protocols().contains(&protocol.as_str())
    }

    /// Checks protocol and transport before converting a node.
    pub fn supports_node(&self, node: &ProxyNode) -> bool {
        let Some(kind) = nodes::node_type_lower(node) else {
            return false;
        };
        if !self.supports_protocol(&kind) {
            return false;
        }
        if matches!(self, Self::Clash | Self::Stash) {
            return true;
        }
        let network = nodes::node_network(node);
        if matches!(self, Self::Links | Self::V2Ray | Self::SingBox) {
            return matches!(
                network.as_str(),
                "" | "tcp" | "ws" | "grpc" | "http" | "h2" | "httpupgrade"
            );
        }
        if node.contains_key("reality-opts")
            || nodes::node_str(node, "flow").is_some_and(|flow| !flow.is_empty())
        {
            return false;
        }
        match kind.as_str() {
            "vmess" | "vless" => matches!(network.as_str(), "" | "tcp" | "ws"),
            "trojan" if *self == Self::Loon => matches!(network.as_str(), "" | "tcp" | "ws"),
            "trojan" => matches!(network.as_str(), "" | "tcp"),
            "ss" => nodes::node_str(node, "plugin").is_none_or(|plugin| plugin == "obfs"),
            _ => true,
        }
    }

    /// Resolves a client hint or canonical id to a format.
    ///
    /// Matching is case-insensitive and trims surrounding whitespace. The
    /// alias set intentionally follows the routing rules used by
    /// subscribe-server, for example `hiddify`, `nekobox` and `nekoray` all map
    /// to [`SubscriptionFormat::V2Ray`] because those clients consume base64
    /// encoded share links.
    pub fn from_alias(value: &str) -> Option<Self> {
        let key = value.trim().to_ascii_lowercase();
        match key.as_str() {
            "clash" | "mihomo" | "clash-meta" | "clashmeta" | "clash_meta" | "meta"
            | "openclash" | "clash-verge" | "verge" | "clashx" | "clashx-pro" => Some(Self::Clash),
            "singbox" | "sing-box" | "box" | "sfa" | "sfi" | "sfm" => Some(Self::SingBox),
            "v2ray" | "v2rayn" | "v2rayng" | "base64" | "shadowrocket" | "hiddify" | "nekobox"
            | "nekoray" => Some(Self::V2Ray),
            "links" | "link" | "uri" | "uris" | "plain" | "raw" => Some(Self::Links),
            "surge" | "surge4" | "surge5" => Some(Self::Surge),
            "quantumultx" | "quantumult" | "quantumult-x" | "quantumult_x" | "qx" => {
                Some(Self::QuantumultX)
            }
            "loon" => Some(Self::Loon),
            "surfboard" | "surfboard2" => Some(Self::Surfboard),
            "stash" => Some(Self::Stash),
            _ => None,
        }
    }
}

/// Machine readable description of one subscription format capability.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FormatCapability {
    /// Stable format id, see [`SubscriptionFormat::canonical_id`].
    pub id: String,
    /// Human readable client name.
    pub name: String,
    /// HTTP content type of the rendered payload.
    pub content_type: String,
    /// File extension of the rendered payload.
    pub extension: String,
    /// Protocols the format can represent.
    pub protocols: Vec<String>,
    /// Whether the format is recommended for new clients.
    pub recommended: bool,
}

/// Error returned when a subscription payload cannot be rendered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderError(pub String);

impl std::fmt::Display for RenderError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "render error: {}", self.0)
    }
}

impl std::error::Error for RenderError {}

impl From<serde_yaml::Error> for RenderError {
    fn from(error: serde_yaml::Error) -> Self {
        Self(error.to_string())
    }
}

/// Returns the capability matrix for every subscription format.
pub fn format_capabilities() -> Vec<FormatCapability> {
    SubscriptionFormat::all()
        .iter()
        .map(|format| FormatCapability {
            id: format.canonical_id().to_string(),
            name: format.display_name().to_string(),
            content_type: format.content_type().to_string(),
            extension: format.ext().to_string(),
            protocols: format
                .protocols()
                .iter()
                .map(|protocol| protocol.to_string())
                .collect(),
            recommended: is_recommended(*format),
        })
        .collect()
}

fn is_recommended(format: SubscriptionFormat) -> bool {
    matches!(
        format,
        SubscriptionFormat::Clash | SubscriptionFormat::SingBox | SubscriptionFormat::V2Ray
    )
}

/// Renders a subscription in the requested format.
///
/// Nodes that use a protocol the format cannot represent are skipped silently;
/// an empty node list still produces a valid, importable configuration.
pub fn render_subscription(
    format: SubscriptionFormat,
    nodes: &[ProxyNode],
    profile_name: &str,
) -> Result<String, RenderError> {
    let selected: Vec<ProxyNode> = nodes
        .iter()
        .filter(|node| format.supports_node(node))
        .cloned()
        .collect();
    let profile = nodes::sanitize_profile_name(profile_name);
    match format {
        SubscriptionFormat::Clash => {
            clash::render_clash_profile(&selected, &profile).map_err(RenderError::from)
        }
        SubscriptionFormat::Stash => {
            clash::render_stash_profile(&selected, &profile).map_err(RenderError::from)
        }
        SubscriptionFormat::SingBox => Ok(singbox::render_singbox_config(&selected)),
        SubscriptionFormat::V2Ray => Ok(render_v2ray_payload(&selected)),
        SubscriptionFormat::Links => Ok(render_link_lines(&selected)),
        SubscriptionFormat::Surge => Ok(text::render_surge_profile(&selected, &profile)),
        SubscriptionFormat::QuantumultX => {
            Ok(text::render_quantumultx_profile(&selected, &profile))
        }
        SubscriptionFormat::Loon => Ok(text::render_loon_profile(&selected, &profile)),
        SubscriptionFormat::Surfboard => Ok(text::render_surfboard_profile(&selected, &profile)),
    }
}

fn node_share_link(node: &ProxyNode) -> Option<String> {
    if let Some(raw) = nodes::node_str(node, link::LINK_KEY)
        .map(str::trim)
        .filter(|raw| !raw.is_empty())
    {
        return Some(raw.to_string());
    }
    link::render_link(node).ok()
}

fn node_share_links(nodes: &[ProxyNode]) -> Vec<String> {
    nodes.iter().filter_map(node_share_link).collect()
}

fn render_link_lines(nodes: &[ProxyNode]) -> String {
    let mut body = node_share_links(nodes).join("\n");
    if !body.is_empty() {
        body.push('\n');
    }
    body
}

fn render_v2ray_payload(nodes: &[ProxyNode]) -> String {
    let links = node_share_links(nodes);
    if links.is_empty() {
        return String::new();
    }
    let payload = STANDARD.encode(links.join("\n").as_bytes());
    format!("{payload}\n")
}

#[cfg(test)]
mod tests;
