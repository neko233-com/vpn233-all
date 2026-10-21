//! Line oriented configuration rendering for Surge, Quantumult X, Loon and
//! Surfboard.
//!
//! Every renderer skips protocols the target client cannot import and escapes
//! node names so a single hostile name cannot break the whole profile.

use crate::nodes;
use crate::ProxyNode;

const LOCAL_DOMAIN_RULE: &str = "DOMAIN-SUFFIX,local,DIRECT";

/// Replaces characters that would break a comma separated client config line.
fn line_value(raw: &str) -> String {
    raw.chars()
        .filter(|character| !character.is_control())
        .map(|character| if character == ',' { '_' } else { character })
        .collect()
}

/// Quotes a value for Loon style `key = "value"` positions.
fn quoted(raw: &str) -> String {
    let escaped = line_value(raw).replace('\\', "\\\\").replace('"', "\\\"");
    format!("\"{escaped}\"")
}

fn group_members(names: &[String]) -> String {
    let mut members = names.to_vec();
    members.push(nodes::DIRECT_POLICY.to_string());
    members.join(", ")
}

fn append_rule_section(body: &mut String) {
    body.push_str("[Rule]\n");
    body.push_str(LOCAL_DOMAIN_RULE);
    body.push('\n');
    for cidr in nodes::PRIVATE_CIDRS {
        body.push_str(&format!("IP-CIDR,{cidr},DIRECT,no-resolve\n"));
    }
    for cidr in nodes::PRIVATE_IPV6_CIDRS {
        body.push_str(&format!("IP-CIDR6,{cidr},DIRECT,no-resolve\n"));
    }
    body.push_str(&format!("FINAL,{}\n", nodes::MAIN_GROUP));
}

// ---------------------------------------------------------------------------
// Surge / Surfboard (shared dialect)
// ---------------------------------------------------------------------------

/// Renders a Surge configuration for the given nodes.
pub fn render_surge_config(nodes: &[ProxyNode]) -> String {
    render_surge_profile(nodes, nodes::DEFAULT_PROFILE)
}

/// Renders a Surfboard configuration for the given nodes.
pub fn render_surfboard_config(nodes: &[ProxyNode]) -> String {
    render_surfboard_profile(nodes, nodes::DEFAULT_PROFILE)
}

pub(crate) fn render_surge_profile(nodes: &[ProxyNode], profile_name: &str) -> String {
    render_surge_like(nodes, profile_name, "Surge", false)
}

pub(crate) fn render_surfboard_profile(nodes: &[ProxyNode], profile_name: &str) -> String {
    render_surge_like(nodes, profile_name, "Surfboard", true)
}

fn render_surge_like(
    nodes: &[ProxyNode],
    profile_name: &str,
    brand: &str,
    allow_ssr: bool,
) -> String {
    let profile = nodes::sanitize_profile_name(profile_name);
    let mut body = String::new();
    body.push_str(&format!("# VPN233 {brand} profile: {profile}\n"));
    body.push_str("[General]\n");
    body.push_str("loglevel = notify\n");
    body.push_str("dns-server = system\n");
    body.push_str(
        "skip-proxy = 127.0.0.1, 192.168.0.0/16, 10.0.0.0/8, 172.16.0.0/12, localhost, *.local\n",
    );
    body.push_str("[Proxy]\n");
    let mut names: Vec<String> = Vec::new();
    for (name, node) in nodes::prepared_line_nodes(nodes, nodes::LINE_RESERVED) {
        if let Some(line) = surge_like_proxy_line(&name, &node, allow_ssr) {
            names.push(name);
            body.push_str(&line);
            body.push('\n');
        }
    }
    body.push_str("[Proxy Group]\n");
    body.push_str(&format!(
        "{} = select, {}\n",
        nodes::MAIN_GROUP,
        group_members(&names)
    ));
    append_rule_section(&mut body);
    body
}

