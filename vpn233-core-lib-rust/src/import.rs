//! Import node definitions from URI, Base64, Clash/Mihomo and sing-box feeds.
//!
//! Only proxy definitions are read: upstream DNS, rules, remote providers,
//! dashboards and probe URLs are never copied into generated profiles.

use serde_json::Value as Json;
use serde_yaml::{Mapping, Value};

use crate::{boolean, formats, link, nodes, number, proxy_node, string, LinkError, ProxyNode};

/// Parses a subscription without making network requests. Standard and URL-safe
/// Base64, optional padding, UTF-8 BOM, comments and CRLF are accepted. Errors
/// contain indexes and fixed descriptions, never the private input contents.
pub fn parse_subscription(input: &str) -> (Vec<ProxyNode>, Vec<LinkError>) {
    parse_payload(input, true)
}

fn parse_payload(input: &str, allow_base64: bool) -> (Vec<ProxyNode>, Vec<LinkError>) {
    let text = input.trim().trim_start_matches('\u{feff}').trim();
    if text.is_empty() {
        return (Vec::new(), Vec::new());
    }
    if let Ok(document) = serde_yaml::from_str::<Value>(text) {
        if let Some(proxies) = document.get("proxies") {
            return match proxies.as_sequence() {
                Some(proxies) => collect(
                    proxies
                        .iter()
                        .enumerate()
                        .map(|(index, value)| (index, import_clash(value, index))),
                ),
                None => invalid("Clash proxies must be a sequence"),
            };
        }
        if document.get("outbounds").is_some() {
            let Ok(json) = serde_json::from_str::<Json>(text) else {
                return invalid("sing-box subscription must be valid JSON");
            };
            let Some(outbounds) = json.get("outbounds").and_then(Json::as_array) else {
                return invalid("sing-box outbounds must be an array");
            };
            return collect(outbounds.iter().enumerate().filter_map(|(index, value)| {
                let kind = value.get("type").and_then(Json::as_str).unwrap_or("");
                // These are routing policies, not subscription nodes.
                if matches!(kind, "direct" | "block" | "dns" | "selector" | "urltest") {
                    None
                } else {
                    Some((index, import_singbox(value, index)))
                }
            }));
        }
        if let Some(proxies) = document
            .as_sequence()
            .filter(|items| items.iter().any(Value::is_mapping))
        {
            return collect(
                proxies
                    .iter()
                    .enumerate()
                    .map(|(index, value)| (index, import_clash(value, index))),
            );
        }
    }
    if allow_base64 && !text.contains("://") {
        if let Some(decoded) = link::decode_base64_text(text) {
            return parse_payload(&decoded, false);
        }
    }
    let mut parsed = Vec::new();
    let mut errors = Vec::new();
    for (index, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        match crate::parse_link(line, index) {
            Ok(node) => parsed.push(node),
            Err(error) => errors.push(error),
        }
    }
    (parsed, errors)
}

fn invalid(reason: &str) -> (Vec<ProxyNode>, Vec<LinkError>) {
    (Vec::new(), vec![LinkError::new(0, reason)])
}

fn collect(
    items: impl Iterator<Item = (usize, Result<ProxyNode, &'static str>)>,
) -> (Vec<ProxyNode>, Vec<LinkError>) {
    let mut parsed = Vec::new();
    let mut errors = Vec::new();
    for (index, result) in items {
        match result {
            Ok(node) => parsed.push(node),
            Err(reason) => errors.push(LinkError::new(index, reason)),
        }
    }
    (parsed, errors)
}

fn canonical_protocol(raw: &str) -> String {
    match raw.trim().to_ascii_lowercase().as_str() {
        "shadowsocks" => "ss".into(),
        "shadowsocksr" => "ssr".into(),
        "hy2" => "hysteria2".into(),
        "socks" | "socks5h" => "socks5".into(),
        "https" => "http".into(),
        other => other.into(),
    }
}

fn import_clash(value: &Value, index: usize) -> Result<ProxyNode, &'static str> {
    let mut node: ProxyNode = serde_yaml::from_value(value.clone())
        .map_err(|_| "proxy definition must be a string-keyed mapping")?;
    node = nodes::strip_internal_keys(&node);
    let original = nodes::node_type_lower(&node).ok_or("proxy definition is missing a type")?;
    let kind = canonical_protocol(&original);
    if !formats::SUPPORTED_PROTOCOLS.contains(&kind.as_str()) {
        return Err("unsupported proxy protocol");
    }
    if original == "https" {
        node.insert("tls".into(), boolean(true));
    }
    node.insert("type".into(), string(kind));
    validate_node(&mut node, index)?;
    Ok(node)
}

