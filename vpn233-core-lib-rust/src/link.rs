//! Subscription link parsing and rendering.
//!
//! The parser accepts the link formats that v2rayN, v2rayNG, Shadowrocket,
//! NekoBox, sing-box and the other clients served by VPN233 produce, and maps
//! them onto the Clash/Mihomo field names used by [`ProxyNode`](crate::ProxyNode).
//!
//! Parsing never panics: invalid input is reported as [`LinkError`] with the
//! line index and a human readable reason. Every parsed node keeps the original
//! link in the internal `vpn233-link` attribute so link based subscriptions can
//! round-trip without re-synthesis.

use std::collections::BTreeMap;
use std::fmt;

use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use base64::Engine as _;
use percent_encoding::{percent_decode_str, utf8_percent_encode, AsciiSet, NON_ALPHANUMERIC};
use serde_yaml::{Mapping, Value};
use url::Url;

use crate::nodes;
use crate::{boolean, number, proxy_node, string, ProxyNode};

/// Internal attribute that stores the original share link of a node.
pub const LINK_KEY: &str = "vpn233-link";

const COMPONENT: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'~');

/// Error returned when a single subscription link cannot be parsed or rendered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkError {
    /// Zero based index of the link inside the subscription payload.
    pub index: usize,
    /// Human readable reason, without any credential material.
    pub reason: String,
}

impl LinkError {
    /// Builds a new link error.
    pub fn new(index: usize, reason: impl Into<String>) -> Self {
        Self {
            index,
            reason: reason.into(),
        }
    }

    /// Returns the human readable reason.
    pub fn message(&self) -> &str {
        &self.reason
    }
}

impl fmt::Display for LinkError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "link #{}: {}", self.index, self.reason)
    }
}

impl std::error::Error for LinkError {}

/// Parses one share link.
pub fn parse_link(raw: &str, index: usize) -> Result<ProxyNode, LinkError> {
    parse_one(raw).map_err(|reason| LinkError::new(index, reason))
}

/// Parses an iterator of share links, skipping malformed entries.
///
/// The returned tuple contains the successfully parsed nodes in input order and
/// the errors for the skipped entries.
pub fn parse_links<I, S>(raw_links: I) -> (Vec<ProxyNode>, Vec<LinkError>)
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut nodes = Vec::new();
    let mut errors = Vec::new();
    for (index, raw) in raw_links.into_iter().enumerate() {
        match parse_link(raw.as_ref(), index) {
            Ok(node) => nodes.push(node),
            Err(error) => errors.push(error),
        }
    }
    (nodes, errors)
}

fn parse_one(raw: &str) -> Result<ProxyNode, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("empty link".to_string());
    }
    if trimmed.chars().any(char::is_control) {
        return Err("link contains control characters".to_string());
    }
    let (scheme, rest) = trimmed
        .split_once("://")
        .ok_or_else(|| "link is missing a scheme separator".to_string())?;
    let scheme = scheme.trim().to_ascii_lowercase();
    let mut node = match scheme.as_str() {
        "vless" => parse_vless(trimmed)?,
        "vmess" => parse_vmess(rest)?,
        "trojan" => parse_trojan(trimmed)?,
        "ss" => parse_ss(rest)?,
        "ssr" => parse_ssr(rest)?,
        "hysteria2" | "hy2" => parse_hysteria2(trimmed)?,
        "tuic" => parse_tuic(trimmed)?,
        "anytls" => parse_anytls(trimmed)?,
        "socks5" | "socks5h" | "socks" => parse_socks5(trimmed)?,
        "http" | "https" => parse_http(trimmed)?,
        _ => return Err("unsupported link scheme".to_string()),
    };
    node.insert(LINK_KEY.to_string(), string(trimmed));
    Ok(node)
}

// ---------------------------------------------------------------------------
// Encoding helpers
// ---------------------------------------------------------------------------

fn decode_component(raw: &str) -> String {
    percent_decode_str(raw).decode_utf8_lossy().into_owned()
}

fn encode_component(raw: &str) -> String {
    utf8_percent_encode(raw, COMPONENT).to_string()
}

fn decode_base64(input: &str) -> Option<Vec<u8>> {
    let filtered: String = input
        .chars()
        .filter(|character| !character.is_whitespace())
        .collect();
    if filtered.is_empty() {
        return None;
    }
    let mut normalized: String = filtered
        .chars()
        .map(|character| match character {
            '-' => '+',
            '_' => '/',
            other => other,
        })
        .collect();
    if normalized.len() % 4 == 1 {
        return None;
    }
    while !normalized.len().is_multiple_of(4) {
        normalized.push('=');
    }
    STANDARD.decode(normalized.as_bytes()).ok()
}

pub(crate) fn decode_base64_text(input: &str) -> Option<String> {
    decode_base64(input).and_then(|bytes| String::from_utf8(bytes).ok())
}

fn split_fragment(input: &str) -> (&str, Option<String>) {
    match input.split_once('#') {
        Some((body, fragment)) => (body, Some(decode_component(fragment))),
        None => (input, None),
    }
}

fn parse_query(raw: &str) -> BTreeMap<String, String> {
    let mut query = BTreeMap::new();
    for pair in raw.split('&') {
        if pair.is_empty() {
            continue;
        }
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        query
            .entry(decode_component(key).trim().to_ascii_lowercase())
            .or_insert_with(|| decode_component(value));
    }
    query
}

fn split_query(input: &str) -> (&str, BTreeMap<String, String>) {
    match input.split_once('?') {
        Some((body, query)) => (body.trim_end_matches('/'), parse_query(query)),
        None => (input, BTreeMap::new()),
    }
}

fn option_non_empty(value: String) -> Option<String> {
    let value = value.trim().to_string();
    if value.is_empty() {
        None
    } else {
        Some(value)
    }
}

// ---------------------------------------------------------------------------
// URL parsing
// ---------------------------------------------------------------------------

struct LinkUrl {
    scheme: String,
    username: Option<String>,
    password: Option<String>,
    host: String,
    port: u16,
    query: BTreeMap<String, String>,
    fragment: Option<String>,
}

impl LinkUrl {
    fn query_value(&self, keys: &[&str]) -> Option<&str> {
        keys.iter()
            .find_map(|key| self.query.get(*key).map(String::as_str))
            .map(str::trim)
            .filter(|value| !value.is_empty())
    }