fn surge_like_proxy_line(name: &str, node: &ProxyNode, allow_ssr: bool) -> Option<String> {
    let kind = nodes::node_type_lower(node)?;
    let server = nodes::node_server(node)?;
    let port = nodes::node_u16(node, "port")?;
    let mut line = format!("{name} = ");
    match kind.as_str() {
        "ss" => {
            let cipher = line_value(nodes::node_str(node, "cipher")?);
            let password = line_value(nodes::node_str(node, "password")?);
            line.push_str(&format!(
                "ss, {server}, {port}, encrypt-method={cipher}, password={password}, udp-relay=true"
            ));
            if let Some(plugin) = nodes::node_str(node, "plugin") {
                if plugin.eq_ignore_ascii_case("obfs") {
                    let mode = nodes::node_mapping(node, "plugin-opts")
                        .and_then(|options| nodes::mapping_str(options, "mode"))
                        .unwrap_or("http");
                    line.push_str(&format!(", obfs={}", line_value(mode)));
                    if let Some(host) = nodes::node_mapping(node, "plugin-opts")
                        .and_then(|options| nodes::mapping_str(options, "host"))
                    {
                        line.push_str(&format!(", obfs-host={}", line_value(host)));
                    }
                }
            }
        }
        "ssr" if allow_ssr => {
            let cipher = line_value(nodes::node_str(node, "cipher")?);
            let password = line_value(nodes::node_str(node, "password")?);
            let protocol = line_value(nodes::node_str(node, "protocol").unwrap_or("origin"));
            let obfs = line_value(nodes::node_str(node, "obfs").unwrap_or("plain"));
            line.push_str(&format!(
                "ssr, {server}, {port}, encrypt-method={cipher}, password={password}, protocol={protocol}, obfs={obfs}, udp-relay=true"
            ));
            if let Some(parameter) = nodes::node_str(node, "protocol-param") {
                line.push_str(&format!(", protocol-param={}", line_value(parameter)));
            }
            if let Some(parameter) = nodes::node_str(node, "obfs-param") {
                line.push_str(&format!(", obfs-param={}", line_value(parameter)));
            }
        }
        "vmess" => {
            let uuid = line_value(nodes::node_str(node, "uuid")?);
            line.push_str(&format!("vmess, {server}, {port}, username={uuid}"));
            if nodes::node_bool(node, "tls").unwrap_or(false) {
                line.push_str(", tls=true");
            }
            if let Some(sni) =
                nodes::node_str(node, "servername").or_else(|| nodes::node_str(node, "sni"))
            {
                line.push_str(&format!(", sni={}", line_value(sni)));
            }
            if nodes::node_str(node, "network")
                .map(|network| network.eq_ignore_ascii_case("ws"))
                .unwrap_or(false)
            {
                line.push_str(", ws=true");
                if let Some(path) = nodes::ws_path(node) {
                    line.push_str(&format!(", ws-path={}", line_value(&path)));
                }
                if let Some(host) = nodes::ws_host(node) {
                    line.push_str(&format!(", ws-headers=Host:{}", line_value(&host)));
                }
            }
            if nodes::node_bool(node, "skip-cert-verify").unwrap_or(false) {
                line.push_str(", skip-cert-verify=true");
            }
        }
        "trojan" => {
            let password = line_value(nodes::node_str(node, "password")?);
            line.push_str(&format!("trojan, {server}, {port}, password={password}"));
            if let Some(sni) =
                nodes::node_str(node, "sni").or_else(|| nodes::node_str(node, "servername"))
            {
                line.push_str(&format!(", sni={}", line_value(sni)));
            }
            if nodes::node_bool(node, "skip-cert-verify").unwrap_or(false) {
                line.push_str(", skip-cert-verify=true");
            }
            if nodes::node_str(node, "network")
                .map(|network| network.eq_ignore_ascii_case("ws"))
                .unwrap_or(false)
            {
                line.push_str(", ws=true");
                if let Some(path) = nodes::ws_path(node) {
                    line.push_str(&format!(", ws-path={}", line_value(&path)));
                }
                if let Some(host) = nodes::ws_host(node) {
                    line.push_str(&format!(", ws-headers=Host:{}", line_value(&host)));
                }
            }
        }
        "http" => {
            line.push_str(&format!("http, {server}, {port}"));
            if let Some(username) = nodes::node_str(node, "username") {
                line.push_str(&format!(", username={}", line_value(username)));
            }
            if let Some(password) = nodes::node_str(node, "password") {
                line.push_str(&format!(", password={}", line_value(password)));
            }
            if nodes::node_bool(node, "tls").unwrap_or(false) {
                line.push_str(", tls=true");
            }
        }
        "socks5" => {
            line.push_str(&format!("socks5, {server}, {port}"));
            if let Some(username) = nodes::node_str(node, "username") {
                line.push_str(&format!(", username={}", line_value(username)));
            }
            if let Some(password) = nodes::node_str(node, "password") {
                line.push_str(&format!(", password={}", line_value(password)));
            }
        }
        _ => return None,
    }
    Some(line)
}

