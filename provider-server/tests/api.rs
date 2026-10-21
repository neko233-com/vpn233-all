//! End-to-end tests for the provider server: HTTP API, authentication, node
//! manifests, generated deployment scripts and the registry heartbeat.

use axum::{
    body::Body,
    http::{header, HeaderMap, Request, StatusCode},
    response::{IntoResponse, Response},
    routing::post,
    Json, Router,
};
use http_body_util::BodyExt;
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    io::Write,
    path::Path,
    process::{Command, Output, Stdio},
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};
use tempfile::TempDir;
use tower::ServiceExt;
use vpn233_provider_server::{
    build_app,
    config::MIRROR_PREFIXES,
    init_state,
    manifest::{format_capabilities, supported_protocols},
    registry::{RegistryReporter, RegistryStep},
    AppState, Config,
};

const ADMIN_USER: &str = "root";
const ADMIN_PASSWORD: &str = "unit-test-admin-password";
const PROVIDER_TOKEN: &str = "unit-test-provider-token";

static ENV_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn test_config() -> Config {
    Config {
        listen_addr: "127.0.0.1".into(),
        listen_port: 0,
        admin_user: ADMIN_USER.into(),
        admin_password: ADMIN_PASSWORD.into(),
        default_node_ip: "127.0.0.1".into(),
        provider_access_token: PROVIDER_TOKEN.into(),
        ..Config::default()
    }
}

async fn test_app() -> (Router, AppState) {
    let state = init_state(test_config()).await;
    (build_app(state.clone()), state)
}

async fn test_app_with(config: Config) -> (Router, AppState) {
    let state = init_state(config).await;
    (build_app(state.clone()), state)
}

fn empty_request(method: &str, uri: &str, token: Option<&str>) -> Request<Body> {
    let mut builder = Request::builder().method(method).uri(uri);
    if let Some(token) = token {
        builder = builder.header(header::AUTHORIZATION, format!("Bearer {token}"));
    }
    builder.body(Body::empty()).expect("valid request")
}

async fn call(app: &Router, method: &str, uri: &str, token: Option<&str>) -> (StatusCode, String) {
    let response = app
        .clone()
        .oneshot(empty_request(method, uri, token))
        .await
        .expect("router responds");
    let status = response.status();
    let body = response
        .into_body()
        .collect()
        .await
        .expect("body collected")
        .to_bytes();
    (status, String::from_utf8_lossy(&body).to_string())
}

async fn call_json(
    app: &Router,
    method: &str,
    uri: &str,
    token: Option<&str>,
) -> (StatusCode, Value) {
    let (status, body) = call(app, method, uri, token).await;
    let value = serde_json::from_str(&body)
        .unwrap_or_else(|error| panic!("expected JSON for {method} {uri}: {error}; body={body}"));
    (status, value)
}

async fn login(app: &Router, user: &str, password: &str) -> (StatusCode, String) {
    let body = json!({"username": user, "password": password}).to_string();
    let request = Request::builder()
        .method("POST")
        .uri("/api/v1/login")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body))
        .expect("valid login request");
    let response = app.clone().oneshot(request).await.expect("router responds");
    let status = response.status();
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("body collected")
        .to_bytes();
    (status, String::from_utf8_lossy(&bytes).to_string())
}

async fn login_token(app: &Router) -> String {
    let (status, body) = login(app, ADMIN_USER, ADMIN_PASSWORD).await;
    assert_eq!(status, StatusCode::OK);
    let value: Value = serde_json::from_str(&body).expect("login JSON");
    value["token"].as_str().expect("token present").to_owned()
}

#[tokio::test]
async fn health_endpoint_reports_service() {
    let (app, _state) = test_app().await;
    let (status, value) = call_json(&app, "GET", "/api/v1/health", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(value["ok"], json!(true));
    assert_eq!(value["service"], json!("vpn233-provider-server"));
}

#[tokio::test]
async fn protocols_endpoint_lists_supported_protocols() {
    let (app, _state) = test_app().await;
    let (status, value) = call_json(&app, "GET", "/api/v1/protocols", None).await;
    assert_eq!(status, StatusCode::OK);
    let protocols = value["protocols"].as_array().expect("protocols array");
    assert_eq!(protocols.len(), supported_protocols().len());
    assert_eq!(protocols.len(), 8);
    assert!(protocols.iter().any(|value| value == "anytls"));
    let formats = value["formats"].as_array().expect("formats array");
    assert_eq!(formats.len(), format_capabilities().len());
    for format in formats {
        assert!(format["id"].is_string());
        assert!(format["protocols"].is_array());
    }
}

#[tokio::test]
async fn formats_endpoint_matches_protocol_capabilities() {
    let (app, _state) = test_app().await;
    let response = app
        .clone()
        .oneshot(empty_request("GET", "/api/v1/formats", None))
        .await
        .expect("router responds");
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response
            .headers()
            .get(header::CACHE_CONTROL)
            .and_then(|value| value.to_str().ok()),
        Some("public, max-age=300")
    );
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("body collected")
        .to_bytes();
    let value: Value = serde_json::from_slice(&bytes).expect("formats JSON");
    let ids = value["formats"]
        .as_array()
        .expect("formats array")
        .iter()
        .filter_map(|format| format["id"].as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        ids,
        vec![
            "clash",
            "singbox",
            "v2ray",
            "links",
            "surge",
            "quantumultx",
            "loon",
            "surfboard",
            "stash"
        ]
    );
    assert_eq!(value["protocols"].as_array().expect("protocols").len(), 8);
}

