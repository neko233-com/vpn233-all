//! Tests for the subscription format registry.

use super::*;
use crate::proxy_node;

#[test]
fn all_formats_have_unique_ids_and_display_names() {
    let mut ids: Vec<&str> = Vec::new();
    for format in SubscriptionFormat::all() {
        assert!(!ids.contains(&format.canonical_id()));
        ids.push(format.canonical_id());
        assert!(!format.display_name().is_empty());
        assert!(!format.ext().is_empty());
        assert!(!format.content_type().is_empty());
    }
    assert_eq!(ids.len(), 9);
}

#[test]
fn capabilities_are_consistent_with_supported_protocols() {
    let capabilities = format_capabilities();
    assert_eq!(capabilities.len(), 9);
    for capability in capabilities {
        assert!(!capability.protocols.is_empty());
        for protocol in capability.protocols {
            assert!(SUPPORTED_PROTOCOLS.contains(&protocol.as_str()));
        }
    }
}

#[test]
fn render_subscription_handles_every_format() {
    let mut node = proxy_node("demo", "ss", "example.invalid", 8388);
    node.insert("cipher".into(), crate::string("aes-256-gcm"));
    node.insert("password".into(), crate::string("secret"));
    let nodes = vec![node];
    for format in SubscriptionFormat::all() {
        let body = render_subscription(*format, &nodes, "demo").unwrap();
        assert!(!body.is_empty(), "{format:?} rendered nothing");
    }
}