    fn display_name(&self, scheme: &str) -> String {
        match self.fragment.as_deref().map(str::trim) {
            Some(fragment) if !fragment.is_empty() => fragment.to_string(),
            _ => format!("{scheme}-{}", self.host),
        }
    }

    fn skip_cert_verify(&self) -> bool {
        self.query_value(&[
            "insecure",
            "allowinsecure",
            "allow_insecure",
            "skip-cert-verify",
            "skip_cert_verify",
        ])
        .map(|value| {
            matches!(
                value.to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(false)
    }
}

fn parse_link_url(raw: &str, default_port: u16) -> Result<LinkUrl, String> {
    let parsed = Url::parse(raw).map_err(|error| format!("invalid URL: {error}"))?;
    let host = parsed
        .host_str()
        .ok_or_else(|| "link is missing a host".to_string())?;
    let host = host
        .trim_start_matches('[')
        .trim_end_matches(']')
        .to_string();
    let port = parsed
        .port()
        .or_else(|| parsed.port_or_known_default())
        .unwrap_or(default_port);
    if port == 0 {
        return Err("link has an invalid port".to_string());
    }
    let username = option_non_empty(decode_component(parsed.username()));
    let password = parsed.password().map(decode_component);
    let mut query = BTreeMap::new();
    for (key, value) in parsed.query_pairs() {
        query
            .entry(key.trim().to_ascii_lowercase())
            .or_insert_with(|| value.into_owned());
    }
    let fragment = parsed
        .fragment()
        .map(decode_component)
        .filter(|fragment| !fragment.trim().is_empty());
    Ok(LinkUrl {
        scheme: parsed.scheme().to_ascii_lowercase(),
        username,
        password,
        host,
        port,
        query,
        fragment,
    })
}

// ---------------------------------------------------------------------------
// Transport helpers
// ---------------------------------------------------------------------------

fn normalize_network(raw: &str) -> Result<String, String> {
    let network = match raw.trim().to_ascii_lowercase().as_str() {
        "" | "tcp" | "raw" => "tcp",
        "ws" | "websocket" => "ws",
        "grpc" | "gun" => "grpc",
        "http" => "http",
        "httpupgrade" | "http-upgrade" => "httpupgrade",
        "h2" | "http2" | "h2mux" => "h2",
        _ => return Err("unsupported transport".to_string()),
    };
    Ok(network.to_string())
}

fn apply_transport(
    node: &mut ProxyNode,
    network: &str,
    path: Option<&str>,
    host: Option<&str>,
    service_name: Option<&str>,
) {
    match network {
        "ws" | "httpupgrade" => {
            let mut options = Mapping::new();
            options.insert(string("path"), string(path.unwrap_or("/")));
            if let Some(host) = host {
                let mut headers = Mapping::new();
                headers.insert(string("Host"), string(host));
                options.insert(string("headers"), Value::Mapping(headers));
            }
            node.insert("ws-opts".to_string(), Value::Mapping(options));
            if network == "httpupgrade" {
                // Mihomo represents HTTPUpgrade with the WebSocket options.
                node.insert("network".to_string(), string("ws"));
                if let Some(Value::Mapping(options)) = node.get_mut("ws-opts") {
                    options.insert(string("v2ray-http-upgrade"), boolean(true));
                }
            }
        }
        "grpc" => {
            let mut options = Mapping::new();
            if let Some(service_name) = service_name {
                options.insert(string("grpc-service-name"), string(service_name));
            }
            node.insert("grpc-opts".to_string(), Value::Mapping(options));
        }
        "http" => {
            let mut options = Mapping::new();
            options.insert(
                string("path"),
                Value::Sequence(vec![string(path.unwrap_or("/"))]),
            );
            if let Some(host) = host {
                let mut headers = Mapping::new();
                headers.insert(string("Host"), Value::Sequence(vec![string(host)]));
                options.insert(string("headers"), Value::Mapping(headers));
            }
            node.insert("http-opts".to_string(), Value::Mapping(options));
        }
        "h2" => {
            let mut options = Mapping::new();
            if let Some(path) = path {
                options.insert(string("path"), string(path));
            }
            if let Some(host) = host {
                options.insert(string("host"), Value::Sequence(vec![string(host)]));
            }
            node.insert("h2-opts".to_string(), Value::Mapping(options));
        }
        _ => {}
    }
}

fn insert_text(node: &mut ProxyNode, key: &str, value: &str) {
    let value = value.trim();
    if !value.is_empty() {
        node.insert(key.to_string(), string(value));
    }
}

fn insert_optional_text(node: &mut ProxyNode, key: &str, value: Option<&str>) {
    if let Some(value) = value {
        insert_text(node, key, value);
    }
}

fn insert_number(node: &mut ProxyNode, key: &str, value: u64) {
    node.insert(key.to_string(), number(value));
}

fn insert_boolean(node: &mut ProxyNode, key: &str, value: bool) {
    node.insert(key.to_string(), boolean(value));
}

fn insert_alpn(node: &mut ProxyNode, raw: &str) {
    let protocols: Vec<Value> = raw
        .split(',')
        .map(str::trim)
        .filter(|protocol| !protocol.is_empty())
        .map(string)
        .collect();
    if !protocols.is_empty() {
        node.insert("alpn".to_string(), Value::Sequence(protocols));
    }
}

fn apply_common_options(node: &mut ProxyNode, url: &LinkUrl, default_sni_key: &str) {
    insert_optional_text(
        node,
        default_sni_key,
        url.query_value(&["sni", "servername", "peer"]),
    );
    if let Some(alpn) = url.query_value(&["alpn"]) {
        insert_alpn(node, alpn);
    }
    if url.skip_cert_verify() {
        insert_boolean(node, "skip-cert-verify", true);
    }
}

// ---------------------------------------------------------------------------
// VLESS / VMess / Trojan
// ---------------------------------------------------------------------------

fn parse_vless(raw: &str) -> Result<ProxyNode, String> {
    let url = parse_link_url(raw, 443)?;
    let uuid = url
        .username
        .clone()
        .filter(|uuid| !uuid.is_empty())
        .ok_or_else(|| "vless link is missing a UUID".to_string())?;
    let network = normalize_network(url.query_value(&["type", "network"]).unwrap_or("tcp"))?;
    let security = url
        .query_value(&["security"])
        .unwrap_or("none")
        .to_ascii_lowercase();
    let name = url.display_name("vless");
    let mut node = proxy_node(&name, "vless", &url.host, url.port);
    insert_text(&mut node, "uuid", &uuid);
    insert_boolean(
        &mut node,
        "tls",
        matches!(security.as_str(), "tls" | "reality" | "xtls"),
    );
    insert_text(&mut node, "network", &network);
    insert_optional_text(&mut node, "flow", url.query_value(&["flow"]));
    insert_optional_text(
        &mut node,
        "servername",
        url.query_value(&["sni", "servername", "peer"]),
    );
    insert_optional_text(
        &mut node,
        "client-fingerprint",
        url.query_value(&["fp", "fingerprint"]),
    );
    if let Some(alpn) = url.query_value(&["alpn"]) {
        insert_alpn(&mut node, alpn);
    }
    if url.skip_cert_verify() {
        insert_boolean(&mut node, "skip-cert-verify", true);
    }
    let path = url.query_value(&["path"]);
    let host = url.query_value(&["host"]);
    let service_name = url.query_value(&["servicename", "service-name", "service_name"]);
    apply_transport(&mut node, &network, path, host, service_name);
    if security == "reality" {
        let mut reality = Mapping::new();
        if let Some(public_key) = url.query_value(&["pbk", "public-key", "publickey"]) {
            reality.insert(string("public-key"), string(public_key));
        }
        if let Some(short_id) = url.query_value(&["sid", "short-id", "shortid"]) {
            reality.insert(string("short-id"), string(short_id));
        }
        node.insert("reality-opts".to_string(), Value::Mapping(reality));
        insert_boolean(&mut node, "tls", true);
    }
    Ok(node)
}

fn json_text(object: &serde_json::Map<String, serde_json::Value>, key: &str) -> Option<String> {
    match object.get(key)? {
        serde_json::Value::String(text) => Some(text.clone()),
        serde_json::Value::Number(number) => Some(number.to_string()),
        serde_json::Value::Bool(flag) => Some(flag.to_string()),
        _ => None,
    }
}

fn json_number(object: &serde_json::Map<String, serde_json::Value>, key: &str) -> Option<u64> {
    match object.get(key)? {
        serde_json::Value::Number(number) => number
            .as_u64()
            .or_else(|| number.as_i64().and_then(|value| u64::try_from(value).ok())),
        serde_json::Value::String(text) => text.trim().parse().ok(),
        _ => None,
    }
}

fn parse_vmess(rest: &str) -> Result<ProxyNode, String> {
    let rest = rest.trim();
    let (payload, fragment) = split_fragment(rest);
    let decoded = decode_base64_text(payload)
        .or_else(|| decode_base64_text(rest))
        .ok_or_else(|| "vmess payload is not valid base64".to_string())?;
    let value: serde_json::Value = serde_json::from_str(decoded.trim())
        .map_err(|_| "vmess payload is not valid JSON".to_string())?;
    let object = value
        .as_object()
        .ok_or_else(|| "vmess payload must be a JSON object".to_string())?;
    let server = json_text(object, "add")
        .map(|server| server.trim().to_string())
        .filter(|server| !server.is_empty())
        .ok_or_else(|| "vmess payload is missing the server address".to_string())?;
    let port_value = json_number(object, "port")
        .ok_or_else(|| "vmess payload is missing the port".to_string())?;
    let port = u16::try_from(port_value)
        .ok()
        .filter(|port| *port > 0)
        .ok_or_else(|| "vmess payload has an invalid port".to_string())?;
    let uuid = json_text(object, "id")
        .map(|uuid| uuid.trim().to_string())
        .filter(|uuid| !uuid.is_empty())
        .ok_or_else(|| "vmess payload is missing the UUID".to_string())?;
    let name = json_text(object, "ps")
        .filter(|name| !name.trim().is_empty())
        .or(fragment)
        .unwrap_or_else(|| format!("vmess-{server}"));
    let mut node = proxy_node(name.trim(), "vmess", &server, port);
    insert_text(&mut node, "uuid", &uuid);
    if let Some(alter_id) = json_number(object, "aid") {
        insert_number(&mut node, "alterId", alter_id);
    }
    let cipher = json_text(object, "scy")
        .map(|cipher| cipher.trim().to_string())
        .filter(|cipher| !cipher.is_empty())
        .unwrap_or_else(|| "auto".to_string());
    insert_text(&mut node, "cipher", &cipher);
    let tls_enabled = json_text(object, "tls")
        .map(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "tls" | "reality" | "true" | "1"
            )
        })
        .unwrap_or(false);
    if tls_enabled {
        insert_boolean(&mut node, "tls", true);
    }
    insert_optional_text(&mut node, "servername", json_text(object, "sni").as_deref());
    insert_optional_text(
        &mut node,
        "client-fingerprint",
        json_text(object, "fp").as_deref(),
    );
    if let Some(alpn) = json_text(object, "alpn") {
        insert_alpn(&mut node, &alpn);
    }
    if object
        .get("allowInsecure")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false)
        || json_text(object, "allowInsecure")
            .is_some_and(|value| matches!(value.as_str(), "true" | "1"))
    {
        insert_boolean(&mut node, "skip-cert-verify", true);
    }
    let network =
        normalize_network(&json_text(object, "net").unwrap_or_else(|| "tcp".to_string()))?;
    let header_type = json_text(object, "type").unwrap_or_default();
    let network = if network == "tcp" && header_type.eq_ignore_ascii_case("http") {
        "http".to_string()
    } else {
        network
    };
    insert_text(&mut node, "network", &network);
    let path = option_non_empty(json_text(object, "path").unwrap_or_default());
    let host = option_non_empty(json_text(object, "host").unwrap_or_default());
    apply_transport(
        &mut node,
        &network,
        path.as_deref(),
        host.as_deref(),
        path.as_deref(),
    );
    Ok(node)
}

