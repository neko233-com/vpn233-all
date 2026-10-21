//! sing-box JSON configuration rendering.

use serde_json::{json, Value as JsonValue};
use serde_yaml::Value;

use crate::nodes;
use crate::ProxyNode;

type JsonMap = serde_json::Map<String, JsonValue>;

/// Tags that nodes may never reuse because generated outbounds own them.
const RESERVED_TAGS: &[&str] = &[
    nodes::MAIN_GROUP,
    nodes::AUTO_GROUP,
    nodes::FALLBACK_GROUP,
    nodes::PROXIES_GROUP,
    "direct",
    "mixed-in",
];

/// Renders a complete sing-box configuration for the given nodes.
///
/// Nodes with protocols sing-box cannot express are skipped. The output is a
/// JSON object suitable for `sing-box run -c`.
pub fn render_singbox_config(nodes: &[ProxyNode]) -> String {
    let mut outbounds: Vec<JsonValue> = Vec::new();
    let mut tags: Vec<String> = Vec::new();
    for (tag, node) in nodes::prepared_structured_nodes(nodes, RESERVED_TAGS) {
        if let Some(outbound) = convert_node(&tag, &node) {
            outbounds.push(JsonValue::Object(outbound));
            tags.push(tag);
        }
    }
    outbounds.push(selector_outbound(&tags));
    outbounds.push(url_test_outbound(&tags));
    outbounds.push(json!({"type": "direct", "tag": "direct"}));
    let private_cidrs: Vec<&str> = nodes::PRIVATE_CIDRS
        .iter()
        .chain(nodes::PRIVATE_IPV6_CIDRS)
        .copied()
        .collect();
    let config = json!({
        "log": {"level": "info", "timestamp": true},
        "inbounds": [{
            "type": "mixed",
            "tag": "mixed-in",
            "listen": "127.0.0.1",
            "listen_port": nodes::MIXED_PORT,
        }],
        "outbounds": outbounds,
        "route": {
            "rules": [{"ip_cidr": private_cidrs, "outbound": "direct"}],
            "final": nodes::MAIN_GROUP,
        },
    });
    let body = serde_json::to_string_pretty(&config).unwrap_or_else(|_| "{}".to_string());
    format!("{body}\n")
}

fn selector_outbound(tags: &[String]) -> JsonValue {
    let mut outbounds: Vec<JsonValue> = tags.iter().map(|tag| json!(tag)).collect();
    outbounds.push(json!(nodes::AUTO_GROUP));
    outbounds.push(json!("direct"));
    json!({
        "type": "selector",
        "tag": nodes::MAIN_GROUP,
        "outbounds": outbounds,
        "default": nodes::AUTO_GROUP,
    })
}

fn url_test_outbound(tags: &[String]) -> JsonValue {
    let outbounds: Vec<JsonValue> = if tags.is_empty() {
        vec![json!("direct")]
    } else {
        tags.iter().map(|tag| json!(tag)).collect()
    };
    json!({
        "type": "selector",
        "tag": nodes::AUTO_GROUP,
        "outbounds": outbounds,
    })
}

fn insert_text(target: &mut JsonMap, key: &str, value: Option<&str>) {
    if let Some(value) = value {
        let value = value.trim();
        if !value.is_empty() {
            target.insert(key.to_string(), JsonValue::String(value.to_string()));
        }
    }
}

fn insert_bool(target: &mut JsonMap, key: &str, value: bool) {
    target.insert(key.to_string(), JsonValue::Bool(value));
}

fn insert_number(target: &mut JsonMap, key: &str, value: u64) {
    target.insert(key.to_string(), JsonValue::from(value));
}

fn tls_object(node: &ProxyNode, enabled: bool) -> Option<JsonValue> {
    let reality = nodes::node_mapping(node, "reality-opts");
    if !enabled && reality.is_none() {
        return None;
    }
    let mut tls = JsonMap::new();
    insert_bool(&mut tls, "enabled", true);
    insert_text(
        &mut tls,
        "server_name",
        nodes::node_str(node, "servername").or_else(|| nodes::node_str(node, "sni")),
    );
    if nodes::node_bool(node, "skip-cert-verify").unwrap_or(false) {
        insert_bool(&mut tls, "insecure", true);
    }
    let alpn = nodes::node_strings(node, "alpn");
    if !alpn.is_empty() {
        tls.insert("alpn".to_string(), json!(alpn));
    }
    if let Some(fingerprint) = nodes::node_str(node, "client-fingerprint") {
        let mut utls = JsonMap::new();
        insert_bool(&mut utls, "enabled", true);
        insert_text(&mut utls, "fingerprint", Some(fingerprint));
        tls.insert("utls".to_string(), JsonValue::Object(utls));
    }
    if let Some(reality) = reality {
        let mut value = JsonMap::new();
        insert_bool(&mut value, "enabled", true);
        insert_text(
            &mut value,
            "public_key",
            nodes::mapping_str(reality, "public-key"),
        );
        insert_text(
            &mut value,
            "short_id",
            nodes::mapping_str(reality, "short-id"),
        );
        tls.insert("reality".to_string(), JsonValue::Object(value));
    }
    Some(JsonValue::Object(tls))
}

