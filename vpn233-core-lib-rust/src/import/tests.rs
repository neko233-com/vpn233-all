use super::*;
use base64::{
    engine::general_purpose::{STANDARD, STANDARD_NO_PAD, URL_SAFE, URL_SAFE_NO_PAD},
    Engine,
};

const LINKS: &[&str] = &[
    "vless://11111111-2222-3333-4444-555555555555@example.invalid:443?security=reality&sni=tls.example.invalid&pbk=change-me&sid=abcd&type=grpc&serviceName=change-me#vless",
    "vmess://eyJhZGQiOiJleGFtcGxlLmludmFsaWQiLCJwb3J0Ijo0NDMsImlkIjoiY2hhbmdlLW1lIiwicHMiOiJ2bWVzcyIsIm5ldCI6IndzIiwicGF0aCI6Ii93cyIsInRscyI6InRscyJ9",
    "trojan://change-me@example.invalid:443?type=ws&host=front.example.invalid&path=%2Fws#trojan",
    "ss://YWVzLTI1Ni1nY206Y2hhbmdlLW1l@example.invalid:8388#ss",
    "ssr://ZXhhbXBsZS5pbnZhbGlkOjgzODg6b3JpZ2luOmFlcy0yNTYtZ2NtOnBsYWluOlkyaGhibWRsTFcxbC8_cmVtYXJrcz1jM055",
    "hy2://change-me@example.invalid:443?sni=tls.example.invalid&obfs=salamander&obfs-password=change-me#hy2",
    "tuic://11111111-2222-3333-4444-555555555555:change-me@example.invalid:443?congestion_control=bbr&alpn=h3#tuic",
    "anytls://change-me@example.invalid:443?sni=tls.example.invalid#anytls",
    "socks5://change-me:change-me@example.invalid:1080#socks5",
    "https://change-me:change-me@example.invalid:443?sni=tls.example.invalid#https",
];

#[test]
fn imports_all_ten_uri_protocols_and_roundtrips_endpoints() {
    let (nodes, errors) = parse_subscription(&LINKS.join("\n"));
    assert!(errors.is_empty(), "{errors:?}");
    assert_eq!(nodes.len(), 10);
    for node in nodes {
        let link = crate::render_link(&node).unwrap();
        let parsed = crate::parse_link(&link, 0).unwrap();
        for key in ["type", "server", "port", "uuid", "password", "cipher"] {
            assert_eq!(node.get(key), parsed.get(key), "{key}");
        }
    }
}

#[test]
fn accepts_all_base64_alphabets_padding_wrapping_and_bom() {
    let text = format!(
        "\u{feff}# comment\r\n{}\r\n; comment\r\n{}\r\n",
        LINKS[0], LINKS[3]
    );
    for engine in [&STANDARD, &STANDARD_NO_PAD, &URL_SAFE, &URL_SAFE_NO_PAD] {
        let encoded = engine.encode(text.as_bytes());
        let wrapped = encoded
            .as_bytes()
            .chunks(47)
            .map(|chunk| std::str::from_utf8(chunk).unwrap())
            .collect::<Vec<_>>()
            .join("\r\n");
        let (nodes, errors) = parse_subscription(&wrapped);
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(nodes.len(), 2);
    }
}

#[test]
fn imports_mihomo_options_and_discards_global_configuration() {
    let payload = r#"
proxies:
  - name: extended
    type: vless
    server: example.invalid
    port: '443'
    uuid: change-me
    tls: true
    udp: true
    network: ws
    ws-opts:
      path: /ws
      headers: { Host: front.example.invalid }
      max-early-data: 2048
    reality-opts: { public-key: change-me, short-id: abcd }
    vpn233-link: 'http://must-not-export.example.invalid'
proxy-providers:
  unwanted: { url: 'https://must-not-fetch.example.invalid' }
external-controller: 0.0.0.0:9090
secret: change-me
"#;
    let (nodes, errors) = parse_subscription(payload);
    assert!(errors.is_empty());
    assert_eq!(nodes.len(), 1);
    assert_eq!(nodes[0].get("udp"), Some(&boolean(true)));
    assert!(!nodes[0].contains_key(link::LINK_KEY));
    let rendered =
        crate::render_subscription(formats::SubscriptionFormat::Clash, &nodes, "demo").unwrap();
    assert!(rendered.contains("max-early-data: 2048"));
    for unwanted in [
        "must-not",
        "external-controller",
        "proxy-providers",
        "secret:",
    ] {
        assert!(!rendered.contains(unwanted));
    }
}

#[test]
fn imports_mihomo_only_protocols_without_inventing_uris() {
    for kind in ["hysteria", "wireguard", "snell", "ssh", "mieru"] {
        let payload = format!("proxies:\n- name: demo\n  type: {kind}\n  server: example.invalid\n  port: 443\n  password: change-me\n");
        let (nodes, errors) = parse_subscription(&payload);
        assert!(errors.is_empty(), "{kind}: {errors:?}");
        let output =
            crate::render_subscription(formats::SubscriptionFormat::Clash, &nodes, "demo").unwrap();
        assert!(output.contains(&format!("type: {kind}")));
        assert!(crate::render_link(&nodes[0]).is_err());
        assert!(!formats::SubscriptionFormat::Links.supports_protocol(kind));
    }
}