fn parse_trojan(raw: &str) -> Result<ProxyNode, String> {
    let url = parse_link_url(raw, 443)?;
    let password = match (&url.username, &url.password) {
        (Some(username), Some(password)) => format!("{username}:{password}"),
        (Some(username), None) => username.clone(),
        _ => return Err("trojan link is missing a password".to_string()),
    };
    let network = normalize_network(url.query_value(&["type", "network"]).unwrap_or("tcp"))?;
    let name = url.display_name("trojan");
    let mut node = proxy_node(&name, "trojan", &url.host, url.port);
    insert_text(&mut node, "password", &password);
    apply_common_options(&mut node, &url, "sni");
    insert_optional_text(
        &mut node,
        "client-fingerprint",
        url.query_value(&["fp", "fingerprint"]),
    );
    if network != "tcp" {
        insert_text(&mut node, "network", &network);
        let path = url.query_value(&["path"]);
        let host = url.query_value(&["host"]);
        let service_name = url.query_value(&["servicename", "service-name", "service_name"]);
        apply_transport(&mut node, &network, path, host, service_name);
    }
    Ok(node)
}

// ---------------------------------------------------------------------------
// Shadowsocks / SSR
// ---------------------------------------------------------------------------

const SS_CIPHERS: &[&str] = &[
    "aes-128-gcm",
    "aes-192-gcm",
    "aes-256-gcm",
    "aes-128-ccm",
    "aes-256-ccm",
    "aes-128-gcm-siv",
    "aes-256-gcm-siv",
    "chacha20-ietf-poly1305",
    "xchacha20-ietf-poly1305",
    "chacha8-ietf-poly1305",
    "2022-blake3-aes-128-gcm",
    "2022-blake3-aes-256-gcm",
    "2022-blake3-chacha20-poly1305",
    "aes-128-cfb",
    "aes-192-cfb",
    "aes-256-cfb",
    "aes-128-ctr",
    "aes-192-ctr",
    "aes-256-ctr",
    "rc4-md5",
    "chacha20-ietf",
    "chacha20",
    "xchacha20",
    "des-cfb",
    "bf-cfb",
    "cast5-cfb",
    "table",
    "none",
    "plain",
];

