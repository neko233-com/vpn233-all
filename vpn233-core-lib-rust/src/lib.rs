//! Rust core compatibility layer for VPN233.
//!
//! This crate is the shared boundary between VPN233's services and the proxy
//! clients they serve. It provides:
//!
//! - a Clash/Mihomo compatible [`ProxyNode`] model and configuration renderer;
//! - share-link parsing and rendering for every supported protocol;
//! - subscription rendering for Clash/Mihomo, sing-box, v2rayN/v2rayNG,
//!   raw links, Surge, Quantumult X, Loon, Surfboard and Stash;
//! - extension points for custom protocol adapters.
//!
//! TUN handling, kernel routing and the actual proxy transports remain the
//! responsibility of the upstream runtime; this crate only produces and
//! consumes configuration.

use serde::Serialize;
use serde_yaml::Value;
use std::collections::BTreeMap;

pub mod clash;
pub mod formats;
pub mod import;
pub mod link;
pub mod nodes;
pub mod singbox;
pub mod text;

pub use clash::render_clash_config;
pub use formats::{
    format_capabilities, render_subscription, FormatCapability, RenderError, SubscriptionFormat,
};
pub use import::parse_subscription;
pub use link::{parse_link, parse_links, render_link, LinkError};
pub use singbox::render_singbox_config;
pub use text::{
    render_loon_config, render_quantumultx_config, render_surfboard_config, render_surge_config,
};

/// A single proxy definition using the Clash/Mihomo field names.
pub type ProxyNode = BTreeMap<String, Value>;

/// Full client configuration model used by `vpn233-client`.
#[derive(Debug, Clone, Serialize)]
pub struct CoreConfig {
    pub mixed_port: u16,
    pub allow_lan: bool,
    pub mode: String,
    pub proxies: Vec<ProxyNode>,
    pub rules: Vec<RouteRule>,
}

/// One routing rule.
#[derive(Debug, Clone, Serialize)]
pub struct RouteRule {
    pub matcher: String,
    pub target: String,
}

/// A typed outbound definition that can be converted into a [`ProxyNode`].
#[derive(Debug, Clone, Serialize)]
pub struct Outbound {
    pub tag: String,
    pub kind: String,
    pub server: String,
    pub port: u16,
    pub options: ProxyNode,
}

impl Outbound {
    /// Converts this outbound into a proxy node.
    pub fn into_proxy_node(self) -> ProxyNode {
        let mut node = proxy_node(&self.tag, &self.kind, &self.server, self.port);
        node.extend(self.options);
        node
    }
}

/// Extension point for protocol adapters that are not part of the built-in set.
pub trait CustomProtocol: Send + Sync {
    /// Unique protocol name.
    fn protocol_name(&self) -> &'static str;
    /// Converts a generic node into a protocol specific node.
    fn render_proxy(&self, input: &ProxyNode) -> Result<ProxyNode, String>;
}

/// Registry for [`CustomProtocol`] implementations.
#[derive(Default)]
pub struct ProtocolRegistry {
    protocols: BTreeMap<String, Box<dyn CustomProtocol>>,
}

impl ProtocolRegistry {
    /// Registers (or replaces) a custom protocol.
    pub fn register(&mut self, protocol: impl CustomProtocol + 'static) {
        self.protocols
            .insert(protocol.protocol_name().into(), Box::new(protocol));
    }

    /// Renders a node through a registered custom protocol.
    pub fn render(&self, name: &str, input: &ProxyNode) -> Result<ProxyNode, String> {
        self.protocols
            .get(name)
            .ok_or_else(|| format!("unknown protocol: {name}"))?
            .render_proxy(input)
    }
}

/// Every protocol the compatibility layer supports.
pub fn supported_protocols() -> &'static [&'static str] {
    formats::SUPPORTED_PROTOCOLS
}

/// Builds a YAML string value.
pub fn string(value: impl Into<String>) -> Value {
    Value::String(value.into())
}

/// Builds a YAML numeric value.
pub fn number(value: u64) -> Value {
    Value::Number(value.into())
}

/// Builds a YAML boolean value.
pub fn boolean(value: bool) -> Value {
    Value::Bool(value)
}

/// Builds a minimal proxy node with the required Clash/Mihomo fields.
pub fn proxy_node(name: &str, kind: &str, server: &str, port: u16) -> ProxyNode {
    let mut node = ProxyNode::new();
    node.insert("name".into(), string(name));
    node.insert("type".into(), string(kind));
    node.insert("server".into(), string(server));
    node.insert("port".into(), number(port.into()));
    node
}

/// Renders the raw share links of the given nodes, one per line.
///
/// Nodes without an internal `vpn233-link` attribute are skipped; use
/// [`render_link`] to synthesize a link from a node.
pub fn render_links(nodes: &[ProxyNode]) -> String {
    nodes
        .iter()
        .filter_map(|node| node.get(link::LINK_KEY).and_then(Value::as_str))
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    struct DemoProtocol;

    impl CustomProtocol for DemoProtocol {
        fn protocol_name(&self) -> &'static str {
            "demo"
        }
        fn render_proxy(&self, input: &ProxyNode) -> Result<ProxyNode, String> {
            let mut output = input.clone();
            output.insert("type".into(), string("demo"));
            Ok(output)
        }
    }

    #[test]
    fn custom_protocols_are_replaceable() {
        let mut registry = ProtocolRegistry::default();
        registry.register(DemoProtocol);
        let input = proxy_node("node", "direct", "127.0.0.1", 443);
        assert_eq!(
            registry.render("demo", &input).unwrap().get("type"),
            Some(&string("demo"))
        );
    }

    #[test]
    fn clash_output_contains_proxy_and_route_sections() {
        let nodes = vec![proxy_node("node", "vless", "127.0.0.1", 443)];
        let body = render_clash_config(&nodes).unwrap();
        assert!(body.contains("proxies:"));
        assert!(body.contains("proxy-groups:"));
        assert!(body.contains("rules:"));
        assert!(!body.contains("name: DIRECT\n"));
        assert!(!body.contains("GEOSITE,"));
        assert!(!body.contains("GEOIP,"));
    }

    #[test]
    fn clash_output_has_a_valid_direct_fallback_without_nodes() {
        let body = render_clash_config(&[]).unwrap();
        assert!(body.contains("name: VPN233"));
        assert!(body.contains("- DIRECT"));
    }

    #[test]
    fn supported_protocols_are_exposed() {
        assert_eq!(
            supported_protocols(),
            &[
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
            ]
        );
    }

    #[test]
    fn legacy_render_links_uses_the_internal_link_attribute() {
        let mut node = proxy_node("node", "ss", "example.invalid", 8388);
        node.insert("vpn233-link".into(), string("ss://example"));
        assert_eq!(render_links(&[node]), "ss://example");
    }

    #[test]
    fn outbound_conversion_keeps_custom_options() {
        let mut options = ProxyNode::new();
        options.insert("password".into(), string("secret"));
        let outbound = Outbound {
            tag: "node".into(),
            kind: "trojan".into(),
            server: "example.invalid".into(),
            port: 443,
            options,
        };
        let node = outbound.into_proxy_node();
        assert_eq!(node.get("type"), Some(&string("trojan")));
        assert_eq!(node.get("password"), Some(&string("secret")));
    }
}
