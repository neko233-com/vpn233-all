//! Shared helpers for proxy nodes and subscription renderers.
//!
//! A [`ProxyNode`](crate::ProxyNode) is a `BTreeMap` of YAML values that uses
//! the Clash/Mihomo field names. The helpers in this module keep every renderer
//! consistent about internal key stripping, name escaping, de-duplication and
//! typed field access.

use std::collections::BTreeSet;

use serde_yaml::{Mapping, Value};

use crate::{string, ProxyNode};

/// Prefix reserved for VPN233-internal node attributes such as `vpn233-link`.
pub const INTERNAL_KEY_PREFIX: &str = "vpn233-";

/// Default profile label used when a caller passes an empty name.
pub const DEFAULT_PROFILE: &str = "VPN233";
/// Name of the primary selector proxy group / outbound.
pub const MAIN_GROUP: &str = "VPN233";
/// Historical automatic group name, rendered as a manual selector by default.
pub const AUTO_GROUP: &str = "VPN233 Auto";
/// Name of the fallback proxy group.
pub const FALLBACK_GROUP: &str = "VPN233 Fallback";
/// Name of the nested manual selection group.
pub const PROXIES_GROUP: &str = "Proxies";
/// Built-in direct policy name.
pub const DIRECT_POLICY: &str = "DIRECT";
/// Built-in reject policy name.
pub const REJECT_POLICY: &str = "REJECT";

/// Local mixed inbound port used by generated client configurations.
pub const MIXED_PORT: u16 = 7890;

/// Private IPv4 ranges that every generated rule set routes directly.
pub const PRIVATE_CIDRS: &[&str] = &[
    "127.0.0.0/8",
    "10.0.0.0/8",
    "172.16.0.0/12",
    "192.168.0.0/16",
];

/// Local IPv6 destinations that must never be sent to an upstream proxy.
pub const PRIVATE_IPV6_CIDRS: &[&str] = &["::1/128", "fc00::/7", "fe80::/10"];

/// Names that a Clash/Mihomo node may not reuse because groups own them.
pub const CLASH_RESERVED: &[&str] = &[
    MAIN_GROUP,
    AUTO_GROUP,
    FALLBACK_GROUP,
    PROXIES_GROUP,
    DIRECT_POLICY,
    REJECT_POLICY,
];

/// Names reserved in line oriented client formats.
pub const LINE_RESERVED: &[&str] = &[MAIN_GROUP, PROXIES_GROUP, DIRECT_POLICY, REJECT_POLICY];

/// Characters that break line oriented client formats when used inside names.
const NAME_REPLACEMENTS: &[char] = &[',', '=', '"', '\n', '\r', '\t', '#', ';'];

/// Returns true when the key is a VPN233-internal attribute that must never
/// leak into a user-visible configuration.
pub fn is_internal_key(key: &str) -> bool {
    key.starts_with(INTERNAL_KEY_PREFIX)
}

/// Reads a string attribute from a node.
pub fn node_str<'a>(node: &'a ProxyNode, key: &str) -> Option<&'a str> {
    node.get(key).and_then(Value::as_str)
}

/// Reads the normalized (lowercase, trimmed) protocol type of a node.
pub fn node_type_lower(node: &ProxyNode) -> Option<String> {
    node_str(node, "type")
        .map(str::trim)
        .filter(|kind| !kind.is_empty())
        .map(str::to_ascii_lowercase)
}

/// Returns the effective V2Ray transport, including Mihomo's HTTPUpgrade flag.
pub fn node_network(node: &ProxyNode) -> String {
    let network = node_str(node, "network")
        .unwrap_or("tcp")
        .trim()
        .to_ascii_lowercase();
    if network == "ws"
        && node_mapping(node, "ws-opts")
            .and_then(|options| mapping_get(options, "v2ray-http-upgrade"))
            .and_then(Value::as_bool)
            .unwrap_or(false)
    {
        "httpupgrade".to_string()
    } else {
        network
    }
}

/// Reads the trimmed `server` attribute of a node.
pub fn node_server(node: &ProxyNode) -> Option<&str> {
    node_str(node, "server")
        .map(str::trim)
        .filter(|server| !server.is_empty())
}

/// Reads the trimmed `name` attribute of a node.
pub fn node_name(node: &ProxyNode) -> Option<&str> {
    node_str(node, "name")
        .map(str::trim)
        .filter(|name| !name.is_empty())
}

/// Reads an unsigned numeric attribute, tolerating numeric strings.
pub fn node_number(node: &ProxyNode, key: &str) -> Option<u64> {
    match node.get(key)? {
        Value::Number(value) => value.as_u64(),
        Value::String(value) => value.trim().parse().ok(),
        _ => None,
    }
}

