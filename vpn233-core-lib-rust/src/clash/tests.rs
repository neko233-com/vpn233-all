//! Tests for Clash/Stash rendering.

use super::*;
use crate::proxy_node;

#[test]
fn clash_output_is_parseable_yaml() {
    let nodes = vec![proxy_node("demo", "ss", "example.invalid", 8388)];
    let body = render_clash_profile(&nodes, "Test Profile").unwrap();
    let parsed: serde_yaml::Value = serde_yaml::from_str(&body).unwrap();
    assert!(parsed.get("proxies").is_some());
    assert!(parsed.get("proxy-groups").is_some());
    assert!(parsed.get("rules").is_some());
}

#[test]
fn stash_output_uses_the_same_schema() {
    let body = render_stash_profile(&[], "Test").unwrap();
    assert!(body.starts_with("# VPN233 Stash profile: Test"));
    assert!(serde_yaml::from_str::<serde_yaml::Value>(&body).is_ok());
}