// ---------------------------------------------------------------------------
// Quantumult X
// ---------------------------------------------------------------------------

/// Renders a Quantumult X configuration for the given nodes.
pub fn render_quantumultx_config(nodes: &[ProxyNode]) -> String {
    render_quantumultx_profile(nodes, nodes::DEFAULT_PROFILE)
}

pub(crate) fn render_quantumultx_profile(nodes: &[ProxyNode], profile_name: &str) -> String {
    let profile = nodes::sanitize_profile_name(profile_name);
    let mut body = String::new();
    body.push_str(&format!("# VPN233 Quantumult X profile: {profile}\n"));
    body.push_str("[general]\n");
    body.push_str("[server_local]\n");
    let mut tags: Vec<String> = Vec::new();
    for (name, node) in nodes::prepared_line_nodes(nodes, nodes::LINE_RESERVED) {
        if let Some(line) = quantumultx_server_line(&name, &node) {
            tags.push(name);
            body.push_str(&line);
            body.push('\n');
        }
    }
    body.push_str("[policy]\n");
    tags.push("direct".to_string());
    body.push_str(&format!(
        "static={}, {}\n",
        nodes::MAIN_GROUP,
        tags.join(", ")
    ));
    body
}

fn quantumultx_server_line(name: &str, node: &ProxyNode) -> Option<String> {
    let kind = nodes::node_type_lower(node)?;
    let server = nodes::node_server(node)?;
    let port = nodes::node_u16(node, "port")?;
    let mut line = match kind.as_str() {
        "ss" => {
            let cipher = line_value(nodes::node_str(node, "cipher")?);
            let password = line_value(nodes::node_str(node, "password")?);
            format!("shadowsocks={server}:{port}, method={cipher}, password={password}")
        }
        "vmess" => {
            let uuid = line_value(nodes::node_str(node, "uuid")?);
            let mut line =
                format!("vmess={server}:{port}, method=chacha20-poly1305, password={uuid}");
            if nodes::node_bool(node, "tls").unwrap_or(false) {
                line.push_str(", over-tls=true");
                if let Some(sni) =
                    nodes::node_str(node, "servername").or_else(|| nodes::node_str(node, "sni"))
                {
                    line.push_str(&format!(", tls-host={}", line_value(sni)));
                }
            }
            if nodes::node_str(node, "network")
                .map(|network| network.eq_ignore_ascii_case("ws"))
                .unwrap_or(false)
            {
                let tls = nodes::node_bool(node, "tls").unwrap_or(false);
                line.push_str(if tls { ", obfs=wss" } else { ", obfs=ws" });
                if let Some(host) = nodes::ws_host(node) {
                    line.push_str(&format!(", obfs-host={}", line_value(&host)));
                }
                let path = nodes::ws_path(node).unwrap_or_else(|| "/".to_string());
                line.push_str(&format!(", obfs-uri={}", line_value(&path)));
            }
            line
        }
        "trojan" => {
            let password = line_value(nodes::node_str(node, "password")?);
            let mut line = format!("trojan={server}:{port}, password={password}, over-tls=true");
            if let Some(sni) =
                nodes::node_str(node, "sni").or_else(|| nodes::node_str(node, "servername"))
            {
                line.push_str(&format!(", tls-host={}", line_value(sni)));
            }
            let verification = if nodes::node_bool(node, "skip-cert-verify").unwrap_or(false) {
                "tls-verification=false"
            } else {
                "tls-verification=true"
            };
            line.push_str(&format!(", {verification}"));
            line
        }
        "vless" => {
            let uuid = line_value(nodes::node_str(node, "uuid")?);
            let mut line = format!("vless={server}:{port}, method=none, password={uuid}");
            if nodes::node_bool(node, "tls").unwrap_or(false) {
                line.push_str(", over-tls=true");
                if let Some(sni) =
                    nodes::node_str(node, "servername").or_else(|| nodes::node_str(node, "sni"))
                {
                    line.push_str(&format!(", tls-host={}", line_value(sni)));
                }
            }
            if nodes::node_str(node, "network")
                .map(|network| network.eq_ignore_ascii_case("ws"))
                .unwrap_or(false)
            {
                let tls = nodes::node_bool(node, "tls").unwrap_or(false);
                line.push_str(if tls { ", obfs=wss" } else { ", obfs=ws" });
                if let Some(host) = nodes::ws_host(node) {
                    line.push_str(&format!(", obfs-host={}", line_value(&host)));
                }
                let path = nodes::ws_path(node).unwrap_or_else(|| "/".to_string());
                line.push_str(&format!(", obfs-uri={}", line_value(&path)));
            }
            line
        }
        "http" => {
            let mut line = format!("http={server}:{port}");
            if let Some(username) = nodes::node_str(node, "username") {
                line.push_str(&format!(", username={}", line_value(username)));
            }
            if let Some(password) = nodes::node_str(node, "password") {
                line.push_str(&format!(", password={}", line_value(password)));
            }
            if nodes::node_bool(node, "tls").unwrap_or(false) {
                line.push_str(", over-tls=true");
            }
            line
        }
        "socks5" => {
            let mut line = format!("socks5={server}:{port}");
            if let Some(username) = nodes::node_str(node, "username") {
                line.push_str(&format!(", username={}", line_value(username)));
            }
            if let Some(password) = nodes::node_str(node, "password") {
                line.push_str(&format!(", password={}", line_value(password)));
            }
            line
        }
        _ => return None,
    };
    line.push_str(", fast-open=false, udp-relay=false");
    line.push_str(&format!(", tag={}", line_value(name)));
    Some(line)
}

