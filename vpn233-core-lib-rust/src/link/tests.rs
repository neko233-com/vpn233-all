//! Tests for link parsing and rendering.

use super::*;

#[test]
fn parses_a_basic_vless_link() {
    let node = parse_link(
        "vless://11111111-2222-3333-4444-555555555555@example.invalid:443?security=tls&sni=example.invalid&type=ws&path=%2Fws#demo",
        0,
    )
    .unwrap();
    assert_eq!(nodes::node_type_lower(&node).as_deref(), Some("vless"));
    assert_eq!(nodes::node_server(&node), Some("example.invalid"));
    assert_eq!(nodes::node_u16(&node, "port"), Some(443));
    assert_eq!(nodes::node_name(&node), Some("demo"));
}

#[test]
fn invalid_links_return_indexed_errors() {
    let (nodes, errors) = parse_links(["", "nonsense", "vless://u@example.invalid:443#x"]);
    assert_eq!(nodes.len(), 1);
    assert_eq!(errors.len(), 2);
    assert_eq!(errors[0].index, 0);
    assert_eq!(errors[1].index, 1);
}

#[test]
fn render_link_generates_a_parseable_ss_link() {
    let mut link = proxy_node("demo", "ss", "example.invalid", 8388);
    link.insert("cipher".into(), string("aes-256-gcm"));
    link.insert("password".into(), string("secret"));
    let raw = render_link(&link).unwrap();
    let parsed = parse_link(&raw, 0).unwrap();
    assert_eq!(nodes::node_type_lower(&parsed).as_deref(), Some("ss"));
    assert_eq!(nodes::node_str(&parsed, "cipher"), Some("aes-256-gcm"));
}

#[test]
fn transport_roundtrips_preserve_http_hosts_and_httpupgrade() {
    for kind in ["vless", "trojan"] {
        for network in ["ws", "grpc", "http", "h2", "httpupgrade"] {
            let raw = format!("{kind}://change-me@example.invalid:443?type={network}&security=tls&host=front.example.invalid&path=%2Ftransport&serviceName=service&fp=chrome#demo");
            let node = parse_link(&raw, 0).unwrap();
            let rendered = render_link(&node).unwrap();
            let reparsed = parse_link(&rendered, 0).unwrap();
            assert_eq!(nodes::node_network(&node), nodes::node_network(&reparsed));
            assert_eq!(
                nodes::transport_host(&node),
                nodes::transport_host(&reparsed)
            );
            assert_eq!(
                nodes::transport_path(&node),
                nodes::transport_path(&reparsed)
            );
            assert_eq!(nodes::grpc_service(&node), nodes::grpc_service(&reparsed));
            assert_eq!(
                nodes::node_str(&node, "client-fingerprint"),
                nodes::node_str(&reparsed, "client-fingerprint")
            );
        }
    }
}

#[test]
fn hysteria2_username_password_auth_and_ipv6_are_preserved() {
    let raw =
        "hy2://user:change%3Ame@[2001:db8::1]:443?obfs=salamander&obfs-password=change-me#demo";
    let parsed = parse_link(raw, 0).unwrap();
    assert_eq!(nodes::node_str(&parsed, "password"), Some("user:change:me"));
    let reparsed = parse_link(&render_link(&parsed).unwrap(), 0).unwrap();
    assert_eq!(
        nodes::node_str(&reparsed, "password"),
        Some("user:change:me")
    );
    assert_eq!(nodes::node_server(&reparsed), Some("2001:db8::1"));
}

#[test]
fn ssr_ipv6_and_optional_parameters_roundtrip() {
    let mut node = proxy_node("ipv6", "ssr", "2001:db8::1", 8388);
    node.insert("cipher".into(), string("aes-256-cfb"));
    node.insert("password".into(), string("change-me"));
    node.insert("protocol".into(), string("auth_sha1_v4"));
    node.insert("obfs".into(), string("tls1.2_ticket_auth"));
    node.insert("protocol-param".into(), string("change-me:change-me"));
    node.insert("obfs-param".into(), string("front.example.invalid"));
    let parsed = parse_link(&render_link(&node).unwrap(), 0).unwrap();
    for key in [
        "server",
        "port",
        "protocol",
        "cipher",
        "password",
        "obfs",
        "protocol-param",
        "obfs-param",
    ] {
        assert_eq!(node.get(key), parsed.get(key), "{key}");
    }
}

#[test]
fn errors_do_not_echo_untrusted_scheme_transport_cipher_or_json_values() {
    for raw in [
        "private-password-change-me://example.invalid",
        "vless://change-me@example.invalid?type=private-password-change-me",
        "ss://private-password-change-me:change-me@example.invalid:8388",
        "vmess://eyJwcml2YXRlLXBhc3N3b3JkLWNoYW5nZS1tZSI6IFVOREVGSU5FRH0=",
        "trojan://change-me@example.invalid:0",
        "vless://change-me@example.invalid:443\nhttp://example.invalid:80",
    ] {
        let error = parse_link(raw, 7).unwrap_err();
        assert_eq!(error.index, 7);
        assert!(!error.to_string().contains("private-password"));
        assert!(!error.to_string().contains("change-me"));
    }
}