#[tokio::test]
async fn login_success_returns_working_bearer_token() {
    let (app, _state) = test_app().await;
    let (status, body) = login(&app, ADMIN_USER, ADMIN_PASSWORD).await;
    assert_eq!(status, StatusCode::OK);
    let value: Value = serde_json::from_str(&body).expect("login JSON");
    assert_eq!(value["token_type"], json!("Bearer"));
    assert_eq!(value["expires_in"], json!(12 * 60 * 60_u64));
    let token = value["token"].as_str().expect("token").to_owned();
    let (status, _body) = call(&app, "GET", "/api/v1/config", Some(&token)).await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn login_rejects_wrong_password() {
    let (app, _state) = test_app().await;
    let (status, body) = login(&app, ADMIN_USER, "definitely-wrong").await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert!(body.contains("invalid credentials"));
}

#[tokio::test]
async fn login_rate_limit_returns_too_many_requests() {
    let (app, state) = test_app_with(Config {
        login_rate_limit: 10,
        ..test_config()
    })
    .await;
    for attempt in 0..10 {
        let (status, _body) = login(&app, ADMIN_USER, "wrong-password").await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "attempt {attempt}");
    }
    let (status, body) = login(&app, ADMIN_USER, "wrong-password").await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    assert!(body.contains("too many"));
    // The correct password is also blocked while the failure window is full.
    let (status, _body) = login(&app, ADMIN_USER, ADMIN_PASSWORD).await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    let mut auth = state.auth.write().await;
    assert!(auth.failure_count("local", vpn233_provider_server::now_secs()) >= 10);
}