#[test]
fn imports_singbox_nodes_but_never_remote_probes_or_routing_policies() {
    let payload = r#"{
      "outbounds": [
        {"type":"vless","tag":"demo","server":"example.invalid","server_port":443,"uuid":"change-me","flow":"xtls-rprx-vision",
         "tls":{"enabled":true,"server_name":"tls.example.invalid","alpn":["h2"],"utls":{"enabled":true,"fingerprint":"chrome"},"reality":{"enabled":true,"public_key":"change-me","short_id":"abcd"}},
         "transport":{"type":"grpc","service_name":"demo"}},
        {"type":"urltest","tag":"probe","url":"https://must-not-probe.example.invalid"},
        {"type":"selector","tag":"select","outbounds":["demo"]},
        {"type":"direct","tag":"direct"}
      ],
      "route":{"rule_set":[{"url":"https://must-not-fetch.example.invalid"}]}
    }"#;
    let (nodes, errors) = parse_subscription(payload);
    assert!(errors.is_empty(), "{errors:?}");
    assert_eq!(nodes.len(), 1);
    let config: Json = serde_json::from_str(&crate::render_singbox_config(&nodes)).unwrap();
    let outbound = &config["outbounds"][0];
    assert_eq!(outbound["tls"]["reality"]["public_key"], "change-me");
    assert_eq!(outbound["tls"]["utls"]["fingerprint"], "chrome");
    assert_eq!(outbound["transport"]["service_name"], "demo");
    assert!(!config.to_string().contains("must-not"));
}

#[test]
fn imports_httpupgrade_as_mihomo_websocket_upgrade_and_exports_real_transport() {
    let payload = r#"{"outbounds":[{"type":"trojan","tag":"demo","server":"example.invalid","server_port":443,"password":"change-me","tls":{"enabled":true},"transport":{"type":"httpupgrade","host":"front.example.invalid","path":"/upgrade"}}]}"#;
    let (nodes, errors) = parse_subscription(payload);
    assert!(errors.is_empty());
    assert_eq!(nodes::node_network(&nodes[0]), "httpupgrade");
    let config: Json = serde_json::from_str(&crate::render_singbox_config(&nodes)).unwrap();
    assert_eq!(config["outbounds"][0]["transport"]["type"], "httpupgrade");
    assert_eq!(
        config["outbounds"][0]["transport"]["host"],
        "front.example.invalid"
    );
    let uri = crate::render_link(&nodes[0]).unwrap();
    assert!(uri.contains("type=httpupgrade"));
    assert_eq!(
        nodes::node_network(&crate::parse_link(&uri, 0).unwrap()),
        "httpupgrade"
    );
}

#[test]
fn malformed_nodes_are_skipped_without_disclosing_private_inputs() {
    let payload = r#"proxies:
  - { name: bad, type: private-password-change-me, server: example.invalid, port: 443 }
  - { name: bad, type: http, server: 'example.invalid, private-password-change-me', port: 443 }
  - { name: bad, type: http, server: example.invalid, port: 0 }
  - { name: ok, type: https, server: example.invalid, port: 443 }
"#;
    let (nodes, errors) = parse_subscription(payload);
    assert_eq!(nodes.len(), 1);
    assert_eq!(errors.len(), 3);
    assert_eq!(nodes[0].get("tls"), Some(&boolean(true)));
    assert!(!format!("{errors:?}").contains("private-password"));
}

#[test]
fn unsupported_transports_and_chains_are_explicit_errors() {
    for outbound in [
        r#"{"type":"vless","server":"example.invalid","server_port":443,"uuid":"change-me","transport":{"type":"quic"}}"#,
        r#"{"type":"http","server":"example.invalid","server_port":443,"detour":"missing"}"#,
        r#"{"type":"socks","version":"4","server":"example.invalid","server_port":1080}"#,
    ] {
        let (nodes, errors) = parse_subscription(&format!("{{\"outbounds\":[{outbound}]}}"));
        assert!(nodes.is_empty());
        assert_eq!(errors.len(), 1);
    }
}

#[test]
fn no_generated_format_configures_third_party_probes_or_dns() {
    let node = crate::parse_link(LINKS[3], 0).unwrap();
    for format in formats::SubscriptionFormat::all() {
        let output =
            crate::render_subscription(*format, std::slice::from_ref(&node), "demo").unwrap();
        for unwanted in [
            "gstatic",
            "223.5.5.5",
            "119.29.29.29",
            "url-test",
            "urltest",
            "server_check_url",
            "#!MANAGED-CONFIG",
        ] {
            assert!(!output.contains(unwanted), "{format:?} contains {unwanted}");
        }
    }
}