fn transport_object(node: &ProxyNode) -> Option<JsonValue> {
    let network = nodes::node_network(node);
    let mut transport = JsonMap::new();
    match network.as_str() {
        "ws" | "httpupgrade" => {
            transport.insert("type".to_string(), json!(network));
            transport.insert(
                "path".to_string(),
                json!(nodes::ws_path(node).unwrap_or_else(|| "/".to_string())),
            );
            if let Some(host) = nodes::ws_host(node) {
                if network == "httpupgrade" {
                    transport.insert("host".to_string(), json!(host));
                } else {
                    let mut headers = JsonMap::new();
                    headers.insert("Host".to_string(), json!(host));
                    transport.insert("headers".to_string(), JsonValue::Object(headers));
                }
            }
            if network == "ws" {
                if let Some(options) = nodes::node_mapping(node, "ws-opts") {
                    if let Some(headers) = nodes::mapping_get(options, "headers") {
                        transport.insert("headers".into(), nodes::yaml_to_json(headers));
                    }
                    for (source, target) in [
                        ("max-early-data", "max_early_data"),
                        ("early-data-header-name", "early_data_header_name"),
                    ] {
                        if let Some(value) = nodes::mapping_get(options, source) {
                            transport.insert(target.into(), nodes::yaml_to_json(value));
                        }
                    }
                }
            }
        }
        "grpc" => {
            transport.insert("type".to_string(), json!("grpc"));
            if let Some(service) = nodes::grpc_service(node) {
                transport.insert("service_name".to_string(), json!(service));
            }
        }
        "http" | "h2" => {
            transport.insert("type".to_string(), json!("http"));
            transport.insert(
                "path".to_string(),
                json!(nodes::transport_path(node).unwrap_or_else(|| "/".to_string())),
            );
            let hosts = nodes::node_mapping(node, "h2-opts")
                .and_then(|options| nodes::mapping_get(options, "host"))
                .map(nodes::value_strings)
                .unwrap_or_default();
            if !hosts.is_empty() {
                transport.insert("host".to_string(), json!(hosts));
            } else if let Some(host) = nodes::transport_host(node) {
                transport.insert("host".to_string(), json!([host]));
            }
        }
        _ => return None,
    }
    Some(JsonValue::Object(transport))
}

fn shadowsocks_plugin(node: &ProxyNode) -> Option<(String, String)> {
    let plugin = nodes::node_str(node, "plugin")?.trim();
    if plugin.is_empty() {
        return None;
    }
    let options = nodes::node_mapping(node, "plugin-opts");
    match plugin.to_ascii_lowercase().as_str() {
        "obfs" => {
            let mode = options
                .and_then(|options| nodes::mapping_str(options, "mode"))
                .unwrap_or("http");
            let mut plugin_options = format!("obfs={mode}");
            if let Some(host) = options.and_then(|options| nodes::mapping_str(options, "host")) {
                plugin_options.push_str(&format!(";obfs-host={host}"));
            }
            Some(("obfs-local".to_string(), plugin_options))
        }
        "v2ray-plugin" => {
            let mut parts: Vec<String> = Vec::new();
            if let Some(options) = options {
                for key in ["mode", "host", "path"] {
                    if let Some(value) = nodes::mapping_str(options, key) {
                        parts.push(format!("{key}={value}"));
                    }
                }
                for key in ["tls", "mux"] {
                    if nodes::mapping_get(options, key)
                        .and_then(Value::as_bool)
                        .unwrap_or(false)
                    {
                        parts.push(key.to_string());
                    }
                }
            }
            Some(("v2ray-plugin".to_string(), parts.join(";")))
        }
        other => Some((other.to_string(), String::new())),
    }
}