fn validate_cipher(cipher: &str) -> Result<String, String> {
    let normalized = cipher.trim().to_ascii_lowercase();
    if normalized.is_empty() {
        return Err("link is missing a cipher".to_string());
    }
    if SS_CIPHERS.contains(&normalized.as_str()) {
        Ok(normalized)
    } else {
        Err("unsupported cipher".to_string())
    }
}

fn decode_ss_userinfo(userinfo: &str) -> Result<String, String> {
    let decoded = decode_component(userinfo);
    if decoded.contains(':') {
        Ok(decoded)
    } else {
        decode_base64_text(userinfo).ok_or_else(|| "ss userinfo is not valid base64".to_string())
    }
}

fn split_host_port(raw: &str, default_port: u16, context: &str) -> Result<(String, u16), String> {
    let raw = raw.trim().trim_end_matches('/');
    if raw.is_empty() {
        return Err(format!("{context} is missing a host"));
    }
    let (host, port) = if let Some(stripped) = raw.strip_prefix('[') {
        let (host, rest) = stripped
            .split_once(']')
            .ok_or_else(|| format!("{context} has an invalid IPv6 host"))?;
        let port = match rest.strip_prefix(':') {
            Some(port) => port
                .trim()
                .parse::<u16>()
                .map_err(|_| format!("{context} has an invalid port"))?,
            None => default_port,
        };
        (host.to_string(), port)
    } else if let Some((host, port)) = raw.rsplit_once(':') {
        if host.contains(':') {
            (raw.to_string(), default_port)
        } else {
            let port = port
                .trim()
                .parse::<u16>()
                .map_err(|_| format!("{context} has an invalid port"))?;
            (host.to_string(), port)
        }
    } else {
        (raw.to_string(), default_port)
    };
    if host.trim().is_empty() {
        return Err(format!("{context} is missing a host"));
    }
    if port == 0 {
        return Err(format!("{context} has an invalid port"));
    }
    Ok((host.trim().to_string(), port))
}

pub(crate) fn apply_ss_plugin(query: &BTreeMap<String, String>, node: &mut ProxyNode) {
    let Some(raw) = query
        .get("plugin")
        .filter(|plugin| !plugin.trim().is_empty())
    else {
        return;
    };
    let mut segments = raw.split(';');
    let plugin_name = segments.next().unwrap_or_default().trim();
    if plugin_name.is_empty() {
        return;
    }
    let mut options = Mapping::new();
    for segment in segments {
        let (key, value) = match segment.split_once('=') {
            Some((key, value)) => (key.trim(), value.trim()),
            None => (segment.trim(), ""),
        };
        if key.is_empty() {
            continue;
        }
        let value = match key {
            "tls" | "mux" => boolean(matches!(
                value.to_ascii_lowercase().as_str(),
                "1" | "true" | "yes"
            )),
            _ => string(value),
        };
        options.insert(string(key), value);
    }
    match plugin_name.to_ascii_lowercase().as_str() {
        "obfs-local" | "simple-obfs" | "obfs" => {
            let mode = options
                .get(string("obfs"))
                .and_then(Value::as_str)
                .unwrap_or("http");
            let mut plugin_options = Mapping::new();
            plugin_options.insert(string("mode"), string(mode));
            if let Some(host) = options.get(string("obfs-host")).and_then(Value::as_str) {
                plugin_options.insert(string("host"), string(host));
            }
            node.insert("plugin".to_string(), string("obfs"));
            node.insert("plugin-opts".to_string(), Value::Mapping(plugin_options));
        }
        "v2ray-plugin" => {
            node.insert("plugin".to_string(), string("v2ray-plugin"));
            node.insert("plugin-opts".to_string(), Value::Mapping(options));
        }
        other => {
            node.insert("plugin".to_string(), string(other));
            node.insert("plugin-opts".to_string(), Value::Mapping(options));
        }
    }
}