// ---------------------------------------------------------------------------
// Loon
// ---------------------------------------------------------------------------

/// Renders a Loon configuration for the given nodes.
pub fn render_loon_config(nodes: &[ProxyNode]) -> String {
    render_loon_profile(nodes, nodes::DEFAULT_PROFILE)
}

pub(crate) fn render_loon_profile(nodes: &[ProxyNode], profile_name: &str) -> String {
    let profile = nodes::sanitize_profile_name(profile_name);
    let mut body = String::new();
    body.push_str(&format!("# VPN233 Loon profile: {profile}\n"));
    body.push_str("[General]\n");
    body.push_str("loglevel = notify\n");
    body.push_str("dns-server = system\n");
    body.push_str("[Proxy]\n");
    let mut names: Vec<String> = Vec::new();
    for (name, node) in nodes::prepared_line_nodes(nodes, nodes::LINE_RESERVED) {
        if let Some(line) = loon_proxy_line(&name, &node) {
            names.push(name);
            body.push_str(&line);
            body.push('\n');
        }
    }
    body.push_str("[Proxy Group]\n");
    body.push_str(&format!(
        "{} = select, {}\n",
        nodes::MAIN_GROUP,
        group_members(&names)
    ));
    append_rule_section(&mut body);
    body
}

fn push_loon_transport(line: &mut String, node: &ProxyNode) {
    let Some(network) = nodes::node_str(node, "network") else {
        return;
    };
    match network.trim().to_ascii_lowercase().as_str() {
        "ws" => {
            line.push_str(", transport=ws");
            if let Some(path) = nodes::ws_path(node) {
                line.push_str(&format!(", path={}", line_value(&path)));
            }
            if let Some(host) = nodes::ws_host(node) {
                line.push_str(&format!(", host={}", line_value(&host)));
            }
        }
        "grpc" => {
            line.push_str(", transport=grpc");
            if let Some(service) = nodes::grpc_service(node) {
                line.push_str(&format!(", serviceName={}", line_value(&service)));
            }
        }
        _ => {}
    }
}

