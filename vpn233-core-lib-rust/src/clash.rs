//! Clash / Mihomo configuration rendering plus the Stash dialect, which is
//! structurally compatible.

use serde::Serialize;
use serde_yaml::{Mapping, Value};

use crate::nodes;
use crate::{string, ProxyNode};

/// Service groups appended to every generated profile. They only reference the
/// `Proxies` group so the profile imports without downloading external geodata.
const SERVICE_GROUPS: &[(&str, &str)] = &[
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

/// Renders a Clash/Mihomo configuration for the given nodes.
///
/// This is the legacy entry point; it keeps the historical output shape while
/// stripping VPN233-internal attributes and de-duplicating node names.
pub fn render_clash_config(nodes: &[ProxyNode]) -> Result<String, serde_yaml::Error> {
    render_clash_body(nodes, None)
}

/// Renders a Clash/Mihomo configuration with a profile comment header.
pub fn render_clash_profile(
    nodes: &[ProxyNode],
    profile_name: &str,
) -> Result<String, serde_yaml::Error> {
    let profile = nodes::sanitize_profile_name(profile_name);
    render_clash_body(
        nodes,
        Some(format!("# VPN233 Clash / Mihomo profile: {profile}")),
    )
}

/// Renders a Stash configuration. Stash accepts the Clash proxy/group/rule
/// schema, so the body is shared and only the header differs.
pub fn render_stash_profile(
    nodes: &[ProxyNode],
    profile_name: &str,
) -> Result<String, serde_yaml::Error> {
    let profile = nodes::sanitize_profile_name(profile_name);
    render_clash_body(nodes, Some(format!("# VPN233 Stash profile: {profile}")))
}

fn reserved_names() -> Vec<&'static str> {
    let mut reserved = nodes::CLASH_RESERVED.to_vec();
    reserved.extend(SERVICE_GROUPS.iter().map(|(name, _)| *name));
    reserved
}

fn proxy_groups(group_names: &[Value]) -> Vec<Value> {
    let mut select = Mapping::new();
    select.insert(string("name"), string(nodes::MAIN_GROUP));
    select.insert(string("type"), string("select"));
    select.insert(string("proxies"), Value::Sequence(group_names.to_vec()));

    let mut auto = Mapping::new();
    auto.insert(string("name"), string(nodes::AUTO_GROUP));
    // Keep the historical group names without silently probing a third party.
    // Users may explicitly add their own url-test policy after importing.
    auto.insert(string("type"), string("select"));
    auto.insert(string("proxies"), Value::Sequence(group_names.to_vec()));

    let mut fallback = Mapping::new();
    fallback.insert(string("name"), string(nodes::FALLBACK_GROUP));
    fallback.insert(string("type"), string("select"));
    fallback.insert(string("proxies"), Value::Sequence(group_names.to_vec()));

    let mut proxies = Mapping::new();
    proxies.insert(string("name"), string(nodes::PROXIES_GROUP));
    proxies.insert(string("type"), string("select"));
    proxies.insert(
        string("proxies"),
        Value::Sequence(vec![
            string(nodes::AUTO_GROUP),
            string(nodes::FALLBACK_GROUP),
        ]),
    );

    let mut groups = vec![
        Value::Mapping(select),
        Value::Mapping(auto),
        Value::Mapping(fallback),
        Value::Mapping(proxies),
    ];
    for (name, _) in SERVICE_GROUPS {
        let mut group = Mapping::new();
        group.insert(string("name"), string(*name));
        group.insert(string("type"), string("select"));
        group.insert(
            string("proxies"),
            Value::Sequence(vec![
                string(nodes::PROXIES_GROUP),
                string(nodes::DIRECT_POLICY),
            ]),
        );
        groups.push(Value::Mapping(group));
    }
    groups
}

fn clash_rules() -> Vec<String> {
    // Keep subscription output self-contained. GEOSITE/GEOIP rules make
    // Clash Verge download external geodata during import, which can fail
    // behind restricted networks and roll back an otherwise valid
    // subscription. Users can still choose service groups manually.
    let mut rules = vec!["DOMAIN-SUFFIX,local,DIRECT".to_string()];
    rules.extend(
        nodes::PRIVATE_CIDRS
            .iter()
            .map(|cidr| format!("IP-CIDR,{cidr},DIRECT,no-resolve")),
    );
    rules.extend(
        nodes::PRIVATE_IPV6_CIDRS
            .iter()
            .map(|cidr| format!("IP-CIDR6,{cidr},DIRECT,no-resolve")),
    );
    rules.push(format!("MATCH,{}", nodes::MAIN_GROUP));
    rules
}

fn render_clash_body(
    nodes: &[ProxyNode],
    header: Option<String>,
) -> Result<String, serde_yaml::Error> {
    let reserved = reserved_names();
    let prepared = nodes::prepared_structured_nodes(nodes, &reserved);
    let names: Vec<Value> = prepared.iter().map(|(name, _)| string(name)).collect();
    let proxies: Vec<ProxyNode> = prepared.into_iter().map(|(_, node)| node).collect();
    let group_names = if names.is_empty() {
        vec![string(nodes::DIRECT_POLICY)]
    } else {
        names
    };
    let config = ClashConfig {
        mixed_port: nodes::MIXED_PORT,
        allow_lan: false,
        mode: "rule",
        log_level: "info",
        proxies,
        proxy_groups: proxy_groups(&group_names),
        rules: clash_rules(),
    };
    let body = serde_yaml::to_string(&config)?;
    Ok(match header {
        Some(header) => format!("{header}\n{body}"),
        None => body,
    })
}

#[cfg(test)]
mod tests;