fn convert_node(tag: &str, node: &ProxyNode) -> Option<JsonMap> {
    let kind = nodes::node_type_lower(node)?;
    let server = nodes::node_server(node)?;
    let port = u64::from(nodes::node_u16(node, "port")?);
    let mut outbound = JsonMap::new();
    insert_text(&mut outbound, "tag", Some(tag));
    insert_text(&mut outbound, "server", Some(server));
    insert_number(&mut outbound, "server_port", port);
    let tls = nodes::node_bool(node, "tls").unwrap_or(false);
    match kind.as_str() {
        "vless" => {
            outbound.insert("type".to_string(), json!("vless"));
            insert_text(&mut outbound, "uuid", nodes::node_str(node, "uuid"));
            insert_text(&mut outbound, "flow", nodes::node_str(node, "flow"));
            if let Some(tls) = tls_object(node, tls) {
                outbound.insert("tls".to_string(), tls);
            }
            if let Some(transport) = transport_object(node) {
                outbound.insert("transport".to_string(), transport);
            }
        }
        "vmess" => {
            outbound.insert("type".to_string(), json!("vmess"));
            insert_text(&mut outbound, "uuid", nodes::node_str(node, "uuid"));
            insert_text(
                &mut outbound,
                "security",
                Some(nodes::node_str(node, "cipher").unwrap_or("auto")),
            );
            if let Some(alter_id) = nodes::node_number(node, "alterId").filter(|value| *value > 0) {
                insert_number(&mut outbound, "alter_id", alter_id);
            }
            if let Some(tls) = tls_object(node, tls) {
                outbound.insert("tls".to_string(), tls);
            }
            if let Some(transport) = transport_object(node) {
                outbound.insert("transport".to_string(), transport);
            }
        }
        "trojan" => {
            outbound.insert("type".to_string(), json!("trojan"));
            insert_text(&mut outbound, "password", nodes::node_str(node, "password"));
            if let Some(tls) = tls_object(node, true) {
                outbound.insert("tls".to_string(), tls);
            }
            if let Some(transport) = transport_object(node) {
                outbound.insert("transport".to_string(), transport);
            }
        }
        "ss" => {
            outbound.insert("type".to_string(), json!("shadowsocks"));
            insert_text(&mut outbound, "method", nodes::node_str(node, "cipher"));
            insert_text(&mut outbound, "password", nodes::node_str(node, "password"));
            if let Some((plugin, plugin_options)) = shadowsocks_plugin(node) {
                insert_text(&mut outbound, "plugin", Some(&plugin));
                insert_text(&mut outbound, "plugin_opts", Some(&plugin_options));
            }
        }
        "hysteria2" | "hy2" => {
            outbound.insert("type".to_string(), json!("hysteria2"));
            insert_text(&mut outbound, "password", nodes::node_str(node, "password"));
            if let Some(tls) = tls_object(node, true) {
                outbound.insert("tls".to_string(), tls);
            }
            if let Some(obfs) = nodes::node_str(node, "obfs").filter(|value| !value.is_empty()) {
                let mut obfs_object = JsonMap::new();
                obfs_object.insert("type".to_string(), json!(obfs));
                insert_text(
                    &mut obfs_object,
                    "password",
                    nodes::node_str(node, "obfs-password"),
                );
                outbound.insert("obfs".to_string(), JsonValue::Object(obfs_object));
            }
            if let Some(up) = nodes::node_str(node, "up")
                .and_then(nodes::parse_bandwidth)
                .or_else(|| nodes::node_number(node, "up"))
            {
                insert_number(&mut outbound, "up_mbps", up);
            }
            if let Some(down) = nodes::node_str(node, "down")
                .and_then(nodes::parse_bandwidth)
                .or_else(|| nodes::node_number(node, "down"))
            {
                insert_number(&mut outbound, "down_mbps", down);
            }
        }
        "tuic" => {
            outbound.insert("type".to_string(), json!("tuic"));
            insert_text(&mut outbound, "uuid", nodes::node_str(node, "uuid"));
            insert_text(&mut outbound, "password", nodes::node_str(node, "password"));
            insert_text(
                &mut outbound,
                "congestion_control",
                nodes::node_str(node, "congestion-controller"),
            );
            insert_text(
                &mut outbound,
                "udp_relay_mode",
                nodes::node_str(node, "udp-relay-mode"),
            );
            if let Some(reduce_rtt) = nodes::node_bool(node, "reduce-rtt") {
                insert_bool(&mut outbound, "zero_rtt_handshake", reduce_rtt);
            }
            if let Some(tls) = tls_object(node, true) {
                outbound.insert("tls".to_string(), tls);
            }
        }
        "anytls" => {
            outbound.insert("type".to_string(), json!("anytls"));
            insert_text(&mut outbound, "password", nodes::node_str(node, "password"));
            for (source, target) in [
                ("idle-session-check-interval", "idle_session_check_interval"),
                ("idle-session-timeout", "idle_session_timeout"),
            ] {
                if let Some(seconds) = nodes::node_number(node, source) {
                    outbound.insert(target.into(), json!(format!("{seconds}s")));
                }
            }
            if let Some(sessions) = nodes::node_number(node, "min-idle-session") {
                insert_number(&mut outbound, "min_idle_session", sessions);
            }
            if let Some(tls) = tls_object(node, true) {
                outbound.insert("tls".to_string(), tls);
            }
        }
        "socks5" | "socks" => {
            outbound.insert("type".to_string(), json!("socks"));
            insert_text(&mut outbound, "version", Some("5"));
            insert_text(&mut outbound, "username", nodes::node_str(node, "username"));
            insert_text(&mut outbound, "password", nodes::node_str(node, "password"));
        }
        "http" | "https" => {
            outbound.insert("type".to_string(), json!("http"));
            insert_text(&mut outbound, "username", nodes::node_str(node, "username"));
            insert_text(&mut outbound, "password", nodes::node_str(node, "password"));
            if let Some(tls) = tls_object(node, tls) {
                outbound.insert("tls".to_string(), tls);
            }
        }
        _ => return None,
    }
    Some(outbound)
}

#[cfg(test)]
mod tests;