/// Reads a `u16` attribute, tolerating numeric strings.
pub fn node_u16(node: &ProxyNode, key: &str) -> Option<u16> {
    node_number(node, key).and_then(|value| u16::try_from(value).ok())
}

/// Reads a boolean attribute, tolerating `"true"`/`"1"` style strings.
pub fn node_bool(node: &ProxyNode, key: &str) -> Option<bool> {
    match node.get(key)? {
        Value::Bool(value) => Some(*value),
        Value::Number(value) => value.as_u64().map(|value| value != 0),
        Value::String(value) => match value.trim().to_ascii_lowercase().as_str() {
            "true" | "1" | "yes" | "on" => Some(true),
            "false" | "0" | "no" | "off" => Some(false),
            _ => None,
        },
        _ => None,
    }
}

/// Reads a string list attribute, accepting sequences, numbers and CSV strings.
pub fn node_strings(node: &ProxyNode, key: &str) -> Vec<String> {
    node.get(key).map(value_strings).unwrap_or_default()
}

/// Reads a nested mapping attribute.
pub fn node_mapping<'a>(node: &'a ProxyNode, key: &str) -> Option<&'a Mapping> {
    node.get(key).and_then(Value::as_mapping)
}

/// Mapping lookup helper because `serde_yaml::Mapping` keys are `Value`s.
pub fn mapping_get<'a>(mapping: &'a Mapping, key: &str) -> Option<&'a Value> {
    mapping.get(Value::String(key.to_string()))
}

/// Reads a string attribute from a nested mapping.
pub fn mapping_str<'a>(mapping: &'a Mapping, key: &str) -> Option<&'a str> {
    mapping_get(mapping, key).and_then(Value::as_str)
}

/// Returns the first string of a scalar or sequence value.
pub fn value_first_string(value: &Value) -> Option<&str> {
    match value {
        Value::String(text) => Some(text.as_str()),
        Value::Sequence(items) => items.iter().find_map(Value::as_str),
        _ => None,
    }
}

/// Normalizes a YAML value into a list of strings.
pub fn value_strings(value: &Value) -> Vec<String> {
    match value {
        Value::String(text) => text
            .split(',')
            .map(str::trim)
            .filter(|item| !item.is_empty())
            .map(str::to_string)
            .collect(),
        Value::Sequence(items) => items
            .iter()
            .filter_map(Value::as_str)
            .map(str::trim)
            .filter(|item| !item.is_empty())
            .map(str::to_string)
            .collect(),
        Value::Number(number) => vec![number.to_string()],
        _ => Vec::new(),
    }
}

/// Reads the `path` of a WebSocket transport (`ws-opts.path`).
pub fn ws_path(node: &ProxyNode) -> Option<String> {
    node_mapping(node, "ws-opts")
        .and_then(|options| mapping_str(options, "path"))
        .map(str::to_string)
}

/// Reads the `Host` header of a WebSocket transport (`ws-opts.headers.Host`).
pub fn ws_host(node: &ProxyNode) -> Option<String> {
    let options = node_mapping(node, "ws-opts")?;
    let headers = mapping_get(options, "headers").and_then(Value::as_mapping)?;
    mapping_get(headers, "Host")
        .or_else(|| mapping_get(headers, "host"))
        .and_then(value_first_string)
        .map(str::to_string)
}

/// Reads the gRPC service name (`grpc-opts.grpc-service-name`).
pub fn grpc_service(node: &ProxyNode) -> Option<String> {
    node_mapping(node, "grpc-opts")
        .and_then(|options| mapping_str(options, "grpc-service-name"))
        .map(str::to_string)
}

/// Reads the path of any HTTP-like transport (`ws-opts`, `http-opts`, `h2-opts`).
pub fn transport_path(node: &ProxyNode) -> Option<String> {
    for key in ["ws-opts", "http-opts", "h2-opts"] {
        if let Some(options) = node_mapping(node, key) {
            if let Some(path) = mapping_get(options, "path")
                .and_then(value_first_string)
                .map(str::to_string)
            {
                return Some(path);
            }
        }
    }
    None
}

/// Reads the host of any HTTP-like transport (`ws-opts`, `http-opts`, `h2-opts`).
pub fn transport_host(node: &ProxyNode) -> Option<String> {
    if let Some(host) = ws_host(node) {
        return Some(host);
    }
    for key in ["http-opts", "h2-opts"] {
        if let Some(options) = node_mapping(node, key) {
            if let Some(host) = mapping_get(options, "headers")
                .and_then(Value::as_mapping)
                .and_then(|headers| {
                    mapping_get(headers, "Host").or_else(|| mapping_get(headers, "host"))
                })
                .and_then(value_first_string)
            {
                return Some(host.to_string());
            }
            if let Some(host) = mapping_get(options, "host")
                .and_then(value_first_string)
                .map(str::to_string)
            {
                return Some(host);
            }
        }
    }
    None
}

