use serde::Serialize;
use serde_yaml::{Mapping, Value};
use std::collections::BTreeMap;

pub type ProxyNode = BTreeMap<String, Value>;

#[derive(Debug, Clone, Serialize)]
pub struct CoreConfig {
    pub mixed_port: u16,
    pub allow_lan: bool,
    pub mode: String,
    pub proxies: Vec<ProxyNode>,
    pub rules: Vec<RouteRule>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RouteRule {
    pub matcher: String,
    pub target: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Outbound {
    pub tag: String,
    pub kind: String,
    pub server: String,
    pub port: u16,
    pub options: ProxyNode,
}

impl Outbound {
    pub fn into_proxy_node(self) -> ProxyNode {
        let mut node = proxy_node(&self.tag, &self.kind, &self.server, self.port);
        node.extend(self.options);
        node
    }
}

pub trait CustomProtocol: Send + Sync {
    fn protocol_name(&self) -> &'static str;
    fn render_proxy(&self, input: &ProxyNode) -> Result<ProxyNode, String>;
}

#[derive(Default)]
pub struct ProtocolRegistry {
    protocols: BTreeMap<String, Box<dyn CustomProtocol>>,
}

impl ProtocolRegistry {
    pub fn register(&mut self, protocol: impl CustomProtocol + 'static) {
        self.protocols
            .insert(protocol.protocol_name().into(), Box::new(protocol));
    }

    pub fn render(&self, name: &str, input: &ProxyNode) -> Result<ProxyNode, String> {
        self.protocols
            .get(name)
            .ok_or_else(|| format!("unknown protocol: {name}"))?
            .render_proxy(input)
    }
}

#[derive(Debug, Clone, Serialize)]
struct ClashConfig {
    #[serde(rename = "mixed-port")]
    mixed_port: u16,
    #[serde(rename = "allow-lan")]
    allow_lan: bool,
    mode: &'static str,
    #[serde(rename = "log-level")]
    log_level: &'static str,
    proxies: Vec<ProxyNode>,
    #[serde(rename = "proxy-groups")]
    proxy_groups: Vec<Value>,
    rules: Vec<String>,
}

pub fn string(value: impl Into<String>) -> Value {
    Value::String(value.into())
}

pub fn number(value: u64) -> Value {
    Value::Number(value.into())
}

pub fn boolean(value: bool) -> Value {
    Value::Bool(value)
}

pub fn proxy_node(name: &str, kind: &str, server: &str, port: u16) -> ProxyNode {
    let mut node = ProxyNode::new();
    node.insert("name".into(), string(name));
    node.insert("type".into(), string(kind));
    node.insert("server".into(), string(server));
    node.insert("port".into(), number(port.into()));
    node
}

pub fn render_clash_config(nodes: &[ProxyNode]) -> Result<String, serde_yaml::Error> {
    let names: Vec<Value> = nodes
        .iter()
        .filter_map(|node| node.get("name").cloned())
        .collect();
    let group_names = if names.is_empty() {
        vec![string("DIRECT")]
    } else {
        names.clone()
    };

    let mut select = Mapping::new();
    select.insert(string("name"), string("VPN233"));
    select.insert(string("type"), string("select"));
    select.insert(string("proxies"), Value::Sequence(group_names.clone()));

    let mut auto = Mapping::new();
    auto.insert(string("name"), string("VPN233 Auto"));
    auto.insert(string("type"), string("url-test"));
    auto.insert(string("proxies"), Value::Sequence(group_names.clone()));
    auto.insert(
        string("url"),
        string("https://www.gstatic.com/generate_204"),
    );
    auto.insert(string("interval"), number(300));

    let mut fallback = Mapping::new();
    fallback.insert(string("name"), string("VPN233 Fallback"));
    fallback.insert(string("type"), string("fallback"));
    fallback.insert(string("proxies"), Value::Sequence(group_names));
    fallback.insert(
        string("url"),
        string("https://www.gstatic.com/generate_204"),
    );
    fallback.insert(string("interval"), number(300));

    let mut proxies = Mapping::new();
    proxies.insert(string("name"), string("Proxies"));
    proxies.insert(string("type"), string("select"));
    proxies.insert(
        string("proxies"),
        Value::Sequence(vec![string("VPN233 Auto"), string("VPN233 Fallback")]),
    );

    let service_groups = [
        ("Netflix", "netflix"),
        ("HBO", "hbo"),
        ("DisneyPlus", "disney"),
        ("YouTube", "youtube"),
        ("Bahamut", "bahamut"),
        ("Bilibili", "bilibili"),
        ("MyTVSuper", "mytvsuper"),
        ("AI", "category-ai-!cn"),
        ("Telegram", "telegram"),
        ("Crypto", "category-cryptocurrency"),
        ("Steam", "steam"),
        ("Epic", "epicgames"),
        ("Xbox", "xbox"),
        ("PlayStation", "playstation"),
        ("Microsoft", "microsoft"),
        ("Scholar", "scholar"),
        ("Apple", "apple"),
        ("Google", "google"),
        ("TikTok", "tiktok"),
    ];
    let mut service_group_values = Vec::new();
    for (name, _) in service_groups {
        let mut group = Mapping::new();
        group.insert(string("name"), string(name));
        group.insert(string("type"), string("select"));
        group.insert(
            string("proxies"),
            Value::Sequence(vec![string("Proxies"), string("DIRECT")]),
        );
        service_group_values.push(Value::Mapping(group));
    }

    let config = ClashConfig {
        mixed_port: 7890,
        allow_lan: false,
        mode: "rule",
        log_level: "info",
        proxies: nodes.to_vec(),
        proxy_groups: vec![
            Value::Mapping(select),
            Value::Mapping(auto),
            Value::Mapping(fallback),
            Value::Mapping(proxies),
        ]
        .into_iter()
        .chain(service_group_values)
        .collect(),
        // Keep subscription output self-contained. GEOSITE/GEOIP rules make
        // Clash Verge download external geodata during import, which can
        // fail behind restricted networks and roll back an otherwise valid
        // subscription. Users can still choose service groups manually.
        rules: vec![
            "DOMAIN-SUFFIX,local,DIRECT".into(),
            "IP-CIDR,127.0.0.0/8,DIRECT,no-resolve".into(),
            "IP-CIDR,10.0.0.0/8,DIRECT,no-resolve".into(),
            "IP-CIDR,172.16.0.0/12,DIRECT,no-resolve".into(),
            "IP-CIDR,192.168.0.0/16,DIRECT,no-resolve".into(),
            "MATCH,VPN233".into(),
        ],
    };
    serde_yaml::to_string(&config)
}

pub fn render_links(nodes: &[ProxyNode]) -> String {
    nodes
        .iter()
        .filter_map(|node| node.get("vpn233-link").and_then(Value::as_str))
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
}
