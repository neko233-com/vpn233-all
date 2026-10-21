//! Official-core syntax checks. The normal test validates the shared fixtures;
//! the ignored test executes only `mihomo -t`, never starts listeners or probes.

use std::{collections::BTreeSet, path::PathBuf, process::Command};
use vpn233_core_lib_rust::supported_protocols;

#[path = "support/mihomo_fixtures.rs"]
mod fixtures;

#[test]
fn every_advertised_mihomo_protocol_has_a_complete_import_fixture() {
    let nodes = fixtures::protocol_nodes();
    let actual: BTreeSet<&str> = nodes
        .iter()
        .map(|node| node["type"].as_str().unwrap())
        .collect();
    let expected: BTreeSet<&str> = supported_protocols().iter().copied().collect();
    assert_eq!(actual, expected);
    let configurations = fixtures::configurations();
    // This public coverage contract is 57 synthetic outbound configurations plus
    // eight individual inbound configurations and their combined configuration: 66.
    assert_eq!(configurations.len(), 57);
    for (name, body) in configurations {
        let yaml: serde_yaml::Value = serde_yaml::from_str(&body).unwrap();
        assert!(!yaml["proxies"].as_sequence().unwrap().is_empty(), "{name}");
        for forbidden in [
            "proxy-providers:",
            "GEOSITE,",
            "GEOIP,",
            "url-test",
            "urltest",
            "https://www.gstatic.com",
        ] {
            assert!(!body.contains(forbidden), "{name}");
        }
    }
}

#[test]
#[ignore = "requires an independently verified official Mihomo executable via VPN233_MIHOMO_TEST_BINARY"]
fn official_mihomo_accepts_each_protocol_and_combined_output() {
    let binary = std::env::var_os("VPN233_MIHOMO_TEST_BINARY")
        .map(PathBuf::from)
        .expect("VPN233_MIHOMO_TEST_BINARY must name a verified external executable");
    let root = std::env::var_os("VPN233_MIHOMO_TEST_DIR")
        .map(PathBuf::from)
        .expect("VPN233_MIHOMO_TEST_DIR must name an ignored local output directory");
    std::fs::create_dir_all(&root).unwrap();
    let mut failures = Vec::new();
    for (name, body) in fixtures::configurations() {
        let directory = root.join(&name);
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("config.yaml");
        std::fs::write(&path, body).unwrap();
        let output = Command::new(&binary)
            .args(["-t", "-d"])
            .arg(&directory)
            .arg("-f")
            .arg(&path)
            .output()
            .expect("official parser must start");
        std::fs::write(directory.join("parser.stdout"), &output.stdout).unwrap();
        std::fs::write(directory.join("parser.stderr"), &output.stderr).unwrap();
        if !output.status.success() {
            failures.push(name);
        }
    }
    assert!(failures.is_empty(), "official Mihomo rejected synthetic protocol cases: {failures:?}; inspect local parser diagnostics");
}