fn parse_ss(rest: &str) -> Result<ProxyNode, String> {
    let (body, fragment) = split_fragment(rest.trim());
    let (body, query) = split_query(body);
    let (cipher, password, host, port) = if let Some((userinfo, host_port)) = body.rsplit_once('@')
    {
        let credentials = decode_ss_userinfo(userinfo)?;
        let (cipher, password) = credentials
            .split_once(':')
            .ok_or_else(|| "ss credentials must have the form method:password".to_string())?;
        let (host, port) = split_host_port(host_port, 8388, "ss link")?;
        (cipher.to_string(), password.to_string(), host, port)
    } else {
        let decoded =
            decode_base64_text(body).ok_or_else(|| "ss payload is not valid base64".to_string())?;
        let (credentials, host_port) = decoded
            .rsplit_once('@')
            .ok_or_else(|| "ss payload is missing the host part".to_string())?;
        let (cipher, password) = credentials
            .split_once(':')
            .ok_or_else(|| "ss credentials must have the form method:password".to_string())?;
        let (host, port) = split_host_port(host_port, 8388, "ss link")?;
        (cipher.to_string(), password.to_string(), host, port)
    };
    let cipher = validate_cipher(&cipher)?;
    let name = fragment
        .filter(|fragment| !fragment.trim().is_empty())
        .unwrap_or_else(|| format!("ss-{host}"));
    let mut node = proxy_node(name.trim(), "ss", &host, port);
    insert_text(&mut node, "cipher", &cipher);
    insert_text(&mut node, "password", &password);
    apply_ss_plugin(&query, &mut node);
    Ok(node)
}

fn parse_ssr(rest: &str) -> Result<ProxyNode, String> {
    let decoded = decode_base64_text(rest.trim())
        .ok_or_else(|| "ssr payload is not valid base64".to_string())?;
    let (main, query_raw) = match decoded.split_once('?') {
        Some((main, query)) => (main, Some(query)),
        None => (decoded.as_str(), None),
    };
    let main = main.trim().trim_end_matches('/');
    // Split from the credential end so IPv6 hosts can contain colons.
    let mut fields: Vec<&str> = main.rsplitn(6, ':').collect();
    fields.reverse();
    if fields.len() != 6 {
        return Err("ssr payload must contain host:port:protocol:method:obfs:password".to_string());
    }
    let host = fields[0].trim().trim_matches(['[', ']']);
    let port: u16 = fields[1]
        .trim()
        .parse()
        .map_err(|_| "ssr payload has an invalid port".to_string())?;
    if port == 0 {
        return Err("ssr payload has an invalid port".to_string());
    }
    let protocol = fields[2].trim();
    let cipher = fields[3].trim();
    let obfs = fields[4].trim();
    if host.is_empty() {
        return Err("ssr payload is missing a host".to_string());
    }
    if protocol.is_empty() || obfs.is_empty() {
        return Err("ssr payload is missing protocol or obfs".to_string());
    }
    let cipher = validate_cipher(cipher)?;
    let password = decode_base64_text(fields[5].trim())
        .ok_or_else(|| "ssr password is not valid base64url".to_string())?;
    let mut name = None;
    let mut obfs_param = None;
    let mut protocol_param = None;
    if let Some(query) = query_raw {
        for pair in query.split('&') {
            let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
            let decoded_value = decode_base64_text(value).unwrap_or_default();
            match decode_component(key).trim().to_ascii_lowercase().as_str() {
                "remarks" => name = Some(decoded_value),
                "obfsparam" => obfs_param = Some(decoded_value),
                "protoparam" => protocol_param = Some(decoded_value),
                _ => {}
            }
        }
    }
    let name = name
        .filter(|name| !name.trim().is_empty())
        .unwrap_or_else(|| format!("ssr-{host}"));
    let mut node = proxy_node(name.trim(), "ssr", host, port);
    insert_text(&mut node, "cipher", &cipher);
    insert_text(&mut node, "password", &password);
    insert_text(&mut node, "protocol", protocol);
    insert_optional_text(&mut node, "protocol-param", protocol_param.as_deref());
    insert_text(&mut node, "obfs", obfs);
    insert_optional_text(&mut node, "obfs-param", obfs_param.as_deref());
    Ok(node)
}

// ---------------------------------------------------------------------------
// Hysteria2 / TUIC / AnyTLS / SOCKS5 / HTTP
// ---------------------------------------------------------------------------

fn parse_hysteria2(raw: &str) -> Result<ProxyNode, String> {
    let url = parse_link_url(raw, 443)?;
    let password = match (&url.username, &url.password) {
        (Some(username), Some(password)) => format!("{username}:{password}"),
        (None, Some(password)) => password.clone(),
        (Some(password), None) => password.clone(),
        _ => return Err("hysteria2 link is missing a password".to_string()),
    };
    if password.is_empty() {
        return Err("hysteria2 link is missing a password".to_string());
    }
    let name = url.display_name("hysteria2");
    let mut node = proxy_node(&name, "hysteria2", &url.host, url.port);
    insert_text(&mut node, "password", &password);
    insert_optional_text(
        &mut node,
        "sni",
        url.query_value(&["sni", "servername", "peer"]),
    );
    if url.skip_cert_verify() {
        insert_boolean(&mut node, "skip-cert-verify", true);
    }
    insert_optional_text(&mut node, "obfs", url.query_value(&["obfs"]));
    insert_optional_text(
        &mut node,
        "obfs-password",
        url.query_value(&["obfs-password", "obfs_password", "obfspassword"]),
    );
    insert_optional_text(
        &mut node,
        "up",
        url.query_value(&["up", "upmbps", "up-mbps"]),
    );
    insert_optional_text(
        &mut node,
        "down",
        url.query_value(&["down", "downmbps", "down-mbps"]),
    );
    if let Some(alpn) = url.query_value(&["alpn"]) {
        insert_alpn(&mut node, alpn);
    }
    Ok(node)
}

fn parse_tuic(raw: &str) -> Result<ProxyNode, String> {
    let url = parse_link_url(raw, 443)?;
    let uuid = url
        .username
        .clone()
        .filter(|uuid| !uuid.is_empty())
        .ok_or_else(|| "tuic link is missing a UUID".to_string())?;
    let password = url
        .password
        .clone()
        .filter(|password| !password.is_empty())
        .ok_or_else(|| "tuic link is missing a password".to_string())?;
    let name = url.display_name("tuic");
    let mut node = proxy_node(&name, "tuic", &url.host, url.port);
    insert_text(&mut node, "uuid", &uuid);
    insert_text(&mut node, "password", &password);
    insert_optional_text(
        &mut node,
        "sni",
        url.query_value(&["sni", "servername", "peer"]),
    );
    if let Some(alpn) = url.query_value(&["alpn"]) {
        insert_alpn(&mut node, alpn);
    }
    insert_optional_text(
        &mut node,
        "congestion-controller",
        url.query_value(&["congestion_control", "congestion-controller", "congestion"]),
    );
    insert_optional_text(
        &mut node,
        "udp-relay-mode",
        url.query_value(&["udp_relay_mode", "udp-relay-mode"]),
    );
    if url
        .query_value(&["reduce_rtt", "reduce-rtt"])
        .map(|value| matches!(value.to_ascii_lowercase().as_str(), "1" | "true" | "yes"))
        .unwrap_or(false)
    {
        insert_boolean(&mut node, "reduce-rtt", true);
    }
    if url.skip_cert_verify() {
        insert_boolean(&mut node, "skip-cert-verify", true);
    }
    Ok(node)
}

