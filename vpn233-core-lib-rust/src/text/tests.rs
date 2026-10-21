//! Tests for line oriented client formats.

use super::*;
use crate::proxy_node;

#[test]
fn surge_output_contains_the_expected_sections() {
    let nodes = vec![proxy_node("demo", "ss", "example.invalid", 8388)];
    let body = render_surge_config(&nodes);
    assert!(body.starts_with("# VPN233 Surge profile:"));
    for section in ["[General]", "[Proxy]", "[Proxy Group]", "[Rule]"] {
        assert!(body.contains(section), "missing {section}");
    }
    assert!(body.contains("FINAL,VPN233"));
}

#[test]
fn quantumultx_output_contains_the_expected_sections() {
    let body = render_quantumultx_config(&[proxy_node("demo", "ss", "example.invalid", 8388)]);
    for section in ["[general]", "[server_local]", "[policy]"] {
        assert!(body.contains(section), "missing {section}");
    }
    assert!(!body.contains("server_check_url="));
}

#[test]
fn loon_and_surfboard_outputs_are_not_empty() {
    let nodes = vec![proxy_node("demo", "ss", "example.invalid", 8388)];
    let loon = render_loon_config(&nodes);
    let surfboard = render_surfboard_config(&nodes);
    assert!(!loon.is_empty());
    assert!(!surfboard.is_empty());
}