/// Clones a node without any VPN233-internal attributes.
pub fn strip_internal_keys(node: &ProxyNode) -> ProxyNode {
    node.iter()
        .filter(|(key, _)| !is_internal_key(key.as_str()))
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect()
}

/// Replaces characters that are unsafe in line oriented configurations.
pub fn clean_label(raw: &str) -> String {
    let mut cleaned = String::with_capacity(raw.len());
    for character in raw.chars() {
        if character.is_control() || NAME_REPLACEMENTS.contains(&character) {
            cleaned.push('_');
        } else {
            cleaned.push(character);
        }
    }
    cleaned.trim().to_string()
}

/// Cleans a node name and provides a stable fallback when it is empty.
pub fn clean_name(raw: &str, index: usize) -> String {
    let cleaned = clean_label(raw);
    if cleaned.is_empty() {
        format!("vpn233-{}", index + 1)
    } else {
        cleaned
    }
}

/// Cleans a profile name and falls back to the default profile label.
pub fn sanitize_profile_name(raw: &str) -> String {
    let cleaned = clean_label(raw);
    if cleaned.is_empty() {
        DEFAULT_PROFILE.to_string()
    } else {
        cleaned
    }
}

fn unique_name(base: &str, used: &mut BTreeSet<String>) -> String {
    if used.insert(base.to_string()) {
        return base.to_string();
    }
    let mut suffix = 2_u32;
    loop {
        let candidate = format!("{base}-{suffix}");
        if used.insert(candidate.clone()) {
            return candidate;
        }
        suffix += 1;
    }
}

fn prepare(nodes: &[ProxyNode], reserved: &[&str], line_safe: bool) -> Vec<(String, ProxyNode)> {
    let mut used: BTreeSet<String> = reserved.iter().map(|name| name.to_string()).collect();
    let mut prepared = Vec::with_capacity(nodes.len());
    for (index, node) in nodes.iter().enumerate() {
        let raw = node_str(node, "name").unwrap_or_default();
        let base = if line_safe {
            clean_name(raw, index)
        } else {
            raw.trim().to_string()
        };
        let base = if base.is_empty() {
            format!("vpn233-{}", index + 1)
        } else {
            base
        };
        let name = unique_name(&base, &mut used);
        let mut copy = strip_internal_keys(node);
        copy.insert("name".to_string(), string(name.clone()));
        prepared.push((name, copy));
    }
    prepared
}

/// Prepares nodes for structured renderers (Clash, sing-box): strips internal
/// attributes and de-duplicates names against each other and against the
/// renderer's reserved group names.
pub fn prepared_structured_nodes(
    nodes: &[ProxyNode],
    reserved: &[&str],
) -> Vec<(String, ProxyNode)> {
    prepare(nodes, reserved, false)
}

/// Prepares nodes for line oriented renderers (Surge, Loon, Quantumult X,
/// Surfboard): additionally escapes characters that would break the syntax.
pub fn prepared_line_nodes(nodes: &[ProxyNode], reserved: &[&str]) -> Vec<(String, ProxyNode)> {
    prepare(nodes, reserved, true)
}

/// Parses a bandwidth value such as `"50 Mbps"` or `"50"` into Mbps.
pub fn parse_bandwidth(raw: &str) -> Option<u64> {
    let raw = raw.trim();
    let split = raw
        .find(|character: char| !character.is_ascii_digit() && character != '.')
        .unwrap_or(raw.len());
    let amount = raw[..split].parse::<f64>().ok()?;
    let scale = match raw[split..].trim() {
        "" | "Mbps" | "mbps" => 1.0,
        "bps" => 0.000_001,
        "Kbps" | "kbps" => 0.001,
        "Gbps" | "gbps" => 1000.0,
        "Bps" => 0.000_008,
        "KBps" => 0.008,
        "MBps" => 8.0,
        "GBps" => 8000.0,
        _ => return None,
    };
    let mbps = amount * scale;
    // The target field is integer Mbps: round positive sub-Mbps limits up.
    (mbps.is_finite() && mbps > 0.0 && mbps < u64::MAX as f64).then(|| mbps.ceil() as u64)
}