fn parse_anytls(raw: &str) -> Result<ProxyNode, String> {
    let url = parse_link_url(raw, 443)?;
    let password = match (&url.username, &url.password) {
        (Some(username), Some(password)) => format!("{username}:{password}"),
        (Some(username), None) => username.clone(),
        _ => return Err("anytls link is missing a password".to_string()),
    };
    let name = url.display_name("anytls");
    let mut node = proxy_node(&name, "anytls", &url.host, url.port);
    insert_text(&mut node, "password", &password);
    insert_optional_text(
        &mut node,
        "sni",
        url.query_value(&["sni", "servername", "peer"]),
    );
    if url.skip_cert_verify() {
        insert_boolean(&mut node, "skip-cert-verify", true);
    }
    Ok(node)
}

fn parse_socks5(raw: &str) -> Result<ProxyNode, String> {
    let url = parse_link_url(raw, 1080)?;
    let name = url.display_name("socks5");
    let mut node = proxy_node(&name, "socks5", &url.host, url.port);
    insert_optional_text(&mut node, "username", url.username.as_deref());
    insert_optional_text(&mut node, "password", url.password.as_deref());
    Ok(node)
}

fn parse_http(raw: &str) -> Result<ProxyNode, String> {
    let url = parse_link_url(raw, 443)?;
    let name = url.display_name("http");
    let mut node = proxy_node(&name, "http", &url.host, url.port);
    insert_optional_text(&mut node, "username", url.username.as_deref());
    insert_optional_text(&mut node, "password", url.password.as_deref());
    insert_boolean(&mut node, "tls", url.scheme == "https");
    apply_common_options(&mut node, &url, "sni");
    Ok(node)
}

// ---------------------------------------------------------------------------
// Link rendering
// ---------------------------------------------------------------------------

fn format_host(host: &str) -> String {
    if host.contains(':') && !host.starts_with('[') {
        format!("[{host}]")
    } else {
        host.to_string()
    }
}

fn encode_authority(value: &str) -> String {
    encode_component(value)
}

fn format_link(
    scheme: &str,
    userinfo: Option<&str>,
    host: &str,
    port: u16,
    query: &[(&str, String)],
    name: &str,
) -> String {
    let mut link = String::from(scheme);
    link.push_str("://");
    if let Some(userinfo) = userinfo {
        link.push_str(userinfo);
        link.push('@');
    }
    link.push_str(&format_host(host));
    link.push(':');
    link.push_str(&port.to_string());
    if !query.is_empty() {
        let pairs: Vec<String> = query
            .iter()
            .map(|(key, value)| format!("{key}={}", encode_component(value)))
            .collect();
        link.push('?');
        link.push_str(&pairs.join("&"));
    }
    link.push('#');
    link.push_str(&encode_component(name));
    link
}

fn push_optional(query: &mut Vec<(&'static str, String)>, key: &'static str, value: Option<&str>) {
    if let Some(value) = value {
        let value = value.trim();
        if !value.is_empty() {
            query.push((key, value.to_string()));
        }
    }
}

fn push_joined(query: &mut Vec<(&'static str, String)>, key: &'static str, values: Vec<String>) {
    if !values.is_empty() {
        query.push((key, values.join(",")));
    }
}

fn require_text<'a>(node: &'a ProxyNode, key: &str) -> Result<&'a str, String> {
    nodes::node_str(node, key)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("node is missing `{key}`"))
}

fn push_transport_query(node: &ProxyNode, network: &str, query: &mut Vec<(&'static str, String)>) {
    match network {
        "ws" | "http" | "h2" | "httpupgrade" => {
            query.push(("type", network.to_string()));
            let path = nodes::transport_path(node);
            push_optional(query, "path", path.as_deref());
            let host = nodes::transport_host(node);
            push_optional(query, "host", host.as_deref());
        }
        "grpc" => {
            query.push(("type", "grpc".to_string()));
            let service = nodes::grpc_service(node);
            push_optional(query, "serviceName", service.as_deref());
        }
        _ => {}
    }
}

fn plugin_parameter(node: &ProxyNode) -> Option<String> {
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
            let mut parameter = format!("obfs-local;obfs={mode}");
            if let Some(host) = options.and_then(|options| nodes::mapping_str(options, "host")) {
                parameter.push_str(&format!(";obfs-host={host}"));
            }
            Some(parameter)
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
                        parts.push(format!("{key}=true"));
                    }
                }
            }
            let mut parameter = String::from("v2ray-plugin");
            for part in parts {
                parameter.push(';');
                parameter.push_str(&part);
            }
            Some(parameter)
        }
        plugin_name => {
            let mut parameter = plugin_name.to_string();
            if let Some(options) = options {
                for (key, value) in options {
                    if let (Some(key), Some(value)) = (key.as_str(), value.as_str()) {
                        parameter.push(';');
                        parameter.push_str(&format!("{key}={value}"));
                    }
                }
            }
            Some(parameter)
        }
    }
}