fn push_loon_tls(line: &mut String, node: &ProxyNode) {
    if nodes::node_bool(node, "tls").unwrap_or(false) {
        line.push_str(", tls=true");
        if let Some(sni) =
            nodes::node_str(node, "servername").or_else(|| nodes::node_str(node, "sni"))
        {
            line.push_str(&format!(", sni={}", line_value(sni)));
        }
    }
    if nodes::node_bool(node, "skip-cert-verify").unwrap_or(false) {
        line.push_str(", skip-cert-verify=true");
    }
}

fn loon_proxy_line(name: &str, node: &ProxyNode) -> Option<String> {
    let kind = nodes::node_type_lower(node)?;
    let server = nodes::node_server(node)?;
    let port = nodes::node_u16(node, "port")?;
    let line = match kind.as_str() {
        "ss" => {
            let cipher = line_value(nodes::node_str(node, "cipher")?);
            let password = quoted(nodes::node_str(node, "password")?);
            format!("{name} = Shadowsocks, {server}, {port}, {cipher}, {password}")
        }
        "vmess" => {
            let uuid = quoted(nodes::node_str(node, "uuid")?);
            let cipher = line_value(nodes::node_str(node, "cipher").unwrap_or("auto"));
            let alter_id = nodes::node_number(node, "alterId").unwrap_or(0);
            let mut line = format!(
                "{name} = VMess, {server}, {port}, {uuid}, alterId={alter_id}, cipher={cipher}"
            );
            push_loon_tls(&mut line, node);
            push_loon_transport(&mut line, node);
            line
        }
        "vless" => {
            let uuid = quoted(nodes::node_str(node, "uuid")?);
            let mut line = format!("{name} = VLESS, {server}, {port}, {uuid}");
            push_loon_tls(&mut line, node);
            push_loon_transport(&mut line, node);
            line
        }
        "trojan" => {
            let password = quoted(nodes::node_str(node, "password")?);
            let mut line = format!("{name} = Trojan, {server}, {port}, {password}");
            line.push_str(", tls=true");
            if let Some(sni) =
                nodes::node_str(node, "sni").or_else(|| nodes::node_str(node, "servername"))
            {
                line.push_str(&format!(", sni={}", line_value(sni)));
            }
            if nodes::node_bool(node, "skip-cert-verify").unwrap_or(false) {
                line.push_str(", skip-cert-verify=true");
            }
            push_loon_transport(&mut line, node);
            line
        }
        "hysteria2" | "hy2" => {
            let password = quoted(nodes::node_str(node, "password")?);
            let mut line = format!("{name} = Hysteria2, {server}, {port}, {password}");
            if let Some(sni) =
                nodes::node_str(node, "sni").or_else(|| nodes::node_str(node, "servername"))
            {
                line.push_str(&format!(", sni={}", line_value(sni)));
            }
            if nodes::node_bool(node, "skip-cert-verify").unwrap_or(false) {
                line.push_str(", skip-cert-verify=true");
            }
            line
        }
        "http" => {
            let mut line = format!("{name} = HTTP, {server}, {port}");
            if let Some(username) = nodes::node_str(node, "username") {
                line.push_str(&format!(", {}, ", quoted(username)));
                line.push_str(&quoted(
                    nodes::node_str(node, "password").unwrap_or_default(),
                ));
            }
            if nodes::node_bool(node, "tls").unwrap_or(false) {
                line.push_str(", tls=true");
            }
            line
        }
        "socks5" => {
            let mut line = format!("{name} = Socks5, {server}, {port}");
            if let Some(username) = nodes::node_str(node, "username") {
                line.push_str(&format!(", {}, ", quoted(username)));
                line.push_str(&quoted(
                    nodes::node_str(node, "password").unwrap_or_default(),
                ));
            }
            line
        }
        _ => return None,
    };
    Some(line)
}

#[cfg(test)]
mod tests;