fn validate_node(node: &mut ProxyNode, index: usize) -> Result<(), &'static str> {
    let server = nodes::node_server(node).ok_or("proxy definition is missing a server")?;
    if server.chars().any(|ch| {
        ch.is_control() || ch.is_whitespace() || matches!(ch, '/' | '@' | ',' | '#' | ';')
    }) {
        return Err("proxy definition has an invalid server");
    }
    let port = nodes::node_u16(node, "port")
        .filter(|port| *port > 0)
        .ok_or("proxy definition has an invalid port")?;
    // Exporters consume a single endpoint. Do not silently flatten a range.
    if node.contains_key("port-range") {
        return Err("port-range proxy definitions require a concrete port");
    }
    if nodes::node_str(node, "dialer-proxy").is_some_and(|value| !value.is_empty()) {
        return Err("proxy chains with dialer-proxy are not supported");
    }
    node.insert("port".into(), number(port.into()));
    if nodes::node_name(node).is_none() {
        node.insert("name".into(), string(format!("vpn233-{}", index + 1)));
    }
    Ok(())
}

fn copy_json(node: &mut ProxyNode, source: &Json, key: &str, target: &str) {
    if let Some(value) = source.get(key) {
        if let Ok(value) = serde_yaml::to_value(value) {
            node.insert(target.into(), value);
        }
    }
}

fn import_singbox(value: &Json, index: usize) -> Result<ProxyNode, &'static str> {
    let raw_kind = value
        .get("type")
        .and_then(Json::as_str)
        .ok_or("outbound is missing a type")?;
    let kind = canonical_protocol(raw_kind);
    if !formats::SubscriptionFormat::SingBox.supports_protocol(&kind) {
        return Err("unsupported sing-box outbound protocol");
    }
    if raw_kind == "socks"
        && value
            .get("version")
            .and_then(Json::as_str)
            .is_some_and(|version| version != "5")
    {
        return Err("only SOCKS5 outbounds can be imported");
    }
    if value.get("detour").is_some() {
        return Err("outbound chains with detour are not supported");
    }
    let server = value
        .get("server")
        .and_then(Json::as_str)
        .ok_or("outbound is missing a server")?;
    let port = value
        .get("server_port")
        .and_then(Json::as_u64)
        .and_then(|port| u16::try_from(port).ok())
        .filter(|port| *port > 0)
        .ok_or("outbound has an invalid server_port")?;
    let name = value.get("tag").and_then(Json::as_str).unwrap_or("");
    let mut node = proxy_node(name, &kind, server, port);
    for (source, target) in [
        ("uuid", "uuid"),
        ("password", "password"),
        ("username", "username"),
        ("method", "cipher"),
        ("flow", "flow"),
        ("alter_id", "alterId"),
        ("congestion_control", "congestion-controller"),
        ("udp_relay_mode", "udp-relay-mode"),
        ("zero_rtt_handshake", "reduce-rtt"),
        ("up_mbps", "up"),
        ("down_mbps", "down"),
        ("min_idle_session", "min-idle-session"),
    ] {
        copy_json(&mut node, value, source, target);
    }
    if kind == "vmess" {
        copy_json(&mut node, value, "security", "cipher");
    }
    if kind == "anytls" {
        for (source, target) in [
            ("idle_session_check_interval", "idle-session-check-interval"),
            ("idle_session_timeout", "idle-session-timeout"),
        ] {
            if let Some(value) = value.get(source) {
                let raw = value
                    .as_str()
                    .ok_or("AnyTLS session intervals must use duration strings")?;
                let (digits, factor) = if let Some(digits) = raw.strip_suffix('s') {
                    (digits, 1)
                } else if let Some(digits) = raw.strip_suffix('m') {
                    (digits, 60)
                } else if let Some(digits) = raw.strip_suffix('h') {
                    (digits, 3600)
                } else {
                    return Err("unsupported AnyTLS session duration");
                };
                let seconds = digits
                    .parse::<u64>()
                    .ok()
                    .and_then(|seconds| seconds.checked_mul(factor))
                    .ok_or("invalid AnyTLS session duration")?;
                node.insert(target.into(), number(seconds));
            }
        }
    }
    if let Some(tls) = value
        .get("tls")
        .filter(|tls| tls.get("enabled").and_then(Json::as_bool).unwrap_or(false))
    {
        node.insert("tls".into(), boolean(true));
        copy_json(&mut node, tls, "server_name", "servername");
        copy_json(&mut node, tls, "insecure", "skip-cert-verify");
        copy_json(&mut node, tls, "alpn", "alpn");
        copy_json(&mut node, tls, "certificate", "certificate");
        if let Some(utls) = tls
            .get("utls")
            .filter(|utls| utls.get("enabled").and_then(Json::as_bool).unwrap_or(false))
        {
            copy_json(&mut node, utls, "fingerprint", "client-fingerprint");
        }
        if let Some(reality) = tls.get("reality").filter(|reality| {
            reality
                .get("enabled")
                .and_then(Json::as_bool)
                .unwrap_or(false)
        }) {
            let mut options = Mapping::new();
            for (source, target) in [("public_key", "public-key"), ("short_id", "short-id")] {
                if let Some(value) = reality.get(source).and_then(Json::as_str) {
                    options.insert(string(target), string(value));
                }
            }
            node.insert("reality-opts".into(), Value::Mapping(options));
        }
    }
    if let Some(obfs) = value.get("obfs") {
        copy_json(&mut node, obfs, "type", "obfs");
        copy_json(&mut node, obfs, "password", "obfs-password");
    }
    if let Some(transport) = value.get("transport") {
        import_transport(&mut node, transport)?;
    }
    if let Some(plugin) = value.get("plugin").and_then(Json::as_str) {
        // Reuse SIP002 plugin option normalization without reconstructing a URI.
        let options = value
            .get("plugin_opts")
            .and_then(Json::as_str)
            .unwrap_or("");
        let parameter = format!("{plugin};{options}");
        link::apply_ss_plugin(
            &std::collections::BTreeMap::from([("plugin".into(), parameter)]),
            &mut node,
        );
    }
    validate_node(&mut node, index)?;
    Ok(node)
}

