//! Loopback-only proxy tests: no public endpoints or actual subscription data.

use axum::{
    body::Body,
    http::{header, Request, StatusCode},
};
use http_body_util::BodyExt;
use std::{sync::Arc, time::Duration};
use tempfile::TempDir;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    task::JoinHandle,
    time::timeout,
};
use tower::ServiceExt;
use vpn233_client::{auth, build_app, http_proxy, socks, AppState, Config};

async fn state(configure: impl FnOnce(&mut Config)) -> (TempDir, Arc<AppState>) {
    let dir = tempfile::tempdir().unwrap();
    let mut config = Config {
        stats_file: dir.path().join("stats.json"),
        ..Config::default()
    };
    configure(&mut config);
    let state = Arc::new(AppState::new(config).await.unwrap());
    (dir, state)
}

async fn proxy(state: Arc<AppState>, http: bool) -> (TcpStream, JoinHandle<anyhow::Result<()>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let task = tokio::spawn(async move {
        let (stream, peer) = listener.accept().await.unwrap();
        if http {
            http_proxy::handle_http(stream, peer, state).await
        } else {
            socks::handle_socks(stream, peer, state).await
        }
    });
    (TcpStream::connect(address).await.unwrap(), task)
}

async fn echo() -> (u16, JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let task = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut bytes = Vec::new();
        stream.read_to_end(&mut bytes).await.unwrap();
        stream.write_all(&bytes).await.unwrap();
        stream.shutdown().await.unwrap();
    });
    (port, task)
}

async fn socks_request(stream: &mut TcpStream, port: u16) {
    let mut request = vec![5, 1, 0, 1, 127, 0, 0, 1];
    request.extend_from_slice(&port.to_be_bytes());
    stream.write_all(&request).await.unwrap();
    let mut reply = [0; 10];
    stream.read_exact(&mut reply).await.unwrap();
    assert_eq!(&reply[..3], &[5, 0, 0]);
}