fn render_vless(node: &ProxyNode, name: &str, server: &str, port: u16) -> Result<String, String> {
    let uuid = require_text(node, "uuid")?;
    let network = nodes::node_network(node);
    let tls = nodes::node_bool(node, "tls").unwrap_or(false);
    let reality = nodes::node_mapping(node, "reality-opts");
    let mut query: Vec<(&'static str, String)> = Vec::new();
    match network.as_str() {
        "ws" => query.push(("type", "ws".to_string())),
        "grpc" => query.push(("type", "grpc".to_string())),
        "http" => query.push(("type", "http".to_string())),
        "h2" => query.push(("type", "h2".to_string())),
        "httpupgrade" => query.push(("type", "httpupgrade".to_string())),
        _ => {}
    }
    let security = if reality.is_some() {
        "reality"
    } else if tls {
        "tls"
    } else {
        "none"
    };
    query.push(("security", security.to_string()));
    push_optional(
        &mut query,
        "sni",
        nodes::node_str(node, "servername").or_else(|| nodes::node_str(node, "sni")),
    );
    push_joined(&mut query, "alpn", nodes::node_strings(node, "alpn"));
    push_optional(
        &mut query,
        "fp",
        nodes::node_str(node, "client-fingerprint"),
    );
    push_optional(&mut query, "flow", nodes::node_str(node, "flow"));
    if let Some(reality) = reality {
        push_optional(&mut query, "pbk", nodes::mapping_str(reality, "public-key"));
        push_optional(&mut query, "sid", nodes::mapping_str(reality, "short-id"));
    }
    if nodes::node_bool(node, "skip-cert-verify").unwrap_or(false) {
        query.push(("allowInsecure", "1".to_string()));
    }
    match network.as_str() {
        "ws" | "http" | "h2" | "httpupgrade" => {
            let path = nodes::transport_path(node);
            push_optional(&mut query, "path", path.as_deref());
            let host = nodes::transport_host(node);
            push_optional(&mut query, "host", host.as_deref());
        }
        "grpc" => {
            let service = nodes::grpc_service(node);
            push_optional(&mut query, "serviceName", service.as_deref());
        }
        _ => {}
    }
    Ok(format_link(
        "vless",
        Some(&encode_authority(uuid)),
        server,
        port,
        &query,
        name,
    ))
}

fn render_vmess(node: &ProxyNode, name: &str, server: &str, port: u16) -> Result<String, String> {
    let uuid = require_text(node, "uuid")?;
    let network = nodes::node_network(node);
    let tls = nodes::node_bool(node, "tls").unwrap_or(false);
    let (net, header_type, host, path) = match network.as_str() {
        "ws" | "httpupgrade" => (
            network.clone(),
            String::new(),
            nodes::transport_host(node).unwrap_or_default(),
            nodes::transport_path(node).unwrap_or_else(|| "/".to_string()),
        ),
        "grpc" => (
            "grpc".to_string(),
            String::new(),
            String::new(),
            nodes::grpc_service(node).unwrap_or_default(),
        ),
        "http" => (
            "tcp".to_string(),
            "http".to_string(),
            nodes::transport_host(node).unwrap_or_default(),
            nodes::transport_path(node).unwrap_or_else(|| "/".to_string()),
        ),
        "h2" => (
            "h2".to_string(),
            String::new(),
            nodes::transport_host(node).unwrap_or_default(),
            nodes::transport_path(node).unwrap_or_default(),
        ),
        _ => (
            "tcp".to_string(),
            String::new(),
            String::new(),
            String::new(),
        ),
    };
    let payload = serde_json::json!({
        "v": "2",
        "ps": name,
        "add": server,
        "port": port,
        "id": uuid,
        "aid": nodes::node_number(node, "alterId").unwrap_or(0),
        "scy": nodes::node_str(node, "cipher").unwrap_or("auto"),
        "net": net,
        "type": header_type,
        "host": host,
        "path": path,
        "tls": if tls { "tls" } else { "" },
        "sni": nodes::node_str(node, "servername").unwrap_or_default(),
        "alpn": nodes::node_strings(node, "alpn").join(","),
        "fp": nodes::node_str(node, "client-fingerprint").unwrap_or_default(),
        "allowInsecure": nodes::node_bool(node, "skip-cert-verify").unwrap_or(false),
    });
    Ok(format!(
        "vmess://{}",
        STANDARD.encode(payload.to_string().as_bytes())
    ))
}

fn render_trojan(node: &ProxyNode, name: &str, server: &str, port: u16) -> Result<String, String> {
    let password = require_text(node, "password")?;
    let network = nodes::node_network(node);
    let mut query: Vec<(&'static str, String)> = Vec::new();
    push_optional(
        &mut query,
        "sni",
        nodes::node_str(node, "sni").or_else(|| nodes::node_str(node, "servername")),
    );
    push_joined(&mut query, "alpn", nodes::node_strings(node, "alpn"));
    push_optional(
        &mut query,
        "fp",
        nodes::node_str(node, "client-fingerprint"),
    );
    if nodes::node_bool(node, "skip-cert-verify").unwrap_or(false) {
        query.push(("allowInsecure", "1".to_string()));
    }
    push_transport_query(node, &network, &mut query);
    Ok(format_link(
        "trojan",
        Some(&encode_authority(password)),
        server,
        port,
        &query,
        name,
    ))
}

fn render_ss(node: &ProxyNode, name: &str, server: &str, port: u16) -> Result<String, String> {
    let cipher = require_text(node, "cipher")?;
    let password = require_text(node, "password")?;
    let credentials = format!("{cipher}:{password}");
    let userinfo = URL_SAFE_NO_PAD.encode(credentials.as_bytes());
    let mut query: Vec<(&'static str, String)> = Vec::new();
    if let Some(plugin) = plugin_parameter(node) {
        query.push(("plugin", plugin));
    }
    Ok(format_link(
        "ss",
        Some(&userinfo),
        server,
        port,
        &query,
        name,
    ))
}

fn render_ssr(node: &ProxyNode, name: &str, server: &str, port: u16) -> Result<String, String> {
    let cipher = require_text(node, "cipher")?;
    let password = require_text(node, "password")?;
    let protocol = nodes::node_str(node, "protocol").unwrap_or("origin").trim();
    let obfs = nodes::node_str(node, "obfs").unwrap_or("plain").trim();
    let encoded_password = URL_SAFE_NO_PAD.encode(password.as_bytes());
    let mut parameters: Vec<String> = vec![format!(
        "remarks={}",
        URL_SAFE_NO_PAD.encode(name.as_bytes())
    )];
    if let Some(obfs_param) = nodes::node_str(node, "obfs-param").filter(|value| !value.is_empty())
    {
        parameters.push(format!(
            "obfsparam={}",
            URL_SAFE_NO_PAD.encode(obfs_param.as_bytes())
        ));
    }
    if let Some(protocol_param) =
        nodes::node_str(node, "protocol-param").filter(|value| !value.is_empty())
    {
        parameters.push(format!(
            "protoparam={}",
            URL_SAFE_NO_PAD.encode(protocol_param.as_bytes())
        ));
    }
    let payload = format!(
        "{server}:{port}:{protocol}:{cipher}:{obfs}:{encoded_password}/?{}",
        parameters.join("&")
    );
    Ok(format!("ssr://{}", STANDARD.encode(payload.as_bytes())))
}

fn render_hysteria2(
    node: &ProxyNode,
    name: &str,
    server: &str,
    port: u16,
) -> Result<String, String> {
    let password = require_text(node, "password")?;
    let mut query: Vec<(&'static str, String)> = Vec::new();
    push_optional(
        &mut query,
        "sni",
        nodes::node_str(node, "sni").or_else(|| nodes::node_str(node, "servername")),
    );
    if nodes::node_bool(node, "skip-cert-verify").unwrap_or(false) {
        query.push(("insecure", "1".to_string()));
    }
    push_optional(&mut query, "obfs", nodes::node_str(node, "obfs"));
    push_optional(
        &mut query,
        "obfs-password",
        nodes::node_str(node, "obfs-password"),
    );
    push_optional(&mut query, "up", nodes::node_str(node, "up"));
    push_optional(&mut query, "down", nodes::node_str(node, "down"));
    push_joined(&mut query, "alpn", nodes::node_strings(node, "alpn"));
    Ok(format_link(
        "hysteria2",
        Some(&encode_authority(password)),
        server,
        port,
        &query,
        name,
    ))
}

fn render_tuic(node: &ProxyNode, name: &str, server: &str, port: u16) -> Result<String, String> {
    let uuid = require_text(node, "uuid")?;
    let password = require_text(node, "password")?;
    let userinfo = format!("{}:{}", encode_authority(uuid), encode_authority(password));
    let mut query: Vec<(&'static str, String)> = Vec::new();
    push_optional(
        &mut query,
        "sni",
        nodes::node_str(node, "sni").or_else(|| nodes::node_str(node, "servername")),
    );
    push_joined(&mut query, "alpn", nodes::node_strings(node, "alpn"));
    push_optional(
        &mut query,
        "congestion_control",
        nodes::node_str(node, "congestion-controller"),
    );
    push_optional(
        &mut query,
        "udp_relay_mode",
        nodes::node_str(node, "udp-relay-mode"),
    );
    if nodes::node_bool(node, "reduce-rtt").unwrap_or(false) {
        query.push(("reduce_rtt", "1".to_string()));
    }
    if nodes::node_bool(node, "skip-cert-verify").unwrap_or(false) {
        query.push(("allow_insecure", "1".to_string()));
    }
    Ok(format_link(
        "tuic",
        Some(&userinfo),
        server,
        port,
        &query,
        name,
    ))
}

fn render_anytls(node: &ProxyNode, name: &str, server: &str, port: u16) -> Result<String, String> {
    let password = require_text(node, "password")?;
    let mut query: Vec<(&'static str, String)> = Vec::new();
    push_optional(
        &mut query,
        "sni",
        nodes::node_str(node, "sni").or_else(|| nodes::node_str(node, "servername")),
    );
    if nodes::node_bool(node, "skip-cert-verify").unwrap_or(false) {
        query.push(("insecure", "1".to_string()));
    }
    Ok(format_link(
        "anytls",
        Some(&encode_authority(password)),
        server,
        port,
        &query,
        name,
    ))
}

fn render_socks5(node: &ProxyNode, name: &str, server: &str, port: u16) -> Result<String, String> {
    let username = nodes::node_str(node, "username")
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let password = nodes::node_str(node, "password")
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let userinfo = match (username, password) {
        (Some(username), Some(password)) => Some(format!(
            "{}:{}",
            encode_authority(username),
            encode_authority(password)
        )),
        (Some(username), None) => Some(encode_authority(username)),
        (None, Some(password)) => Some(format!(":{}", encode_authority(password))),
        (None, None) => None,
    };
    Ok(format_link(
        "socks5",
        userinfo.as_deref(),
        server,
        port,
        &[],
        name,
    ))
}

fn render_http(node: &ProxyNode, name: &str, server: &str, port: u16) -> Result<String, String> {
    let tls = nodes::node_bool(node, "tls").unwrap_or(false);
    let scheme = if tls { "https" } else { "http" };
    let username = nodes::node_str(node, "username")
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let password = nodes::node_str(node, "password")
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let userinfo = match (username, password) {
        (Some(username), Some(password)) => Some(format!(
            "{}:{}",
            encode_authority(username),
            encode_authority(password)
        )),
        (Some(username), None) => Some(encode_authority(username)),
        (None, Some(password)) => Some(format!(":{}", encode_authority(password))),
        (None, None) => None,
    };
    let mut query = Vec::new();
    push_optional(
        &mut query,
        "sni",
        nodes::node_str(node, "sni").or_else(|| nodes::node_str(node, "servername")),
    );
    if nodes::node_bool(node, "skip-cert-verify").unwrap_or(false) {
        query.push(("insecure", "1".to_string()));
    }
    Ok(format_link(
        scheme,
        userinfo.as_deref(),
        server,
        port,
        &query,
        name,
    ))
}

fn render_link_inner(node: &ProxyNode) -> Result<String, String> {
    let kind = nodes::node_type_lower(node).ok_or_else(|| "node is missing a type".to_string())?;
    let server = nodes::node_server(node).ok_or_else(|| "node is missing a server".to_string())?;
    let port = nodes::node_u16(node, "port")
        .filter(|port| *port > 0)
        .ok_or_else(|| "node has an invalid port".to_string())?;
    let name = nodes::node_name(node).unwrap_or("node");
    match kind.as_str() {
        "vless" => render_vless(node, name, server, port),
        "vmess" => render_vmess(node, name, server, port),
        "trojan" => render_trojan(node, name, server, port),
        "ss" => render_ss(node, name, server, port),
        "ssr" => render_ssr(node, name, server, port),
        "hysteria2" | "hy2" => render_hysteria2(node, name, server, port),
        "tuic" => render_tuic(node, name, server, port),
        "anytls" => render_anytls(node, name, server, port),
        "socks5" | "socks" => render_socks5(node, name, server, port),
        "http" | "https" => render_http(node, name, server, port),
        _ => Err("unsupported protocol".to_string()),
    }
}

/// Renders a proxy node back into a share link.
///
/// The returned error does not carry a link index: it is set to `0` because
/// rendering is not tied to an input subscription line.
pub fn render_link(node: &ProxyNode) -> Result<String, LinkError> {
    render_link_inner(node).map_err(|reason| LinkError::new(0, reason))
}

#[cfg(test)]
mod tests;