/// Converts a YAML value into its JSON equivalent for sing-box rendering.
pub fn yaml_to_json(value: &Value) -> serde_json::Value {
    match value {
        Value::Null => serde_json::Value::Null,
        Value::Bool(flag) => serde_json::Value::Bool(*flag),
        Value::Number(number) => {
            if let Some(value) = number.as_u64() {
                serde_json::Value::from(value)
            } else if let Some(value) = number.as_i64() {
                serde_json::Value::from(value)
            } else if let Some(value) = number.as_f64() {
                serde_json::Value::from(value)
            } else {
                serde_json::Value::Null
            }
        }
        Value::String(text) => serde_json::Value::String(text.clone()),
        Value::Sequence(items) => {
            serde_json::Value::Array(items.iter().map(yaml_to_json).collect())
        }
        Value::Mapping(mapping) => {
            let mut object = serde_json::Map::new();
            for (key, value) in mapping {
                if let Some(key) = key.as_str() {
                    object.insert(key.to_string(), yaml_to_json(value));
                }
            }
            serde_json::Value::Object(object)
        }
        Value::Tagged(tagged) => yaml_to_json(&tagged.value),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proxy_node;

    #[test]
    fn internal_keys_are_detected_and_stripped() {
        let mut node = proxy_node("n", "ss", "example.invalid", 8388);
        node.insert("vpn233-link".into(), string("ss://x"));
        assert!(is_internal_key("vpn233-link"));
        assert!(!is_internal_key("server"));
        let stripped = strip_internal_keys(&node);
        assert!(!stripped.contains_key("vpn233-link"));
        assert!(stripped.contains_key("server"));
    }

    #[test]
    fn clean_label_replaces_reserved_characters() {
        assert_eq!(clean_label("a,b=c\"d\ne\tf"), "a_b_c_d_e_f");
        assert_eq!(clean_label("  plain name  "), "plain name");
    }

    #[test]
    fn clean_name_provides_index_fallback() {
        assert_eq!(clean_name("", 0), "vpn233-1");
        assert_eq!(clean_name("   ", 4), "vpn233-5");
        assert_eq!(clean_name("kept", 0), "kept");
    }

    #[test]
    fn profile_names_fall_back_to_default() {
        assert_eq!(sanitize_profile_name(""), DEFAULT_PROFILE);
        assert_eq!(sanitize_profile_name(" My,Profile "), "My_Profile");
    }

    #[test]
    fn node_accessors_tolerate_scalar_types() {
        let mut node = proxy_node("n", "ss", "example.invalid", 8388);
        node.insert("enabled".into(), Value::String("true".into()));
        node.insert("port-copy".into(), Value::String("443".into()));
        assert!(node_bool(&node, "enabled").unwrap());
        assert_eq!(node_u16(&node, "port-copy"), Some(443));
        assert_eq!(node_u16(&node, "port"), Some(8388));
        assert!(node_bool(&node, "missing").is_none());
    }

    #[test]
    fn prepared_nodes_are_deduplicated_and_reserved_safe() {
        let mut first = proxy_node("dup", "ss", "a.invalid", 1);
        first.insert("vpn233-x".into(), string("hidden"));
        let nodes = vec![
            first,
            proxy_node("dup", "ss", "b.invalid", 2),
            proxy_node("VPN233", "ss", "c.invalid", 3),
        ];
        let prepared = prepared_structured_nodes(&nodes, TEST_RESERVED);
        let names: Vec<&str> = prepared.iter().map(|(name, _)| name.as_str()).collect();
        assert_eq!(names, vec!["dup", "dup-2", "VPN233-2"]);
        assert!(!prepared[0].1.contains_key("vpn233-x"));
    }

    const TEST_RESERVED: &[&str] = &[MAIN_GROUP, PROXIES_GROUP, DIRECT_POLICY];

    #[test]
    fn line_safe_nodes_escape_unfriendly_characters() {
        let nodes = vec![proxy_node("a,b=c", "ss", "example.invalid", 1)];
        let prepared = prepared_line_nodes(&nodes, LINE_RESERVED);
        assert_eq!(prepared[0].0, "a_b_c");
    }

    #[test]
    fn parse_bandwidth_accepts_units() {
        assert_eq!(parse_bandwidth("50 Mbps"), Some(50));
        assert_eq!(parse_bandwidth(" 120"), Some(120));
        assert_eq!(parse_bandwidth("fast"), None);
        assert_eq!(parse_bandwidth("2 Gbps"), Some(2000));
        assert_eq!(parse_bandwidth("1.5 MBps"), Some(12));
        assert_eq!(parse_bandwidth("500 Kbps"), Some(1));
        assert_eq!(parse_bandwidth("50 invalid-unit"), None);
    }

    #[test]
    fn yaml_values_convert_to_json() {
        let mapping: Mapping = serde_yaml::from_str("a: 1\nb: true\nc: [x, y]").unwrap();
        let json = yaml_to_json(&Value::Mapping(mapping));
        assert_eq!(json["a"], serde_json::json!(1));
        assert_eq!(json["b"], serde_json::json!(true));
        assert_eq!(json["c"], serde_json::json!(["x", "y"]));
    }
}
