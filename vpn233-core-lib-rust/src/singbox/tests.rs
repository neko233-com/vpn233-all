//! Tests for sing-box rendering.

use super::*;
use crate::proxy_node;

#[test]
fn singbox_output_is_parseable_json() {
    let nodes = vec![proxy_node("demo", "ss", "example.invalid", 8388)];
    let body = render_singbox_config(&nodes);
    let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(parsed["route"]["final"], "VPN233");
    assert!(parsed["outbounds"].as_array().is_some());
}

#[test]
fn singbox_skips_unsupported_protocols() {
    let body = render_singbox_config(&[proxy_node("demo", "ssr", "example.invalid", 8388)]);
    let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
    let outbounds = parsed["outbounds"].as_array().unwrap();
    assert!(!outbounds.iter().any(|outbound| outbound["tag"] == "demo"));
}