#[tokio::test]
async fn socks5_authentication_and_half_close_transfer_work_without_domain_history() {
    timeout(Duration::from_secs(5), async {
        let (_dir, state) = state(|config| {
            config.socks_user = "user".into();
            config.socks_password = "change-me".into();
        })
        .await;
        let (port, echo_task) = echo().await;
        let (mut client, task) = proxy(state.clone(), false).await;
        client.write_all(&[5, 1, 2]).await.unwrap();
        let mut reply = [0; 2];
        client.read_exact(&mut reply).await.unwrap();
        assert_eq!(reply, [5, 2]);
        client
            .write_all(b"\x01\x04user\x09change-me")
            .await
            .unwrap();
        client.read_exact(&mut reply).await.unwrap();
        assert_eq!(reply, [1, 0]);
        socks_request(&mut client, port).await;
        client.write_all(b"loopback payload").await.unwrap();
        client.shutdown().await.unwrap();
        let mut response = Vec::new();
        client.read_to_end(&mut response).await.unwrap();
        assert_eq!(response, b"loopback payload");
        task.await.unwrap().unwrap();
        echo_task.await.unwrap();
        let stats = state.stats_snapshot().await;
        assert_eq!(stats.bytes_up, 16);
        assert_eq!(stats.bytes_down, 16);
        assert_eq!(stats.active_connections, 0);
        assert!(stats.domains.is_empty());
        assert!(!tokio::fs::read_to_string(state.stats_file())
            .await
            .unwrap()
            .contains("127.0.0.1"));
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn socks5_rejects_wrong_authentication_before_target_access() {
    timeout(Duration::from_secs(5), async {
        let (_dir, state) = state(|config| {
            config.socks_user = "user".into();
            config.socks_password = "change-me".into();
        })
        .await;
        let (mut client, task) = proxy(state.clone(), false).await;
        client.write_all(b"\x05\x01\x02").await.unwrap();
        let mut reply = [0; 2];
        client.read_exact(&mut reply).await.unwrap();
        client.write_all(b"\x01\x04user\x05wrong").await.unwrap();
        client.read_exact(&mut reply).await.unwrap();
        assert_eq!(reply, [1, 1]);
        assert!(task.await.unwrap().is_err());
        assert_eq!(state.stats_snapshot().await.active_connections, 0);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn http_connect_preserves_tunnel_bytes_sent_with_headers() {
    timeout(Duration::from_secs(5), async {
        let (_dir, state) = state(|_| {}).await;
        let (port, echo_task) = echo().await;
        let (mut client, task) = proxy(state, true).await;
        client.write_all(format!("CONNECT 127.0.0.1:{port} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n\r\npipelined payload").as_bytes()).await.unwrap();
        client.shutdown().await.unwrap();
        let mut response = Vec::new();
        client.read_to_end(&mut response).await.unwrap();
        assert!(response.starts_with(b"HTTP/1.1 200 Connection Established\r\n\r\n"));
        assert!(response.ends_with(b"pipelined payload"));
        task.await.unwrap().unwrap();
        echo_task.await.unwrap();
    }).await.unwrap();
}

#[tokio::test]
async fn http_proxy_challenges_and_does_not_forward_proxy_credentials() {
    timeout(Duration::from_secs(5), async {
        let (_dir, state) = state(|config| { config.http_user = "user".into(); config.http_password = "change-me".into(); }).await;
        let (mut client, task) = proxy(state.clone(), true).await;
        client.write_all(b"GET http://example.invalid/ HTTP/1.1\r\n\r\n").await.unwrap();
        let mut response = String::new();
        client.read_to_string(&mut response).await.unwrap();
        assert!(response.starts_with("HTTP/1.1 407"));
        task.await.unwrap().unwrap();
        let origin = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = origin.local_addr().unwrap().port();
        let origin_task = tokio::spawn(async move {
            let (mut stream, _) = origin.accept().await.unwrap();
            let mut request = String::new();
            stream.read_to_string(&mut request).await.unwrap();
            assert!(request.starts_with("GET /path?query=1 HTTP/1.1\r\n"));
            assert!(!request.to_ascii_lowercase().contains("proxy-authorization"));
            assert!(!request.contains("change-me"));
            stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok").await.unwrap();
        });
        let (mut client, task) = proxy(state, true).await;
        let auth = auth::encode_basic("user", "change-me");
        client.write_all(format!("GET http://127.0.0.1:{port}/path?query=1 HTTP/1.1\r\nProxy-Authorization: {auth}\r\n\r\n").as_bytes()).await.unwrap();
        client.shutdown().await.unwrap();
        let mut response = String::new();
        client.read_to_string(&mut response).await.unwrap();
        assert!(response.ends_with("ok"));
        task.await.unwrap().unwrap();
        origin_task.await.unwrap();
    }).await.unwrap();
}

#[tokio::test]
async fn management_config_is_authenticated_and_redacted() {
    let (_dir, state) = state(|config| {
        config.admin_password = "private-admin-value".into();
        config.upstream_socks5 = "user:private-upstream-value@example.invalid".into();
        config.subscription_url = "https://example.invalid/private-subscription-value".into();
    })
    .await;
    let app = build_app(state);
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/config")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/config")
                .header(
                    header::AUTHORIZATION,
                    auth::encode_basic("root", "private-admin-value"),
                )
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers()[header::CACHE_CONTROL],
        "no-store, private"
    );
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    assert!(!String::from_utf8_lossy(&bytes).contains("private-"));
    let response = app
        .oneshot(
            Request::builder()
                .uri("/proxy.pac")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert!(response.headers()[header::CONTENT_TYPE]
        .to_str()
        .unwrap()
        .contains("application/x-ns-proxy-autoconfig"));
}

#[tokio::test]
async fn opting_out_hides_preexisting_domain_history_and_preserves_aggregate_totals() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("stats.json");
    tokio::fs::write(&path, r#"{"active_connections":3,"total_connections":10,"bytes_up":42,"bytes_down":99,"domains":{"private.example.invalid":{"requests":1,"upload_bytes":1,"download_bytes":2,"last_seen":1}}}"#).await.unwrap();
    let state = AppState::new(Config {
        stats_file: path.clone(),
        ..Config::default()
    })
    .await
    .unwrap();
    let stats = state.stats_snapshot().await;
    assert_eq!(stats.bytes_up, 42);
    assert_eq!(stats.bytes_down, 99);
    assert!(stats.domains.is_empty());
    assert_eq!(stats.active_connections, 0);
    state.flush().await.unwrap();
    assert!(!tokio::fs::read_to_string(path)
        .await
        .unwrap()
        .contains("private.example.invalid"));
}

#[tokio::test]
async fn authenticated_socks5_upstream_forwards_binary_payload() {
    timeout(Duration::from_secs(5), async {
        let (_dir, upstream_state) = state(|config| {
            config.socks_user = "user".into();
            config.socks_password = "change-me".into();
        })
        .await;
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let upstream_port = listener.local_addr().unwrap().port();
        let upstream_task = tokio::spawn(async move {
            let (stream, peer) = listener.accept().await.unwrap();
            socks::handle_socks(stream, peer, upstream_state)
                .await
                .unwrap();
        });
        let upstream = vpn233_client::upstream::parse_upstream(&format!(
            "socks5://user:change-me@127.0.0.1:{upstream_port}"
        ))
        .unwrap();
        let (port, echo_task) = echo().await;
        let mut connection =
            vpn233_client::upstream::connect_via_upstream(&upstream, "127.0.0.1", port)
                .await
                .unwrap();
        connection.write_all(b"\x00\x01\xffpayload").await.unwrap();
        connection.shutdown().await.unwrap();
        let mut reply = Vec::new();
        connection.read_to_end(&mut reply).await.unwrap();
        assert_eq!(reply, b"\x00\x01\xffpayload");
        upstream_task.await.unwrap();
        echo_task.await.unwrap();
    })
    .await
    .unwrap();
}