fn import_transport(node: &mut ProxyNode, transport: &Json) -> Result<(), &'static str> {
    let kind = transport
        .get("type")
        .and_then(Json::as_str)
        .ok_or("transport is missing a type")?;
    let mut options = Mapping::new();
    let (network, key) = match kind {
        "ws" | "httpupgrade" => ("ws", "ws-opts"),
        "grpc" => ("grpc", "grpc-opts"),
        "http" => ("h2", "h2-opts"),
        _ => return Err("unsupported sing-box transport"),
    };
    if let Some(path) = transport.get("path").and_then(Json::as_str) {
        options.insert(string("path"), string(path));
    }
    if kind == "grpc" {
        if let Some(service) = transport.get("service_name").and_then(Json::as_str) {
            options.insert(string("grpc-service-name"), string(service));
        }
    } else if network == "ws" {
        if let Some(headers) = transport.get("headers") {
            options.insert(
                string("headers"),
                serde_yaml::to_value(headers).map_err(|_| "invalid transport headers")?,
            );
        }
        for (source, target) in [
            ("max_early_data", "max-early-data"),
            ("early_data_header_name", "early-data-header-name"),
        ] {
            if let Some(value) = transport.get(source) {
                options.insert(
                    string(target),
                    serde_yaml::to_value(value).map_err(|_| "invalid early-data option")?,
                );
            }
        }
        if kind == "httpupgrade" {
            options.insert(string("v2ray-http-upgrade"), boolean(true));
            if let Some(host) = transport.get("host").and_then(Json::as_str) {
                let mut headers = Mapping::new();
                headers.insert(string("Host"), string(host));
                options.insert(string("headers"), Value::Mapping(headers));
            }
        }
    } else if let Some(host) = transport.get("host") {
        options.insert(
            string("host"),
            serde_yaml::to_value(host).map_err(|_| "invalid HTTP transport host")?,
        );
    }
    node.insert("network".into(), string(network));
    node.insert(key.into(), Value::Mapping(options));
    Ok(())
}

#[cfg(test)]
mod tests;