#[tokio::test]
async fn logout_invalidates_the_session() {
    let (app, _state) = test_app().await;
    let token = login_token(&app).await;
    let (status, _body) = call(&app, "GET", "/api/v1/config", Some(&token)).await;
    assert_eq!(status, StatusCode::OK);
    let (status, _body) = call(&app, "POST", "/api/v1/logout", Some(&token)).await;
    assert_eq!(status, StatusCode::OK);
    let (status, _body) = call(&app, "GET", "/api/v1/config", Some(&token)).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    // Logging out twice is rejected.
    let (status, _body) = call(&app, "POST", "/api/v1/logout", Some(&token)).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn expired_sessions_are_rejected() {
    let (app, _state) = test_app_with(Config {
        session_ttl_secs: 1,
        ..test_config()
    })
    .await;
    let token = login_token(&app).await;
    tokio::time::sleep(Duration::from_millis(1_200)).await;
    let (status, _body) = call(&app, "GET", "/api/v1/config", Some(&token)).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn config_endpoint_never_leaks_secrets() {
    let (app, _state) = test_app().await;
    let token = login_token(&app).await;
    let (status, body) = call(&app, "GET", "/api/v1/config", Some(&token)).await;
    assert_eq!(status, StatusCode::OK);
    assert!(!body.contains(ADMIN_PASSWORD));
    assert!(!body.contains(PROVIDER_TOKEN));
    let value: Value = serde_json::from_str(&body).expect("config JSON");
    assert_eq!(value["provider_access_token_configured"], json!(true));
    assert_eq!(
        value["enabled_protocols"]
            .as_array()
            .expect("protocols")
            .len(),
        8
    );
    assert!(value.get("admin_password").is_none());
}

#[tokio::test]
async fn generate_json_contains_manifest_links_and_scripts() {
    let (app, _state) = test_app().await;
    let token = login_token(&app).await;
    let uri = "/api/v1/generate?node_name=unit-node&node_ip=203.0.113.10&port_base=21000&uuid=unit-uuid&password=unit-password";
    let (status, value) = call_json(&app, "GET", uri, Some(&token)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(value["node"]["node_name"], json!("unit-node"));
    assert_eq!(value["node"]["node_ip"], json!("203.0.113.10"));
    assert_eq!(value["node"]["uuid"], json!("unit-uuid"));
    assert_eq!(value["node"]["password"], json!("unit-password"));
    let ports = value["node"]["ports"].as_array().expect("ports");
    assert_eq!(ports.len(), 8);
    assert_eq!(ports[0]["protocol"], json!("vless"));
    assert_eq!(ports[0]["port"], json!(21000));
    assert_eq!(ports[7]["protocol"], json!("socks5"));
    assert_eq!(ports[7]["port"], json!(21007));
    let links = value["links"].as_array().expect("links");
    assert_eq!(links.len(), 8);
    let sh = value["sh"].as_str().expect("sh script");
    assert!(sh.starts_with("#!/usr/bin/env bash"));
    let ps1 = value["ps1"].as_str().expect("ps1 script");
    assert!(ps1.contains("[CmdletBinding()]"));
    assert_eq!(value["deployment"]["service_name"], json!("vpn233-node"));
    assert_eq!(value["deployment"]["mirrors"][0], json!(MIRROR_PREFIXES[0]));
}

#[tokio::test]
async fn generate_sh_and_ps1_are_attachments() {
    let (app, _state) = test_app().await;
    let token = login_token(&app).await;
    for (path, expected_type, filename) in [
        (
            "/api/v1/generate.sh",
            "text/plain; charset=utf-8",
            "vpn233-node.sh",
        ),
        (
            "/api/v1/generate.ps1",
            "text/plain; charset=utf-8",
            "vpn233-node.ps1",
        ),
    ] {
        let response = app
            .clone()
            .oneshot(empty_request("GET", path, Some(&token)))
            .await
            .expect("router responds");
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response
                .headers()
                .get(header::CONTENT_TYPE)
                .and_then(|value| value.to_str().ok()),
            Some(expected_type)
        );
        let disposition = response
            .headers()
            .get(header::CONTENT_DISPOSITION)
            .and_then(|value| value.to_str().ok())
            .expect("content disposition");
        assert!(disposition.contains(filename));
    }
}

#[tokio::test]
async fn generate_links_returns_every_protocol() {
    let (app, _state) = test_app().await;
    let token = login_token(&app).await;
    let (status, body) = call(&app, "GET", "/api/v1/generate.links", Some(&token)).await;
    assert_eq!(status, StatusCode::OK);
    for scheme in [
        "vless://",
        "vmess://",
        "trojan://",
        "ss://",
        "hysteria2://",
        "tuic://",
        "anytls://",
        "socks5://",
    ] {
        assert!(body.contains(scheme), "missing {scheme}");
    }
    assert_eq!(body.lines().count(), 8);
}

#[tokio::test]
async fn generate_unknown_format_is_rejected() {
    let (app, _state) = test_app().await;
    let token = login_token(&app).await;
    let (status, _body) = call(&app, "GET", "/api/v1/generate?format=exotic", Some(&token)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn generate_endpoints_require_authentication() {
    let (app, _state) = test_app().await;
    for path in [
        "/api/v1/generate",
        "/api/v1/generate.sh",
        "/api/v1/generate.ps1",
        "/api/v1/generate.links",
        "/api/v1/config",
    ] {
        let (status, _body) = call(&app, "GET", path, None).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "path {path}");
    }
}

#[tokio::test]
async fn manifest_respects_enabled_protocols() {
    let (app, _state) = test_app_with(Config {
        enabled_protocols: vec!["vless".into(), "ss".into(), "socks5".into()],
        ..test_config()
    })
    .await;
    let token = login_token(&app).await;
    let (status, value) = call_json(&app, "GET", "/api/v1/generate", Some(&token)).await;
    assert_eq!(status, StatusCode::OK);
    let protocols = value["node"]["ports"]
        .as_array()
        .expect("ports")
        .iter()
        .map(|port| port["protocol"].as_str().expect("protocol").to_owned())
        .collect::<Vec<_>>();
    assert_eq!(protocols, vec!["vless", "ss", "socks5"]);
    // The port offsets stay stable even when protocols are filtered out.
    assert_eq!(value["node"]["ports"][0]["port"], json!(10000));
    assert_eq!(value["node"]["ports"][1]["port"], json!(10003));
    assert_eq!(value["node"]["ports"][2]["port"], json!(10007));
}

#[tokio::test]
async fn manifest_port_base_can_be_overridden_per_request() {
    let (app, _state) = test_app().await;
    let token = login_token(&app).await;
    let (status, value) = call_json(
        &app,
        "GET",
        "/api/v1/generate?port_base=25000",
        Some(&token),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(value["node"]["ports"][0]["port"], json!(25000));
    assert_eq!(value["node"]["ports"][7]["port"], json!(25007));
}

#[tokio::test]
async fn nodes_feed_requires_the_provider_token() {
    let (app, _state) = test_app().await;
    let (status, _body) = call(&app, "GET", "/api/v1/nodes", None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, _body) = call(&app, "GET", "/api/v1/nodes?token=wrong-token", None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn nodes_feed_returns_generated_links_with_token() {
    let (app, _state) = test_app().await;
    let (status, body) = call(
        &app,
        "GET",
        &format!("/api/v1/nodes?token={PROVIDER_TOKEN}"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("vless://"));
    let request = Request::builder()
        .method("GET")
        .uri("/api/v1/nodes")
        .header("x-provider-token", PROVIDER_TOKEN)
        .body(Body::empty())
        .expect("request");
    let response = app.clone().oneshot(request).await.expect("response");
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn nodes_feed_returns_configured_custom_links() {
    let custom =
        "vless://11111111-1111-1111-1111-111111111111@example.invalid:443?security=tls#custom"
            .to_owned();
    let (app, _state) = test_app_with(Config {
        node_links: vec![custom.clone()],
        ..test_config()
    })
    .await;
    let (status, body) = call(
        &app,
        "GET",
        &format!("/api/v1/nodes?token={PROVIDER_TOKEN}"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body.trim(), custom);
}

#[tokio::test]
async fn subscribe_verify_checks_the_optional_token() {
    let (app, _state) = test_app_with(Config {
        subscribe_verify_token: "unit-test-subscribe-token".into(),
        ..test_config()
    })
    .await;
    let (status, _body) = call(&app, "GET", "/api/v1/subscribe/verify", None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, value) = call_json(
        &app,
        "GET",
        "/api/v1/subscribe/verify?token=unit-test-subscribe-token",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(value["ok"], json!(true));
    assert_eq!(value["protocols"].as_array().expect("protocols").len(), 8);

    let (public_app, _state) = test_app().await;
    let (status, _value) = call_json(&public_app, "GET", "/api/v1/subscribe/verify", None).await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn generated_sh_has_key_instructions_and_no_placeholders() {
    let (app, _state) = test_app().await;
    let token = login_token(&app).await;
    let (status, sh) = call(&app, "GET", "/api/v1/generate.sh", Some(&token)).await;
    assert_eq!(status, StatusCode::OK);
    for needle in [
        "#!/usr/bin/env bash",
        "set -Eeuo pipefail",
        "trap ",
        "VPN233_NODE_ROOT",
        "VPN233_DRY_RUN",
        "VPN233_SKIP_DOWNLOAD",
        "VPN233_SKIP_SERVICE",
        "VPN233_ALLOW_UNSIGNED",
        "VPN233_PUBLIC_IP",
        "curl -fsSL",
        "sha256sum",
        "systemctl enable --now",
        "vpn233-node.service",
        "ip route get 1.1.1.1",
        "hostname -I",
        "mixed-port: 7890",
        "MIRRORS=(",
    ] {
        assert!(sh.contains(needle), "missing {needle}");
    }
    assert!(!sh.contains("__"));
    assert!(!sh.contains("ipinfo.io"));
    assert!(!sh.contains("ifconfig.me"));
    assert!(!sh.contains(ADMIN_PASSWORD));
    assert!(sh.bytes().all(|byte| byte.is_ascii()));
    // Mirror contract order: each configured prefix must appear in order.
    let mut position = 0;
    for prefix in MIRROR_PREFIXES.iter().filter(|prefix| !prefix.is_empty()) {
        let found = sh[position..]
            .find(prefix)
            .unwrap_or_else(|| panic!("missing mirror {prefix}"));
        position += found + prefix.len();
    }
}

#[tokio::test]
async fn generated_ps1_has_key_instructions_and_no_placeholders() {
    let (app, _state) = test_app().await;
    let token = login_token(&app).await;
    let (status, ps1) = call(&app, "GET", "/api/v1/generate.ps1", Some(&token)).await;
    assert_eq!(status, StatusCode::OK);
    for needle in [
        "[CmdletBinding()]",
        "Set-StrictMode -Version Latest",
        "Tls12",
        "Get-FileHash",
        "schtasks.exe /Create /TN $ServiceName /SC ONSTART /RU SYSTEM",
        "New-NetFirewallRule",
        "VPN233-Node",
        "ProgramData",
        "VPN233_DRY_RUN",
        "VPN233_SKIP_DOWNLOAD",
        "VPN233_SKIP_SERVICE",
        "VPN233_ALLOW_UNSIGNED",
        "Invoke-WebRequest",
    ] {
        assert!(ps1.contains(needle), "missing {needle}");
    }
    assert!(!ps1.contains("__"));
    assert!(!ps1.contains("ipinfo.io"));
    assert!(!ps1.contains(ADMIN_PASSWORD));
    assert!(ps1.bytes().all(|byte| byte.is_ascii()));
    // PowerShell 5.1 compatibility: no modern operators.
    assert!(!ps1.contains("??"));
    assert!(!ps1.contains(" -Parallel"));
}

#[tokio::test]
async fn generate_links_can_decode_vmess_payload() {
    let (app, _state) = test_app().await;
    let token = login_token(&app).await;
    let (status, body) = call(&app, "GET", "/api/v1/generate.links", Some(&token)).await;
    assert_eq!(status, StatusCode::OK);
    let vmess = body
        .lines()
        .find(|line| line.starts_with("vmess://"))
        .expect("vmess link");
    use base64::{engine::general_purpose::STANDARD, Engine};
    let payload = STANDARD
        .decode(vmess.trim_start_matches("vmess://"))
        .expect("vmess base64");
    let payload: Value = serde_json::from_slice(&payload).expect("vmess JSON");
    assert_eq!(payload["v"], json!("2"));
    assert_eq!(payload["port"], json!("10001"));
    assert_eq!(payload["add"], json!("127.0.0.1"));
}

#[tokio::test]
async fn generated_sh_writes_config_into_custom_node_root() {
    let Some(bash) = find_bash() else {
        eprintln!("skipping: bash is not available");
        return;
    };
    let (app, _state) = test_app().await;
    let token = login_token(&app).await;
    let (status, script) = call(
        &app,
        "GET",
        "/api/v1/generate.sh?node_name=unit-node&node_ip=127.0.0.1&uuid=unit-uuid&password=unit-password",
        Some(&token),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let temp = TempDir::new().expect("temp dir");
    let root = forward_slashes(temp.path());
    let output = run_bash_script(
        &bash,
        &script,
        &[
            ("VPN233_NODE_ROOT", root),
            ("VPN233_SKIP_DOWNLOAD", "1".into()),
            ("VPN233_SKIP_SERVICE", "1".into()),
        ],
    )
    .expect("bash script runs");
    assert!(
        output.status.success(),
        "script failed: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let config_path = temp.path().join("mihomo").join("config.yaml");
    let config = std::fs::read_to_string(&config_path).expect("generated config exists");
    assert!(
        config.contains("mixed-port: 7890"),
        "unexpected config content: {config}"
    );
    assert!(config.contains("port: 10000"));
    assert!(config.contains("port: 10007"));
    assert!(config.contains("unit-uuid"));
    assert!(config.contains("unit-password"));
    assert!(!config.contains(ADMIN_PASSWORD));
    let cert_dir = temp.path().join("mihomo").join("certs");
    assert!(cert_dir.join("node.crt").exists());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("vless://"));
    assert!(stdout.contains("deployment finished"));
}

#[cfg(unix)]
#[tokio::test]
async fn generated_sh_writable_node_root_does_not_invoke_available_sudo() {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};

    let bash = find_bash().expect("bash is required for Unix installer regression");
    let (app, _state) = test_app().await;
    let token = login_token(&app).await;
    let (_, script) = call(&app, "GET", "/api/v1/generate.sh", Some(&token)).await;
    let temp = TempDir::new().unwrap();
    let shims = temp.path().join("tools");
    std::fs::create_dir(&shims).unwrap();
    let marker = temp.path().join("sudo-called");
    let actual_uid = Command::new("id").arg("-u").output().unwrap();
    assert!(actual_uid.status.success());
    let actual_uid: u32 = String::from_utf8(actual_uid.stdout)
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    // Exercise the non-root dispatch even when a local Docker test runs as root.
    // CI and the explicit non-root Docker run also verify real file ownership.
    for (name, contents) in [
        (
            "id",
            "#!/bin/sh\nif [ \"$1\" = '-u' ]; then printf '%s\\n' \"$VPN233_TEST_UID\"; else /usr/bin/id \"$@\"; fi\n",
        ),
        (
            "sudo",
            "#!/bin/sh\nprintf 'unexpected elevation\\n' >> \"$VPN233_TEST_SUDO_MARKER\"\nexit 97\n",
        ),
    ] {
        let path = shims.join(name);
        std::fs::write(&path, contents).unwrap();
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let path = std::env::join_paths(
        std::iter::once(shims).chain(std::env::split_paths(&std::env::var_os("PATH").unwrap())),
    )
    .unwrap();
    let root = temp.path().join("new custom node").join("nested");
    let environment = [
        ("PATH", path.to_string_lossy().into_owned()),
        ("VPN233_TEST_UID", actual_uid.max(1000).to_string()),
        ("VPN233_TEST_SUDO_MARKER", forward_slashes(&marker)),
        ("VPN233_NODE_ROOT", forward_slashes(&root)),
        ("VPN233_SKIP_DOWNLOAD", "1".into()),
        ("VPN233_SKIP_SERVICE", "1".into()),
    ];
    let output = run_bash_script(&bash, &script, &environment).unwrap();
    assert!(output.status.success(), "user-owned node deployment failed");
    assert!(!marker.exists(), "writable node root invoked sudo");
    let config = root.join("mihomo/config.yaml");
    assert!(std::fs::read_to_string(&config)
        .unwrap()
        .contains("mixed-port: 7890"));
    for path in [config, root.join("mihomo/certs/node.key")] {
        let metadata = std::fs::metadata(path).unwrap();
        assert_eq!(metadata.uid(), actual_uid);
        assert_eq!(metadata.permissions().mode() & 0o777, 0o600);
    }
    assert_eq!(
        std::fs::metadata(root.join("mihomo"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o700
    );
    if actual_uid != 0 {
        // A protected system parent must still request elevation. The sudo
        // fixture fails before any command can create a system directory.
        let protected = Path::new("/etc").join(temp.path().file_name().unwrap());
        assert!(!protected.exists());
        let mut protected_environment = environment.to_vec();
        protected_environment.push(("VPN233_NODE_ROOT", forward_slashes(&protected)));
        let output = run_bash_script(&bash, &script, &protected_environment).unwrap();
        assert!(!output.status.success());
        assert!(marker.is_file(), "protected system root lost elevation");
        assert!(!protected.exists(), "sudo fixture permitted system writes");
    }
}

#[tokio::test]
async fn generated_sh_dry_run_does_not_touch_the_filesystem() {
    let Some(bash) = find_bash() else {
        eprintln!("skipping: bash is not available");
        return;
    };
    let (app, _state) = test_app().await;
    let token = login_token(&app).await;
    let (status, script) = call(&app, "GET", "/api/v1/generate.sh", Some(&token)).await;
    assert_eq!(status, StatusCode::OK);
    let temp = TempDir::new().expect("temp dir");
    let root = forward_slashes(temp.path());
    let output = run_bash_script(
        &bash,
        &script,
        &[("VPN233_NODE_ROOT", root), ("VPN233_DRY_RUN", "1".into())],
    )
    .expect("bash script runs");
    assert!(
        output.status.success(),
        "dry run failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!temp.path().join("mihomo").exists());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("DRY-RUN OK"));
    assert!(stdout.contains("generated mihomo config:"));
    assert!(stdout.contains("node links:"));
}

#[tokio::test]
async fn generated_sh_passes_bash_syntax_check() {
    let Some(bash) = find_bash() else {
        eprintln!("skipping: bash is not available");
        return;
    };
    let (app, _state) = test_app().await;
    let token = login_token(&app).await;
    let (status, script) = call(&app, "GET", "/api/v1/generate.sh", Some(&token)).await;
    assert_eq!(status, StatusCode::OK);
    let output = run_bash(&bash, &["-n"], &script).expect("bash -n runs");
    assert!(
        output.status.success(),
        "bash -n failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[tokio::test]
async fn generated_ps1_passes_powershell_parser() {
    let Some(powershell) = find_powershell() else {
        eprintln!("skipping: powershell is not available");
        return;
    };
    let (app, _state) = test_app().await;
    let token = login_token(&app).await;
    let (status, script) = call(&app, "GET", "/api/v1/generate.ps1", Some(&token)).await;
    assert_eq!(status, StatusCode::OK);
    let temp = TempDir::new().expect("temp dir");
    let path = temp.path().join("node.ps1");
    std::fs::write(&path, &script).expect("write ps1");
    let command = r#"
$tokens = $null
$errors = $null
[System.Management.Automation.Language.Parser]::ParseFile($env:VPN233_PS_SCRIPT, [ref]$tokens, [ref]$errors) | Out-Null
if ($errors.Count -gt 0) {
    $errors | ForEach-Object { Write-Output $_.Message }
    exit 1
}
exit 0
"#;
    let output = Command::new(&powershell)
        .args(["-NoProfile", "-NonInteractive", "-Command", command])
        .env("VPN233_PS_SCRIPT", &path)
        .output()
        .expect("powershell parser runs");
    assert!(
        output.status.success(),
        "parser rejected the script: {}",
        String::from_utf8_lossy(&output.stdout)
    );
}

#[tokio::test]
async fn generated_ps1_dry_run_runs_without_writes() {
    let Some(powershell) = find_powershell() else {
        eprintln!("skipping: powershell is not available");
        return;
    };
    let (app, _state) = test_app().await;
    let token = login_token(&app).await;
    let (status, script) = call(&app, "GET", "/api/v1/generate.ps1", Some(&token)).await;
    assert_eq!(status, StatusCode::OK);
    let temp = TempDir::new().expect("temp dir");
    let path = temp.path().join("node.ps1");
    std::fs::write(&path, &script).expect("write ps1");
    let node_root = temp.path().join("node-root");
    let output = Command::new(&powershell)
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
        ])
        .arg(&path)
        .arg("-DryRun")
        .arg("-NodeRoot")
        .arg(&node_root)
        .env_remove("VPN233_DRY_RUN")
        .env_remove("VPN233_NODE_ROOT")
        .env_remove("VPN233_SKIP_DOWNLOAD")
        .env_remove("VPN233_SKIP_SERVICE")
        .output()
        .expect("powershell runs");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "ps1 dry run failed: stdout={stdout} stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(stdout.contains("DRY-RUN OK"));
    assert!(stdout.contains("generated mihomo config:"));
    assert!(!node_root.exists());
}

#[tokio::test]
async fn generated_sh_refuses_existing_node_before_any_mutation() {
    let Some(bash) = find_bash() else {
        return;
    };
    let (app, _state) = test_app().await;
    let token = login_token(&app).await;
    let (_, script) = call(&app, "GET", "/api/v1/generate.sh", Some(&token)).await;
    let temp = TempDir::new().unwrap();
    let root = temp.path().join("node");
    std::fs::create_dir_all(root.join("mihomo")).unwrap();
    std::fs::create_dir_all(root.join("bin")).unwrap();
    let path = root.join("mihomo/config.yaml");
    let original = b"# existing node\r\nmixed-port: 7890\r\npassword: change-me-preserved\r\n";
    std::fs::write(&path, original).unwrap();
    let binary = root.join("bin/mihomo");
    std::fs::write(&binary, b"existing-application").unwrap();
    let output = run_bash_script(
        &bash,
        &script,
        &[
            ("VPN233_NODE_ROOT", forward_slashes(&root)),
            ("VPN233_DRY_RUN", "0".into()),
            ("VPN233_SKIP_DOWNLOAD", "1".into()),
            ("VPN233_SKIP_SERVICE", "1".into()),
        ],
    )
    .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("node configuration already exists"));
    assert_eq!(std::fs::read(path).unwrap(), original);
    assert_eq!(std::fs::read(binary).unwrap(), b"existing-application");
    assert!(!root.join("mihomo/certs").exists());
    assert!(!String::from_utf8_lossy(&output.stdout).contains("vless://"));
}

#[tokio::test]
async fn generated_ps1_refuses_existing_node_before_any_mutation() {
    let Some(powershell) = find_powershell() else {
        return;
    };
    let (app, _state) = test_app().await;
    let token = login_token(&app).await;
    let (_, script) = call(&app, "GET", "/api/v1/generate.ps1", Some(&token)).await;
    let temp = TempDir::new().unwrap();
    let root = temp.path().join("node");
    std::fs::create_dir_all(root.join("mihomo")).unwrap();
    std::fs::create_dir_all(root.join("bin")).unwrap();
    let path = root.join("mihomo/config.yaml");
    let original = b"# existing node\r\nmixed-port: 7890\r\npassword: change-me-preserved\r\n";
    std::fs::write(&path, original).unwrap();
    let binary = root.join("bin/mihomo.exe");
    std::fs::write(&binary, b"existing-application").unwrap();
    let script_path = temp.path().join("node.ps1");
    std::fs::write(&script_path, script).unwrap();
    let output = Command::new(powershell)
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
        ])
        .arg(script_path)
        .arg("-NodeRoot")
        .arg(&root)
        .env_remove("VPN233_NODE_ROOT")
        .env("VPN233_DRY_RUN", "0")
        .env("VPN233_SKIP_DOWNLOAD", "1")
        .env("VPN233_SKIP_SERVICE", "1")
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("node configuration already exists"));
    assert_eq!(std::fs::read(path).unwrap(), original);
    assert_eq!(std::fs::read(binary).unwrap(), b"existing-application");
    assert!(!root.join("mihomo/certs").exists());
    assert!(!String::from_utf8_lossy(&output.stdout).contains("vless://"));
}

#[tokio::test]
async fn config_loads_from_file_and_env_overrides() {
    let _guard = ENV_LOCK.lock().await;
    clear_provider_env();
    let temp = TempDir::new().expect("temp dir");
    let config_path = temp.path().join("config.yaml");
    std::fs::write(
        &config_path,
        r#"
listen_addr: "127.0.0.1"
listen_port: 18080
admin_user: "file-admin"
admin_password: "file-password"
default_node_ip: "203.0.113.20"
default_port_base: 12000
enabled_protocols: ["vless"]
download_mirrors: ["https://ghproxy.net/", ""]
"#,
    )
    .expect("write config");
    std::env::set_var("VPN233_ADMIN_PASSWORD", "env-password");
    std::env::set_var("VPN233_DEFAULT_PORT_BASE", "13000");
    std::env::set_var("VPN233_ENABLED_PROTOCOLS", "vless,ss");
    let config = Config::load(&config_path).await.expect("config loads");
    assert_eq!(config.listen_port, 18080);
    assert_eq!(config.admin_user, "file-admin");
    assert_eq!(config.admin_password, "env-password");
    assert_eq!(config.default_port_base, 13000);
    assert_eq!(config.enabled_protocols, vec!["vless", "ss"]);
    assert_eq!(config.mirrors().last().map(String::as_str), Some(""));
    clear_provider_env();
}

#[tokio::test]
async fn admin_password_file_takes_precedence() {
    let _guard = ENV_LOCK.lock().await;
    clear_provider_env();
    let temp = TempDir::new().expect("temp dir");
    let password_file = temp.path().join("admin-password.txt");
    std::fs::write(&password_file, "file-based-password\n").expect("write password file");
    std::env::set_var("VPN233_ADMIN_PASSWORD", "env-password");
    std::env::set_var("VPN233_ADMIN_PASSWORD_FILE", &password_file);
    let config = Config::load(&temp.path().join("missing.yaml"))
        .await
        .expect("default config loads");
    assert_eq!(config.admin_password, "file-based-password");
    clear_provider_env();
}

#[tokio::test]
async fn config_rejects_invalid_ttl_and_listen_address() {
    let config = Config {
        session_ttl_secs: 0,
        ..test_config()
    };
    assert!(config.validate().is_err());
    let config = Config {
        listen_addr: "not an address".into(),
        ..test_config()
    };
    assert!(config.validate().is_err());
}

// ---------------------------------------------------------------------------
// Registry heartbeat E2E against an in-test axum mock registry.
// ---------------------------------------------------------------------------

#[derive(Clone, Default)]
struct MockRegistry {
    registrations: Arc<AtomicUsize>,
    heartbeats: Arc<AtomicUsize>,
    reject_heartbeat: Arc<AtomicBool>,
    last_heartbeat_token: Arc<Mutex<Option<String>>>,
    auth_headers: Arc<Mutex<Vec<String>>>,
    link_counts: Arc<Mutex<Vec<usize>>>,
}

#[derive(Debug, Deserialize)]
struct RegisterPayload {
    provider_id: String,
    node_links: Vec<String>,
    heartbeat_interval_secs: u64,
}

#[derive(Debug, Deserialize)]
struct HeartbeatPayload {
    provider_id: String,
    heartbeat_token: String,
}

async fn spawn_mock_registry() -> (String, MockRegistry) {
    let mock = MockRegistry::default();
    let app = Router::new()
        .route("/api/v1/registry/register", post(mock_register))
        .route("/api/v1/registry/heartbeat", post(mock_heartbeat))
        .with_state(mock.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind mock registry");
    let address = listener.local_addr().expect("mock registry address");
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    (format!("http://{address}"), mock)
}

async fn mock_register(
    axum::extract::State(mock): axum::extract::State<MockRegistry>,
    headers: HeaderMap,
    Json(payload): Json<RegisterPayload>,
) -> Json<Value> {
    assert_eq!(payload.provider_id, "provider-unit-test");
    assert_eq!(payload.heartbeat_interval_secs, 10);
    assert_eq!(
        headers
            .get("x-provider-registry-token")
            .and_then(|value| value.to_str().ok()),
        Some("unit-test-registry-token")
    );
    let count = mock.registrations.fetch_add(1, Ordering::SeqCst) + 1;
    mock.auth_headers.lock().expect("lock").push(
        headers
            .get("x-provider-registry-token")
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_owned(),
    );
    mock.link_counts
        .lock()
        .expect("lock")
        .push(payload.node_links.len());
    Json(json!({
        "heartbeat_token": format!("heartbeat-token-{count}"),
        "heartbeat_interval_secs": 10,
    }))
}

async fn mock_heartbeat(
    axum::extract::State(mock): axum::extract::State<MockRegistry>,
    headers: HeaderMap,
    Json(payload): Json<HeartbeatPayload>,
) -> Response {
    assert_eq!(payload.provider_id, "provider-unit-test");
    mock.auth_headers.lock().expect("lock").push(
        headers
            .get("x-provider-registry-token")
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_owned(),
    );
    if mock.reject_heartbeat.load(Ordering::SeqCst) {
        return (StatusCode::INTERNAL_SERVER_ERROR, "heartbeat rejected").into_response();
    }
    assert!(payload.heartbeat_token.starts_with("heartbeat-token-"));
    mock.heartbeats.fetch_add(1, Ordering::SeqCst);
    *mock.last_heartbeat_token.lock().expect("lock") = Some(payload.heartbeat_token);
    Json(json!({"ok": true})).into_response()
}

#[tokio::test]
async fn registry_registers_heartbeats_and_reregisters_after_rejection() {
    let (registry_url, mock) = spawn_mock_registry().await;
    let config = Config {
        registry_url,
        registry_token: "unit-test-registry-token".into(),
        registry_provider_id: "provider-unit-test".into(),
        heartbeat_interval_secs: 10,
        ..test_config()
    };
    let mut reporter = RegistryReporter::new(&config)
        .expect("reporter builds")
        .expect("reporter enabled");
    assert!(reporter.heartbeat_token().is_none());
    assert_eq!(reporter.interval_secs(), 10);

    let step = reporter.step().await.expect("register step");
    assert!(matches!(step, RegistryStep::Registered { .. }));
    assert_eq!(mock.registrations.load(Ordering::SeqCst), 1);
    let token = reporter.heartbeat_token().expect("token set").to_owned();
    assert_eq!(token, "heartbeat-token-1");

    let step = reporter.step().await.expect("heartbeat step");
    assert_eq!(step, RegistryStep::Heartbeat);
    assert_eq!(mock.heartbeats.load(Ordering::SeqCst), 1);
    assert_eq!(
        mock.last_heartbeat_token.lock().expect("lock").as_deref(),
        Some("heartbeat-token-1")
    );

    mock.reject_heartbeat.store(true, Ordering::SeqCst);
    let step = reporter.step().await.expect("rejected heartbeat step");
    assert_eq!(step, RegistryStep::Rejected);
    assert!(reporter.heartbeat_token().is_none());

    mock.reject_heartbeat.store(false, Ordering::SeqCst);
    let step = reporter.step().await.expect("second register step");
    assert!(matches!(step, RegistryStep::Registered { .. }));
    assert_eq!(mock.registrations.load(Ordering::SeqCst), 2);
    assert_eq!(reporter.heartbeat_token(), Some("heartbeat-token-2"));

    // Auto-generated node links are registered even without explicit config.
    assert!(mock
        .link_counts
        .lock()
        .expect("lock")
        .iter()
        .all(|count| *count > 0));
    // Every request carried the configured registry token.
    assert!(mock
        .auth_headers
        .lock()
        .expect("lock")
        .iter()
        .all(|token| token == "unit-test-registry-token"));
}

#[tokio::test]
async fn registry_reporter_is_disabled_without_url() {
    let config = test_config();
    assert!(RegistryReporter::new(&config).expect("no error").is_none());
    let config = Config {
        registry_url: "http://127.0.0.1:1".into(),
        registry_token: String::new(),
        ..test_config()
    };
    assert!(RegistryReporter::new(&config).is_err());
}

// ---------------------------------------------------------------------------
// Helpers.
// ---------------------------------------------------------------------------

fn clear_provider_env() {
    for key in [
        "VPN233_ADMIN_USER",
        "VPN233_ADMIN_PASSWORD",
        "VPN233_ADMIN_PASSWORD_FILE",
        "VPN233_PROVIDER_ACCESS_TOKEN",
        "VPN233_DEFAULT_PORT_BASE",
        "VPN233_ENABLED_PROTOCOLS",
        "VPN233_DOWNLOAD_MIRRORS",
        "VPN233_NODE_ROOT",
        "VPN233_SESSION_TTL_SECS",
        "VPN233_LOGIN_RATE_LIMIT",
        "VPN233_AUTO_DETECT_PUBLIC_IP",
    ] {
        std::env::remove_var(key);
    }
}

fn find_bash() -> Option<String> {
    if let Ok(path) = std::env::var("VPN233_BASH") {
        if !path.trim().is_empty() && Command::new(&path).arg("--version").output().is_ok() {
            return Some(path);
        }
    }
    let candidates = [
        r"C:\Program Files\Git\bin\bash.exe",
        r"C:\Program Files\Git\usr\bin\bash.exe",
        "bash",
    ];
    for candidate in candidates {
        if Command::new(candidate)
            .arg("--version")
            .output()
            .is_ok_and(|output| output.status.success())
        {
            return Some(candidate.to_owned());
        }
    }
    None
}

fn find_powershell() -> Option<String> {
    for candidate in ["powershell", "pwsh"] {
        if Command::new(candidate)
            .args([
                "-NoProfile",
                "-Command",
                "$PSVersionTable.PSVersion.ToString()",
            ])
            .output()
            .is_ok_and(|output| output.status.success())
        {
            return Some(candidate.to_owned());
        }
    }
    None
}

fn forward_slashes(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn run_bash(bash: &str, args: &[&str], script: &str) -> std::io::Result<Output> {
    let mut command = Command::new(bash);
    command
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn()?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(script.as_bytes())?;
    }
    child.wait_with_output()
}

fn run_bash_script(
    bash: &str,
    script: &str,
    environment: &[(&str, String)],
) -> std::io::Result<Output> {
    let mut command = Command::new(bash);
    command
        .arg("-s")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (key, value) in environment {
        command.env(key, value);
    }
    let mut child = command.spawn()?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(script.as_bytes())?;
    }
    child.wait_with_output()
}
