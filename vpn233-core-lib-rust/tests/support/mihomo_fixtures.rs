//! Shared, deliberately non-deployable inputs for the official parser gate.

use base64::{
    engine::general_purpose::{STANDARD, STANDARD_NO_PAD, URL_SAFE, URL_SAFE_NO_PAD},
    Engine,
};
use vpn233_core_lib_rust::{
    parse_link, parse_subscription, render_link, render_subscription, string, ProxyNode,
    SubscriptionFormat,
};

pub fn protocol_nodes() -> Vec<ProxyNode> {
    let fixture = include_str!("../fixtures/mihomo-protocols.yaml")
        .replace("__WG_PRIVATE__", &STANDARD.encode([0_u8; 32]))
        .replace("__WG_PUBLIC__", &STANDARD.encode([1_u8; 32]));
    imported(&fixture)
}

fn imported(payload: &str) -> Vec<ProxyNode> {
    let (nodes, errors) = parse_subscription(payload);
    assert!(errors.is_empty(), "synthetic fixtures must import");
    assert!(!nodes.is_empty(), "synthetic fixtures must contain nodes");
    nodes
}

pub fn configurations() -> Vec<(String, String)> {
    let nodes = protocol_nodes();
    let mut cases: Vec<(String, Vec<ProxyNode>)> = nodes
        .iter()
        .map(|node| {
            (
                node["type"].as_str().unwrap().to_owned(),
                vec![node.clone()],
            )
        })
        .collect();
    cases.push(("combined".into(), nodes.clone()));

    // Exercise both URI synthesis and parsing instead of only preserving YAML.
    let mut links = Vec::new();
    for node in &nodes {
        if SubscriptionFormat::Links.supports_node(node) {
            let link = render_link(node).unwrap();
            links.push(link.clone());
            cases.push((
                format!("uri-{}", node["type"].as_str().unwrap()),
                vec![parse_link(&link, 0).unwrap()],
            ));
        }
    }
    for (name, engine) in [
        ("base64-standard", &STANDARD),
        ("base64-standard-unpadded", &STANDARD_NO_PAD),
        ("base64-url", &URL_SAFE),
        ("base64-url-unpadded", &URL_SAFE_NO_PAD),
    ] {
        cases.push((name.into(), imported(&engine.encode(links.join("\r\n")))));
    }

    for kind in ["vless", "vmess", "trojan"] {
        let template = nodes.iter().find(|node| node["type"] == kind).unwrap();
        for network in ["ws", "grpc", "http", "h2", "httpupgrade"] {
            let mut node = template.clone();
            node.insert("network".into(), string(network));
            let options = match network {
                "grpc" => ("grpc-opts", "{grpc-service-name: parser-check}"),
                "h2" => ("h2-opts", "{host: [front.example.invalid], path: /parser-check}"),
                "http" => ("http-opts", "{method: GET, path: [/parser-check], headers: {Host: [front.example.invalid]}}"),
                _ => ("ws-opts", "{path: /parser-check, headers: {Host: front.example.invalid}, max-early-data: 2048, early-data-header-name: Sec-WebSocket-Protocol}"),
            };
            node.insert(options.0.into(), serde_yaml::from_str(options.1).unwrap());
            let link = render_link(&node).unwrap();
            cases.push((
                format!("transport-{kind}-{network}"),
                vec![parse_link(&link, 0).unwrap()],
            ));
        }
    }

    let key = URL_SAFE_NO_PAD.encode([2_u8; 32]);
    let reality = format!("vless://11111111-2222-3333-4444-555555555555@example.invalid:443?type=tcp&security=reality&sni=tls.example.invalid&pbk={key}&sid=abcd&fp=chrome&flow=xtls-rprx-vision#parser-check");
    cases.push(("vless-reality".into(), imported(&reality)));

    let peers = format!("proxies:\n- name: wireguard-peers\n  type: wireguard\n  ip: 192.0.2.2\n  private-key: {}\n  peers:\n  - server: example.invalid\n    port: '51820'\n    public-key: {}\n    allowed-ips: [0.0.0.0/0, '::/0']\n", STANDARD.encode([0_u8; 32]), STANDARD.encode([1_u8; 32]));
    cases.push(("wireguard-peers".into(), imported(&peers)));
    cases.push(("mieru-port-range".into(), imported("proxies:\n- name: mieru-range\n  type: mieru\n  server: example.invalid\n  port-range: 2090-2099\n  transport: TCP\n  username: change-me\n  password: change-me\n  multiplexing: MULTIPLEXING_LOW\n")));

    // Roundtrip every advertised sing-box protocol back through its importer.
    for node in &nodes {
        if SubscriptionFormat::SingBox.supports_node(node) {
            let body = render_subscription(
                SubscriptionFormat::SingBox,
                std::slice::from_ref(node),
                "Parser checks",
            )
            .unwrap();
            cases.push((
                format!("singbox-import-{}", node["type"].as_str().unwrap()),
                imported(&body),
            ));
        }
    }

    cases
        .into_iter()
        .map(|(name, nodes)| {
            let body =
                render_subscription(SubscriptionFormat::Clash, &nodes, "Synthetic parser checks")
                    .unwrap();
            (name, body)
        })
        .collect()
}
