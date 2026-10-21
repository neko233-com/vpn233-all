//! HTTP integration tests for the subscription server.
//!
//! Every test builds its own app from an isolated temporary data directory and
//! drives the router through `tower::ServiceExt::oneshot`, so no TCP port or
//! real runtime data is touched.

use std::path::Path;

use axum::{
    body::Body,
    http::{header, HeaderMap, HeaderValue, Method, Request, StatusCode},
    Router,
};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use tempfile::TempDir;
use tower::ServiceExt;
use vpn233_subscribe_server::{build_app, init_state, Config, ServerConfig, SubscriptionSource};

const ADMIN_USER: &str = "root";
const ADMIN_PASSWORD: &str = "test-admin-password";
const REGISTRY_TOKEN: &str = "test-registry-token";

const VLESS_LINK: &str = "vless://11111111-1111-1111-1111-111111111111@example.invalid:443?security=tls&type=tcp&sni=example.invalid#edge-node";
const TROJAN_LINK: &str = "trojan://fake-password@example.invalid:443#trojan-node";
const SS_LINK: &str = "ss://YWVzLTI1Ni1nY206cGFzc3dvcmQ=@example.invalid:8388#ss-node";

fn vmess_link() -> String {
    let payload = json!({
        "v": "2",
        "ps": "vmess-node",
        "add": "example.invalid",
        "port": "443",
        "id": "11111111-1111-1111-1111-111111111111",
        "scy": "auto",
    });
    format!("vmess://{}", BASE64.encode(payload.to_string()))
}

fn all_sample_links() -> Vec<String> {
    vec![
        VLESS_LINK.to_owned(),
        TROJAN_LINK.to_owned(),
        SS_LINK.to_owned(),
        vmess_link(),
    ]
}

struct TestServer {
    app: Router,
    _dir: Option<TempDir>,
}

impl TestServer {
    async fn new() -> Self {
        let dir = tempfile::tempdir().expect("temp dir");
        let mut server = Self::with_dir(dir.path(), |_| {}).await;
        server._dir = Some(dir);
        server
    }

    async fn with(configure: impl FnOnce(&mut Config)) -> Self {
        let dir = tempfile::tempdir().expect("temp dir");
        let mut server = Self::with_dir(dir.path(), configure).await;
        server._dir = Some(dir);
        server
    }

    async fn with_links(links: &[String]) -> Self {
        let links = links.to_vec();
        Self::with(move |config| {
            config.subscriptions = vec![SubscriptionSource {
                name: "alpha".into(),
                links,
                url: String::new(),
                access_token: String::new(),
            }];
        })
        .await
    }

    async fn with_dir(dir: &Path, configure: impl FnOnce(&mut Config)) -> Self {
        let mut config = Config {
            server: ServerConfig {
                host: "127.0.0.1".into(),
                http_port: 0,
                default_format: "clash".into(),
                data_dir: dir.to_string_lossy().into_owned(),
                admin_user: ADMIN_USER.into(),
                admin_password: ADMIN_PASSWORD.into(),
                admin_origins: String::new(),
                web_dir: dir.join("web").to_string_lossy().into_owned(),
                provider_registry_token: REGISTRY_TOKEN.into(),
                provider_stale_after_secs: 120,
                client_stats_url: String::new(),
                client_stats_user: String::new(),
                client_stats_password: String::new(),
                traffic_timezone_offset_hours: 8,
            },
            subscriptions: Vec::new(),
        };
        configure(&mut config);
        let state = init_state(config.clone()).await.expect("init_state");
        Self {
            app: build_app(state),
            _dir: None,
        }
    }

    async fn send(&self, request: Request<Body>) -> (StatusCode, HeaderMap, Vec<u8>) {
        let response = self
            .app
            .clone()
            .oneshot(request)
            .await
            .expect("router must respond");
        let status = response.status();
        let headers = response.headers().clone();
        let body = response
            .into_body()
            .collect()
            .await
            .expect("response body")
            .to_bytes()
            .to_vec();
        (status, headers, body)
    }

    async fn get(&self, uri: &str) -> (StatusCode, HeaderMap, Vec<u8>) {
        self.get_with_ip(uri, None).await
    }

    async fn get_with_ip(&self, uri: &str, ip: Option<&str>) -> (StatusCode, HeaderMap, Vec<u8>) {
        let mut builder = Request::builder().method(Method::GET).uri(uri);
        if let Some(ip) = ip {
            builder = builder.header("x-forwarded-for", ip);
        }
        self.send(builder.body(Body::empty()).expect("request"))
            .await
    }

    async fn get_basic(
        &self,
        uri: &str,
        token: &str,
        password: &str,
    ) -> (StatusCode, HeaderMap, Vec<u8>) {
        self.send(
            Request::builder()
                .method(Method::GET)
                .uri(uri)
                .header(header::AUTHORIZATION, basic_header(token, password))
                .body(Body::empty())
                .expect("request"),
        )
        .await
    }

    async fn login(
        &self,
        username: &str,
        password: &str,
        ip: Option<&str>,
    ) -> (StatusCode, HeaderMap, Vec<u8>) {
        let mut builder = Request::builder()
            .method(Method::POST)
            .uri("/api/auth/login")
            .header(header::CONTENT_TYPE, "application/json");
        if let Some(ip) = ip {
            builder = builder.header("x-forwarded-for", ip);
        }
        self.send(
            builder
                .body(Body::from(
                    json!({"username": username, "password": password}).to_string(),
                ))
                .expect("request"),
        )
        .await
    }

    async fn admin_token(&self) -> String {
        let (status, _, body) = self.login(ADMIN_USER, ADMIN_PASSWORD, None).await;
        assert_eq!(status, StatusCode::OK, "admin login should succeed");
        json_body(&body)["token"]
            .as_str()
            .expect("login response has token")
            .to_owned()
    }

    async fn admin_send(
        &self,
        method: Method,
        uri: &str,
        bearer: &str,
        body: Option<&Value>,
    ) -> (StatusCode, HeaderMap, Vec<u8>) {
        let mut builder = Request::builder()
            .method(method)
            .uri(uri)
            .header(header::AUTHORIZATION, format!("Bearer {bearer}"));
        let body = match body {
            Some(value) => {
                builder = builder.header(header::CONTENT_TYPE, "application/json");
                Body::from(value.to_string())
            }
            None => Body::empty(),
        };
        self.send(builder.body(body).expect("request")).await
    }

    async fn admin_get(&self, uri: &str, bearer: &str) -> (StatusCode, HeaderMap, Vec<u8>) {
        self.admin_send(Method::GET, uri, bearer, None).await
    }

    async fn admin_post(
        &self,
        uri: &str,
        bearer: &str,
        body: &Value,
    ) -> (StatusCode, HeaderMap, Vec<u8>) {
        self.admin_send(Method::POST, uri, bearer, Some(body)).await
    }

    async fn admin_patch(
        &self,
        uri: &str,
        bearer: &str,
        body: &Value,
    ) -> (StatusCode, HeaderMap, Vec<u8>) {
        self.admin_send(Method::PATCH, uri, bearer, Some(body))
            .await
    }

    async fn admin_delete(&self, uri: &str, bearer: &str) -> (StatusCode, HeaderMap, Vec<u8>) {
        self.admin_send(Method::DELETE, uri, bearer, None).await
    }

    async fn create_token(
        &self,
        bearer: &str,
        token: &str,
        password: &str,
        limit_bytes: u64,
    ) -> StatusCode {
        let (status, _, _) = self
            .admin_post(
                "/api/admin/tokens",
                bearer,
                &json!({
                    "token": token,
                    "password": password,
                    "limit_bytes": limit_bytes,
                }),
            )
            .await;
        status
    }

    async fn token_list(&self, bearer: &str) -> Vec<Value> {
        let (status, _, body) = self.admin_get("/api/admin/tokens", bearer).await;
        assert_eq!(status, StatusCode::OK);
        json_body(&body)["tokens"]
            .as_array()
            .cloned()
            .unwrap_or_default()
    }

    async fn register_provider(&self, provider_id: &str, links: &[String]) -> String {
        let (status, _, body) = self
            .send(
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/v1/registry/register")
                    .header("x-provider-registry-token", REGISTRY_TOKEN)
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        json!({
                            "provider_id": provider_id,
                            "name": format!("provider-{provider_id}"),
                            "version": "1.0.0",
                            "node_links": links,
                        })
                        .to_string(),
                    ))
                    .expect("request"),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "register should succeed");
        json_body(&body)["heartbeat_token"]
            .as_str()
            .expect("heartbeat token")
            .to_owned()
    }
}

fn basic_header(token: &str, password: &str) -> HeaderValue {
    HeaderValue::from_str(&format!(
        "Basic {}",
        BASE64.encode(format!("{token}:{password}"))
    ))
    .expect("valid basic header")
}

fn json_body(body: &[u8]) -> Value {
    serde_json::from_slice(body).expect("response body should be JSON")
}

fn body_text(body: &[u8]) -> String {
    String::from_utf8(body.to_vec()).expect("response body should be UTF-8")
}

fn find_token<'a>(tokens: &'a [Value], token: &str) -> &'a Value {
    tokens
        .iter()
        .find(|item| item["token"] == token)
        .expect("token should exist")
}

// ---------------------------------------------------------------------------
// Admin authentication
// ---------------------------------------------------------------------------

#[tokio::test]
async fn status_requires_authentication() {
    let server = TestServer::new().await;
    let (status, _, _) = server.get("/api/status").await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn status_returns_service_info_with_basic_auth() {
    let server = TestServer::new().await;
    let (status, _, body) = server
        .get_basic("/api/status", ADMIN_USER, ADMIN_PASSWORD)
        .await;
    assert_eq!(status, StatusCode::OK);
    let value = json_body(&body);
    assert_eq!(value["ok"], true);
    assert_eq!(value["service"], "vpn233-subscribe-server");
    assert!(value["version"].as_str().is_some_and(|v| !v.is_empty()));
}

#[tokio::test]
async fn status_accepts_admin_jwt_bearer() {
    let server = TestServer::new().await;
    let token = server.admin_token().await;
    let (status, _, body) = server.admin_get("/api/status", &token).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json_body(&body)["ok"], true);
}

#[tokio::test]
async fn admin_login_returns_jwt_and_expiry() {
    let server = TestServer::new().await;
    let (status, _, body) = server.login(ADMIN_USER, ADMIN_PASSWORD, None).await;
    assert_eq!(status, StatusCode::OK);
    let value = json_body(&body);
    assert!(!value["token"].as_str().unwrap_or_default().is_empty());
    assert_eq!(value["user"], ADMIN_USER);
    assert!(value["expires_at"].as_u64().unwrap_or_default() > 0);
}

#[tokio::test]
async fn admin_login_rejects_wrong_password() {
    let server = TestServer::new().await;
    let (status, _, _) = server.login(ADMIN_USER, "wrong-password", None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn admin_login_rejects_wrong_username() {
    let server = TestServer::new().await;
    let (status, _, _) = server.login("intruder", ADMIN_PASSWORD, None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn admin_login_rate_limits_after_repeated_failures() {
    let server = TestServer::new().await;
    let ip = "203.0.113.10";
    for attempt in 1..=10 {
        let (status, _, _) = server.login(ADMIN_USER, "bad", Some(ip)).await;
        assert_eq!(
            status,
            StatusCode::UNAUTHORIZED,
            "attempt {attempt} should be a plain 401"
        );
    }
    let (status, headers, _) = server.login(ADMIN_USER, "bad", Some(ip)).await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    assert!(headers.get(header::RETRY_AFTER).is_some());
    // A different client IP is still allowed to log in.
    let (status, _, _) = server
        .login(ADMIN_USER, ADMIN_PASSWORD, Some("203.0.113.11"))
        .await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn admin_login_success_resets_failure_counter() {
    let server = TestServer::new().await;
    let ip = "203.0.113.12";
    for _ in 0..9 {
        let (status, _, _) = server.login(ADMIN_USER, "bad", Some(ip)).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }
    let (status, _, _) = server.login(ADMIN_USER, ADMIN_PASSWORD, Some(ip)).await;
    assert_eq!(status, StatusCode::OK);
    for _ in 0..9 {
        let (status, _, _) = server.login(ADMIN_USER, "bad", Some(ip)).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }
}

#[derive(serde::Serialize)]
struct TestClaims {
    sub: String,
    iat: u64,
    exp: u64,
}

#[tokio::test]
async fn expired_admin_jwt_is_rejected() {
    let server = TestServer::new().await;
    let secret = format!("vpn233-admin-jwt-v1:{ADMIN_USER}:{ADMIN_PASSWORD}");
    let claims = TestClaims {
        sub: ADMIN_USER.into(),
        iat: 1_600_000_000,
        exp: 1_600_000_001,
    };
    let token = jsonwebtoken::encode(
        &jsonwebtoken::Header::new(jsonwebtoken::Algorithm::HS256),
        &claims,
        &jsonwebtoken::EncodingKey::from_secret(secret.as_bytes()),
    )
    .expect("test JWT");
    let (status, _, _) = server.admin_get("/api/status", &token).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn admin_jwt_with_wrong_secret_is_rejected() {
    let server = TestServer::new().await;
    let claims = TestClaims {
        sub: ADMIN_USER.into(),
        iat: 1_600_000_000,
        exp: 4_000_000_000,
    };
    let token = jsonwebtoken::encode(
        &jsonwebtoken::Header::new(jsonwebtoken::Algorithm::HS256),
        &claims,
        &jsonwebtoken::EncodingKey::from_secret(b"not-the-real-secret"),
    )
    .expect("test JWT");
    let (status, _, _) = server.admin_get("/api/status", &token).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

// ---------------------------------------------------------------------------
// Nodes and admin token management
// ---------------------------------------------------------------------------

#[tokio::test]
async fn nodes_requires_authentication() {
    let server = TestServer::with_links(&all_sample_links()).await;
    let (status, _, _) = server.get("/api/nodes").await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn nodes_returns_parsed_nodes() {
    let server = TestServer::with_links(&all_sample_links()).await;
    let token = server.admin_token().await;
    let (status, _, body) = server.admin_get("/api/nodes", &token).await;
    assert_eq!(status, StatusCode::OK);
    let nodes = json_body(&body);
    let nodes = nodes.as_array().expect("node array");
    assert_eq!(nodes.len(), 4);
    let protocols = nodes
        .iter()
        .filter_map(|node| node["protocol"].as_str())
        .collect::<Vec<_>>();
    for expected in ["vless", "vmess", "trojan", "ss"] {
        assert!(protocols.contains(&expected), "missing {expected}");
    }
}

#[tokio::test]
async fn refresh_returns_node_count() {
    let server = TestServer::with_links(&all_sample_links()).await;
    let token = server.admin_token().await;
    let (status, _, body) = server.admin_get("/api/refresh", &token).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json_body(&body)["nodes"], 4);
}

#[tokio::test]
async fn create_token_requires_admin_auth() {
    let server = TestServer::new().await;
    let (status, _, _) = server
        .send(
            Request::builder()
                .method(Method::POST)
                .uri("/api/admin/tokens")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({"token": "t", "password": "p", "limit_bytes": 10}).to_string(),
                ))
                .expect("request"),
        )
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn create_token_returns_created_and_lists() {
    let server = TestServer::new().await;
    let admin = server.admin_token().await;
    assert_eq!(
        server
            .create_token(&admin, "token-a", "password-a", 1024)
            .await,
        StatusCode::CREATED
    );
    let tokens = server.token_list(&admin).await;
    let token = find_token(&tokens, "token-a");
    assert_eq!(token["limit_bytes"], 1024);
    assert_eq!(token["enabled"], true);
    assert_eq!(token["quota_period"], "lifetime");
    assert_eq!(token["account_id"], "legacy");
    assert_eq!(token["group_id"], "all");
}

#[tokio::test]
async fn create_token_conflicts_for_duplicate() {
    let server = TestServer::new().await;
    let admin = server.admin_token().await;
    assert_eq!(
        server.create_token(&admin, "dup", "pw", 10).await,
        StatusCode::CREATED
    );
    assert_eq!(
        server.create_token(&admin, "dup", "pw", 10).await,
        StatusCode::CONFLICT
    );
}

#[tokio::test]
async fn create_token_validates_required_fields() {
    let server = TestServer::new().await;
    let admin = server.admin_token().await;
    let (status, _, _) = server
        .admin_post(
            "/api/admin/tokens",
            &admin,
            &json!({"token": "", "password": "pw", "limit_bytes": 10}),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _, _) = server
        .admin_post(
            "/api/admin/tokens",
            &admin,
            &json!({"token": "zero", "password": "pw", "limit_bytes": 0}),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn create_token_rejects_unknown_group() {
    let server = TestServer::new().await;
    let admin = server.admin_token().await;
    let (status, _, _) = server
        .admin_post(
            "/api/admin/tokens",
            &admin,
            &json!({"token": "t", "password": "p", "limit_bytes": 10, "group_id": "missing"}),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn disable_token_blocks_subscription() {
    let server = TestServer::with_links(&all_sample_links()).await;
    let admin = server.admin_token().await;
    server.create_token(&admin, "tok", "pw", 10_000).await;
    let (status, headers, _) = server.get_basic("/sub/c", "tok", "pw").await;
    assert_eq!(status, StatusCode::OK);
    assert!(headers.contains_key(header::CONTENT_TYPE));
    let (status, _, _) = server
        .admin_post("/api/admin/tokens/tok", &admin, &json!({}))
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, _, _) = server.get_basic("/sub/c", "tok", "pw").await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn delete_token_removes_it() {
    let server = TestServer::new().await;
    let admin = server.admin_token().await;
    server.create_token(&admin, "gone", "pw", 10).await;
    let (status, _, _) = server.admin_delete("/api/admin/tokens/gone", &admin).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let tokens = server.token_list(&admin).await;
    assert!(tokens.iter().all(|token| token["token"] != "gone"));
    let (status, _, _) = server.admin_delete("/api/admin/tokens/gone", &admin).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn rotate_token_changes_password() {
    let server = TestServer::with_links(&all_sample_links()).await;
    let admin = server.admin_token().await;
    server
        .create_token(&admin, "rotate-me", "old-password", 10_000)
        .await;
    let (status, _, body) = server
        .admin_post("/api/admin/tokens/rotate-me/rotate", &admin, &json!({}))
        .await;
    assert_eq!(status, StatusCode::OK);
    let new_password = json_body(&body)["password"]
        .as_str()
        .expect("new password")
        .to_owned();
    assert_ne!(new_password, "old-password");
    let (status, _, _) = server
        .get_basic("/sub/c", "rotate-me", "old-password")
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, _, _) = server.get_basic("/sub/c", "rotate-me", &new_password).await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn renew_token_requires_expiry_fields() {
    let server = TestServer::new().await;
    let admin = server.admin_token().await;
    server.create_token(&admin, "renew", "pw", 100).await;
    let (status, _, _) = server
        .admin_post("/api/admin/tokens/renew/renew", &admin, &json!({}))
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn renew_token_sets_expiry() {
    let server = TestServer::new().await;
    let admin = server.admin_token().await;
    server.create_token(&admin, "renew", "pw", 100).await;
    let expires_at = 4_000_000_000_u64;
    let (status, _, _) = server
        .admin_post(
            "/api/admin/tokens/renew/renew",
            &admin,
            &json!({"expires_at": expires_at}),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let tokens = server.token_list(&admin).await;
    assert_eq!(find_token(&tokens, "renew")["expires_at"], expires_at);
}

#[tokio::test]
async fn renew_token_extends_with_duration() {
    let server = TestServer::new().await;
    let admin = server.admin_token().await;
    server.create_token(&admin, "renew", "pw", 100).await;
    let before = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock")
        .as_secs();
    let (status, _, _) = server
        .admin_post(
            "/api/admin/tokens/renew/renew",
            &admin,
            &json!({"duration": "seven_days"}),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let tokens = server.token_list(&admin).await;
    let expires_at = find_token(&tokens, "renew")["expires_at"]
        .as_u64()
        .expect("expiry");
    assert!(
        expires_at >= before + 6 * 86_400,
        "expiry should extend ~7d"
    );
}

#[tokio::test]
async fn renew_token_updates_other_uid() {
    let server = TestServer::new().await;
    let admin = server.admin_token().await;
    server.create_token(&admin, "renew", "pw", 100).await;
    let (status, _, _) = server
        .admin_post(
            "/api/admin/tokens/renew/renew",
            &admin,
            &json!({"extend_seconds": 60, "otherUid": "order-42"}),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let tokens = server.token_list(&admin).await;
    let token = find_token(&tokens, "renew");
    assert_eq!(token["other_uid"], "order-42");
    assert_eq!(token["otherUid"], "order-42");
}

#[tokio::test]
async fn renew_token_period_change_resets_usage() {
    let server = TestServer::with_links(&all_sample_links()).await;
    let admin = server.admin_token().await;
    server.create_token(&admin, "period", "pw", 1_000_000).await;
    let (status, _, _) = server.get_basic("/sub/c", "period", "pw").await;
    assert_eq!(status, StatusCode::OK);
    let tokens = server.token_list(&admin).await;
    assert!(
        find_token(&tokens, "period")["used_bytes"]
            .as_u64()
            .unwrap_or_default()
            > 0
    );
    let (status, _, _) = server
        .admin_post(
            "/api/admin/tokens/period/renew",
            &admin,
            &json!({"extend_seconds": 3600, "quota_period": "daily"}),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let tokens = server.token_list(&admin).await;
    let token = find_token(&tokens, "period");
    assert_eq!(token["used_bytes"], 0);
    assert_eq!(token["quota_period"], "daily");
    assert!(token["next_reset_at"].as_u64().is_some());
}

#[tokio::test]
async fn reset_token_clears_usage() {
    let server = TestServer::with_links(&all_sample_links()).await;
    let admin = server.admin_token().await;
    server
        .create_token(&admin, "resetme", "pw", 1_000_000)
        .await;
    let (status, _, _) = server.get_basic("/sub/c", "resetme", "pw").await;
    assert_eq!(status, StatusCode::OK);
    let (status, _, _) = server
        .admin_post("/api/admin/tokens/resetme/reset", &admin, &json!({}))
        .await;
    assert_eq!(status, StatusCode::OK);
    let tokens = server.token_list(&admin).await;
    let token = find_token(&tokens, "resetme");
    assert_eq!(token["used_bytes"], 0);
    assert_eq!(token["requests"], 0);
}

#[tokio::test]
async fn expired_token_is_auto_revoked() {
    let server = TestServer::new().await;
    let admin = server.admin_token().await;
    let (status, _, _) = server
        .admin_post(
            "/api/admin/tokens",
            &admin,
            &json!({
                "token": "expired",
                "password": "pw",
                "limit_bytes": 100,
                "expires_at": 1,
            }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED);
    let tokens = server.token_list(&admin).await;
    let token = find_token(&tokens, "expired");
    assert_eq!(token["enabled"], false);
    assert_eq!(token["disabled_reason"], "expired");
    assert!(token["retired_at"].as_u64().is_some());
}

#[tokio::test]
async fn quota_exceeded_returns_429() {
    let server = TestServer::with_links(&all_sample_links()).await;
    let admin = server.admin_token().await;
    server.create_token(&admin, "tiny", "pw", 1).await;
    let (status, _, _) = server.get_basic("/sub/c", "tiny", "pw").await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
}

#[tokio::test]
async fn subscription_rejects_wrong_password() {
    let server = TestServer::with_links(&all_sample_links()).await;
    let admin = server.admin_token().await;
    server.create_token(&admin, "secure", "right", 10_000).await;
    let (status, _, _) = server.get_basic("/sub/c", "secure", "wrong").await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn subscription_link_returns_jwt_and_filename() {
    let server = TestServer::with_links(&all_sample_links()).await;
    let admin = server.admin_token().await;
    let (status, _, _) = server
        .admin_post(
            "/api/admin/tokens",
            &admin,
            &json!({
                "token": "linked",
                "password": "pw",
                "limit_bytes": 10_000,
                "label": "Home Profile",
            }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED);
    let (status, _, body) = server
        .admin_post("/api/admin/tokens/linked/link", &admin, &json!({}))
        .await;
    assert_eq!(status, StatusCode::OK);
    let value = json_body(&body);
    assert!(!value["access_token"]
        .as_str()
        .unwrap_or_default()
        .is_empty());
    assert_eq!(value["filename"], "Home_Profile.yaml");
}

#[tokio::test]
async fn subscription_accepts_access_token_jwt() {
    let server = TestServer::with_links(&all_sample_links()).await;
    let admin = server.admin_token().await;
    server
        .create_token(&admin, "jwt-token", "pw", 1_000_000)
        .await;
    let (status, _, body) = server
        .admin_post("/api/admin/tokens/jwt-token/link", &admin, &json!({}))
        .await;
    assert_eq!(status, StatusCode::OK);
    let access_token = json_body(&body)["access_token"]
        .as_str()
        .expect("access token")
        .to_owned();
    let (status, _, _) = server
        .get(&format!(
            "/sub/c?token=jwt-token&access_token={access_token}"
        ))
        .await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn subscription_link_rejects_expired_token() {
    let server = TestServer::new().await;
    let admin = server.admin_token().await;
    server
        .admin_post(
            "/api/admin/tokens",
            &admin,
            &json!({
                "token": "shortlived",
                "password": "pw",
                "limit_bytes": 10,
                "expires_at": 1,
            }),
        )
        .await;
    let (status, _, _) = server
        .admin_post("/api/admin/tokens/shortlived/link", &admin, &json!({}))
        .await;
    assert_eq!(status, StatusCode::CONFLICT);
}

// ---------------------------------------------------------------------------
// Accounts, groups, and account links
// ---------------------------------------------------------------------------

#[tokio::test]
async fn accounts_crud_flow() {
    let server = TestServer::new().await;
    let admin = server.admin_token().await;
    let (status, _, body) = server
        .admin_post(
            "/api/admin/accounts",
            &admin,
            &json!({"username": "alice", "display_name": "Alice"}),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(json_body(&body)["account"]["username"], "alice");
    let (status, _, body) = server.admin_get("/api/admin/accounts", &admin).await;
    assert_eq!(status, StatusCode::OK);
    let accounts = json_body(&body);
    let accounts = accounts["accounts"].as_array().expect("accounts");
    assert!(accounts
        .iter()
        .any(|account| account["username"] == "alice"));
    let (status, _, _) = server
        .admin_patch(
            "/api/admin/accounts/alice",
            &admin,
            &json!({"display_name": "Alice Updated"}),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let (status, _, _) = server
        .admin_delete("/api/admin/accounts/alice", &admin)
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, _, body) = server.admin_get("/api/admin/accounts", &admin).await;
    assert_eq!(status, StatusCode::OK);
    let accounts = json_body(&body);
    let alice = accounts["accounts"]
        .as_array()
        .expect("accounts")
        .iter()
        .find(|account| account["account_id"] == "alice")
        .expect("alice account");
    assert_eq!(alice["enabled"], false);
    assert!(alice["deleted_at"].as_u64().is_some());
}

#[tokio::test]
async fn legacy_account_cannot_be_disabled_or_deleted() {
    let server = TestServer::new().await;
    let admin = server.admin_token().await;
    let (status, _, _) = server
        .admin_patch(
            "/api/admin/accounts/legacy",
            &admin,
            &json!({"enabled": false}),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _, _) = server
        .admin_delete("/api/admin/accounts/legacy", &admin)
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn duplicate_account_username_conflicts() {
    let server = TestServer::new().await;
    let admin = server.admin_token().await;
    server
        .admin_post("/api/admin/accounts", &admin, &json!({"username": "bob"}))
        .await;
    let (status, _, _) = server
        .admin_post("/api/admin/accounts", &admin, &json!({"username": "bob"}))
        .await;
    assert_eq!(status, StatusCode::CONFLICT);
}

#[tokio::test]
async fn create_account_link_returns_working_password() {
    let server = TestServer::with_links(&all_sample_links()).await;
    let admin = server.admin_token().await;
    server
        .admin_post("/api/admin/accounts", &admin, &json!({"username": "carol"}))
        .await;
    let (status, _, body) = server
        .admin_post(
            "/api/admin/accounts/carol/links",
            &admin,
            &json!({
                "token": "carol-link",
                "password": "carol-pass",
                "limit_bytes": 500_000,
                "label": "Carol Link",
            }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(json_body(&body)["password"], "carol-pass");
    let (status, _, _) = server.get_basic("/sub/c", "carol-link", "carol-pass").await;
    assert_eq!(status, StatusCode::OK);
    let (status, headers, body) = server
        .admin_get("/api/admin/accounts/carol/links", &admin)
        .await;
    assert_eq!(status, StatusCode::OK);
    let links = json_body(&body);
    let links = links["links"].as_array().expect("links");
    assert_eq!(links.len(), 1);
    assert_eq!(links[0]["token"], "carol-link");
    assert_eq!(links[0]["label"], "Carol Link");
    assert!(headers.contains_key(header::CONTENT_TYPE));
}

#[tokio::test]
async fn create_account_link_rejects_unknown_group() {
    let server = TestServer::new().await;
    let admin = server.admin_token().await;
    let (status, _, _) = server
        .admin_post(
            "/api/admin/accounts/legacy/links",
            &admin,
            &json!({
                "token": "bad-group",
                "password": "pw",
                "limit_bytes": 100,
                "group_id": "missing",
            }),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn rotate_account_link_invalidates_old_password() {
    let server = TestServer::with_links(&all_sample_links()).await;
    let admin = server.admin_token().await;
    server
        .admin_post(
            "/api/admin/accounts/legacy/links",
            &admin,
            &json!({"token": "spin", "password": "old", "limit_bytes": 100_000}),
        )
        .await;
    let (status, _, body) = server
        .admin_post(
            "/api/admin/accounts/legacy/links/spin/rotate",
            &admin,
            &json!({}),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let password = json_body(&body)["password"]
        .as_str()
        .expect("new password")
        .to_owned();
    let (status, _, _) = server.get_basic("/sub/c", "spin", "old").await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, _, _) = server.get_basic("/sub/c", "spin", &password).await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn delete_account_link_blocks_subscription() {
    let server = TestServer::with_links(&all_sample_links()).await;
    let admin = server.admin_token().await;
    server
        .admin_post(
            "/api/admin/accounts/legacy/links",
            &admin,
            &json!({"token": "removable", "password": "pw", "limit_bytes": 100_000}),
        )
        .await;
    let (status, _, _) = server
        .admin_delete("/api/admin/accounts/legacy/links/removable", &admin)
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, _, _) = server.get_basic("/sub/c", "removable", "pw").await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn group_create_reserves_all_identifier() {
    let server = TestServer::new().await;
    let admin = server.admin_token().await;
    let (status, _, _) = server
        .admin_post("/api/admin/groups", &admin, &json!({"name": "all"}))
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn group_crud_and_token_membership() {
    let server = TestServer::with_links(&all_sample_links()).await;
    let admin = server.admin_token().await;
    let (status, _, body) = server
        .admin_post(
            "/api/admin/groups",
            &admin,
            &json!({"name": "family", "node_links": [VLESS_LINK]}),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(json_body(&body)["group"]["node_count"], 1);
    server
        .create_account_link_with_group(&admin, "family-token", "pw", "family")
        .await;
    let (status, _, body) = server.get_basic("/sub/links", "family-token", "pw").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body_text(&body), format!("{VLESS_LINK}\n"));
    let (status, _, body) = server.admin_get("/api/admin/groups", &admin).await;
    assert_eq!(status, StatusCode::OK);
    let groups = json_body(&body);
    assert!(groups["groups"]
        .as_array()
        .expect("groups")
        .iter()
        .any(|group| group["group_id"] == "family"));
    let (status, _, _) = server
        .admin_delete("/api/admin/groups/family", &admin)
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let tokens = server.token_list(&admin).await;
    assert_eq!(find_token(&tokens, "family-token")["group_id"], "all");
}

impl TestServer {
    async fn create_account_link_with_group(
        &self,
        bearer: &str,
        token: &str,
        password: &str,
        group_id: &str,
    ) {
        let (status, _, _) = self
            .admin_post(
                "/api/admin/accounts/legacy/links",
                bearer,
                &json!({
                    "token": token,
                    "password": password,
                    "limit_bytes": 1_000_000,
                    "group_id": group_id,
                }),
            )
            .await;
        assert_eq!(status, StatusCode::CREATED);
    }
}

#[tokio::test]
async fn groups_require_authentication() {
    let server = TestServer::new().await;
    let (status, _, _) = server.get("/api/admin/groups").await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

// ---------------------------------------------------------------------------
// Provider registry
// ---------------------------------------------------------------------------

#[tokio::test]
async fn registry_register_requires_registry_token() {
    let server = TestServer::new().await;
    let (status, _, _) = server
        .send(
            Request::builder()
                .method(Method::POST)
                .uri("/api/v1/registry/register")
                .header("x-provider-registry-token", "wrong-token")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({"provider_id": "p1", "name": "P1", "version": "1"}).to_string(),
                ))
                .expect("request"),
        )
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn registry_register_heartbeat_unregister_flow() {
    let server = TestServer::with_links(&all_sample_links()).await;
    let admin = server.admin_token().await;
    server
        .create_token(&admin, "reg-token", "pw", 1_000_000)
        .await;
    let heartbeat = server
        .register_provider("provider-1", &[VLESS_LINK.to_owned()])
        .await;
    let (status, _, body) = server.admin_get("/api/admin/providers", &admin).await;
    assert_eq!(status, StatusCode::OK);
    let value = json_body(&body);
    let providers = value["providers"].as_array().expect("providers");
    assert_eq!(providers.len(), 1);
    assert_eq!(providers[0]["provider_id"], "provider-1");
    assert_eq!(providers[0]["node_count"], 1);
    assert_eq!(providers[0]["online"], true);
    let (status, _, _) = server.get_basic("/sub/links", "reg-token", "pw").await;
    assert_eq!(status, StatusCode::OK);
    let (status, _, _) = server
        .send(
            Request::builder()
                .method(Method::POST)
                .uri("/api/v1/registry/heartbeat")
                .header("x-provider-registry-token", REGISTRY_TOKEN)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({"provider_id": "provider-1", "heartbeat_token": heartbeat}).to_string(),
                ))
                .expect("request"),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let (status, _, _) = server
        .send(
            Request::builder()
                .method(Method::POST)
                .uri("/api/v1/registry/unregister")
                .header("x-provider-registry-token", REGISTRY_TOKEN)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({"provider_id": "provider-1", "heartbeat_token": heartbeat}).to_string(),
                ))
                .expect("request"),
        )
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (_, _, body) = server.admin_get("/api/admin/providers", &admin).await;
    let value = json_body(&body);
    let providers = value["providers"].as_array().expect("providers");
    assert_eq!(providers[0]["enabled"], false);
    assert_eq!(providers[0]["online"], false);
}

#[tokio::test]
async fn registry_heartbeat_rejects_bad_token() {
    let server = TestServer::new().await;
    server
        .register_provider("provider-2", &[VLESS_LINK.to_owned()])
        .await;
    let (status, _, _) = server
        .send(
            Request::builder()
                .method(Method::POST)
                .uri("/api/v1/registry/heartbeat")
                .header("x-provider-registry-token", REGISTRY_TOKEN)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({"provider_id": "provider-2", "heartbeat_token": "not-the-token"})
                        .to_string(),
                ))
                .expect("request"),
        )
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn registry_heartbeat_unknown_provider_is_not_found() {
    let server = TestServer::new().await;
    let (status, _, _) = server
        .send(
            Request::builder()
                .method(Method::POST)
                .uri("/api/v1/registry/heartbeat")
                .header("x-provider-registry-token", REGISTRY_TOKEN)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({"provider_id": "ghost", "heartbeat_token": "anything"}).to_string(),
                ))
                .expect("request"),
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn registry_reregistration_rotates_heartbeat_token() {
    let server = TestServer::new().await;
    let first = server
        .register_provider("provider-3", &[VLESS_LINK.to_owned()])
        .await;
    let second = server
        .register_provider("provider-3", &[VLESS_LINK.to_owned()])
        .await;
    assert_ne!(first, second);
    let heartbeat_old = server
        .send(
            Request::builder()
                .method(Method::POST)
                .uri("/api/v1/registry/heartbeat")
                .header("x-provider-registry-token", REGISTRY_TOKEN)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({"provider_id": "provider-3", "heartbeat_token": first}).to_string(),
                ))
                .expect("request"),
        )
        .await;
    assert_eq!(heartbeat_old.0, StatusCode::UNAUTHORIZED);
    let heartbeat_new = server
        .send(
            Request::builder()
                .method(Method::POST)
                .uri("/api/v1/registry/heartbeat")
                .header("x-provider-registry-token", REGISTRY_TOKEN)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({"provider_id": "provider-3", "heartbeat_token": second}).to_string(),
                ))
                .expect("request"),
        )
        .await;
    assert_eq!(heartbeat_new.0, StatusCode::OK);
}

#[tokio::test]
async fn disabled_provider_nodes_are_excluded_from_subscription() {
    let server = TestServer::new().await;
    let admin = server.admin_token().await;
    server
        .create_token(&admin, "no-provider", "pw", 1_000_000)
        .await;
    server
        .register_provider("provider-4", &[VLESS_LINK.to_owned()])
        .await;
    let (_, _, body) = server.get_basic("/sub/links", "no-provider", "pw").await;
    assert!(body_text(&body).contains("vless://"));
    let (status, _, _) = server
        .admin_post(
            "/api/admin/providers/provider-4/disable",
            &admin,
            &json!({}),
        )
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (_, _, body) = server.get_basic("/sub/links", "no-provider", "pw").await;
    assert!(!body_text(&body).contains("vless://"));
    // A disabled provider cannot register again.
    let (status, _, _) = server
        .send(
            Request::builder()
                .method(Method::POST)
                .uri("/api/v1/registry/register")
                .header("x-provider-registry-token", REGISTRY_TOKEN)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({"provider_id": "provider-4", "name": "P4", "version": "1"}).to_string(),
                ))
                .expect("request"),
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn stale_providers_are_excluded_from_subscriptions() {
    let server = TestServer::with(|config| {
        config.server.provider_stale_after_secs = 0;
    })
    .await;
    let admin = server.admin_token().await;
    server
        .create_token(&admin, "stale-token", "pw", 1_000_000)
        .await;
    server
        .register_provider("provider-stale", &[VLESS_LINK.to_owned()])
        .await;
    let (status, _, body) = server.get_basic("/sub/links", "stale-token", "pw").await;
    assert_eq!(status, StatusCode::OK);
    assert!(!body_text(&body).contains("vless://"));
}

#[tokio::test]
async fn admin_providers_require_authentication() {
    let server = TestServer::new().await;
    let (status, _, _) = server.get("/api/admin/providers").await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

// ---------------------------------------------------------------------------
// Traffic
// ---------------------------------------------------------------------------

#[tokio::test]
async fn admin_traffic_requires_authentication() {
    let server = TestServer::new().await;
    let (status, _, _) = server.get("/api/admin/traffic").await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn admin_traffic_falls_back_to_subscription_usage() {
    let server = TestServer::with_links(&all_sample_links()).await;
    let admin = server.admin_token().await;
    server
        .create_token(&admin, "traffic", "pw", 1_000_000)
        .await;
    let (status, _, body) = server.get_basic("/sub/c", "traffic", "pw").await;
    assert_eq!(status, StatusCode::OK);
    let charged = body.len() as u64;
    let (status, _, body) = server.admin_get("/api/admin/traffic?days=7", &admin).await;
    assert_eq!(status, StatusCode::OK);
    let value = json_body(&body);
    assert_eq!(value["source"], "subscription_server");
    assert_eq!(value["available"], false);
    assert_eq!(value["range_days"], 7);
    assert!(value["summary"]["download_bytes"].as_u64().unwrap_or(0) >= charged);
    assert!(!value["daily"].as_array().expect("daily").is_empty());
}

#[tokio::test]
async fn admin_traffic_degrades_when_client_relay_is_unreachable() {
    let server = TestServer::with(|config| {
        config.server.client_stats_url = "http://127.0.0.1:9/api/v1/stats".into();
        config.server.client_stats_user = "root".into();
        config.server.client_stats_password = "fake".into();
    })
    .await;
    let admin = server.admin_token().await;
    let (status, _, body) = server.admin_get("/api/admin/traffic", &admin).await;
    assert_eq!(status, StatusCode::OK);
    let value = json_body(&body);
    assert_eq!(value["available"], false);
    assert_eq!(value["range_days"], 30);
    assert!(value["domains"].as_array().expect("domains").is_empty());
}

#[tokio::test]
async fn admin_traffic_clamps_days_query() {
    let server = TestServer::new().await;
    let admin = server.admin_token().await;
    let (status, _, body) = server
        .admin_get("/api/admin/traffic?days=99999", &admin)
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json_body(&body)["range_days"], 400);
    let (status, _, body) = server
        .admin_get("/api/admin/traffic?days=abc", &admin)
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json_body(&body)["range_days"], 30);
}

// ---------------------------------------------------------------------------
// Subscription formats, filenames, headers, and byte accounting
// ---------------------------------------------------------------------------

#[tokio::test]
async fn clash_subscription_returns_yaml_with_nodes() {
    let server = TestServer::with_links(&all_sample_links()).await;
    let admin = server.admin_token().await;
    server.create_token(&admin, "clash", "pw", 1_000_000).await;
    let (status, headers, body) = server.get_basic("/sub/c", "clash", "pw").await;
    assert_eq!(status, StatusCode::OK);
    let content_type = headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    assert!(content_type.starts_with("text/yaml"), "got {content_type}");
    let text = body_text(&body);
    let yaml: serde_yaml::Value = serde_yaml::from_str(&text).expect("clash body is YAML");
    let proxies = yaml["proxies"].as_sequence().expect("proxies list");
    assert_eq!(proxies.len(), 4);
    assert!(text.contains("edge-node"));
    assert!(text.contains("proxy-groups:"));
    assert!(text.contains("rules:"));
}

#[tokio::test]
async fn clash_subscription_sets_cache_and_profile_headers() {
    let server = TestServer::with_links(&all_sample_links()).await;
    let admin = server.admin_token().await;
    server
        .create_token(&admin, "headers", "pw", 1_000_000)
        .await;
    let (status, headers, _) = server.get_basic("/sub/c", "headers", "pw").await;
    assert_eq!(status, StatusCode::OK);
    let cache_control = headers
        .get(header::CACHE_CONTROL)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    assert!(cache_control.contains("no-store"));
    let userinfo = headers
        .get("subscription-userinfo")
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    assert!(userinfo.contains("total=1000000"));
    assert!(userinfo.contains("download="));
    assert_eq!(
        headers
            .get("profile-update-interval")
            .and_then(|value| value.to_str().ok()),
        Some("3600")
    );
    let disposition = headers
        .get(header::CONTENT_DISPOSITION)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    assert!(
        disposition.contains("filename=vpn233.yaml"),
        "{disposition}"
    );
}

#[tokio::test]
async fn subscription_default_uses_configured_format() {
    let server = TestServer::with(|config| {
        config.server.default_format = "links".into();
        config.subscriptions = vec![SubscriptionSource {
            name: "alpha".into(),
            links: vec![VLESS_LINK.into()],
            url: String::new(),
            access_token: String::new(),
        }];
    })
    .await;
    let admin = server.admin_token().await;
    server
        .create_token(&admin, "default-token", "pw", 1_000_000)
        .await;
    let (status, headers, body) = server.get_basic("/sub", "default-token", "pw").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        headers
            .get(header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok()),
        Some("text/plain; charset=utf-8")
    );
    assert_eq!(body_text(&body), format!("{VLESS_LINK}\n"));
}

#[tokio::test]
async fn subscription_links_returns_raw_links() {
    let server = TestServer::with_links(&all_sample_links()).await;
    let admin = server.admin_token().await;
    server
        .create_token(&admin, "links-token", "pw", 1_000_000)
        .await;
    let (status, headers, body) = server.get_basic("/sub/links", "links-token", "pw").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        headers
            .get(header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok()),
        Some("text/plain; charset=utf-8")
    );
    let text = body_text(&body);
    assert_eq!(text.lines().count(), 4);
    for link in all_sample_links() {
        assert!(text.contains(&link), "missing {link}");
    }
}

#[tokio::test]
async fn subscription_base64_decodes_to_links() {
    let server = TestServer::with_links(&all_sample_links()).await;
    let admin = server.admin_token().await;
    server
        .create_token(&admin, "base64-token", "pw", 1_000_000)
        .await;
    let (status, headers, body) = server.get_basic("/sub/base64", "base64-token", "pw").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        headers
            .get(header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok()),
        Some("text/plain; charset=utf-8")
    );
    let decoded = BASE64
        .decode(body_text(&body).trim())
        .expect("base64 subscription decodes");
    let decoded = String::from_utf8(decoded).expect("decoded text");
    assert!(decoded.contains(VLESS_LINK));
    assert_eq!(decoded.lines().count(), 4);
}

#[tokio::test]
async fn subscription_requires_authentication() {
    let server = TestServer::with_links(&all_sample_links()).await;
    for path in [
        "/sub",
        "/sub/c",
        "/sub/links",
        "/sub/base64",
        "/sub/clash",
        "/sub/singbox",
        "/sub/v2ray",
        "/sub/surge",
        "/sub/quantumultx",
        "/sub/loon",
        "/sub/surfboard",
        "/sub/stash",
    ] {
        let (status, _, _) = server.get(path).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "path {path}");
    }
}

#[tokio::test]
async fn subscription_filename_is_sanitized() {
    let server = TestServer::with_links(&all_sample_links()).await;
    let admin = server.admin_token().await;
    server
        .create_token(&admin, "filename", "pw", 1_000_000)
        .await;
    let (status, headers, _) = server
        .get_basic("/sub/c?filename=../evil", "filename", "pw")
        .await;
    assert_eq!(status, StatusCode::OK);
    let disposition = headers
        .get(header::CONTENT_DISPOSITION)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    assert!(disposition.contains("filename=evil.yaml"), "{disposition}");
    assert!(!disposition.contains(".."));
    assert!(!disposition.contains('/'));
}

#[tokio::test]
async fn subscription_filename_falls_back_when_blank() {
    let server = TestServer::with_links(&all_sample_links()).await;
    let admin = server.admin_token().await;
    server
        .create_token(&admin, "filename", "pw", 1_000_000)
        .await;
    let (status, headers, _) = server
        .get_basic("/sub/c?filename=%2F%2F..", "filename", "pw")
        .await;
    assert_eq!(status, StatusCode::OK);
    let disposition = headers
        .get(header::CONTENT_DISPOSITION)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    assert!(
        disposition.contains("filename=vpn233.yaml"),
        "{disposition}"
    );
}

#[tokio::test]
async fn subscription_named_source_filters_nodes() {
    let server = TestServer::with(|config| {
        config.subscriptions = vec![
            SubscriptionSource {
                name: "alpha".into(),
                links: vec![VLESS_LINK.into()],
                url: String::new(),
                access_token: String::new(),
            },
            SubscriptionSource {
                name: "beta".into(),
                links: vec![TROJAN_LINK.into()],
                url: String::new(),
                access_token: String::new(),
            },
        ];
    })
    .await;
    let admin = server.admin_token().await;
    server.create_token(&admin, "named", "pw", 1_000_000).await;
    let (status, _, body) = server.get_basic("/sub/c/alpha", "named", "pw").await;
    assert_eq!(status, StatusCode::OK);
    let text = body_text(&body);
    assert!(text.contains("edge-node"));
    assert!(!text.contains("trojan-node"));
}

#[tokio::test]
async fn subscription_byte_accounting_matches_body_length() {
    let server = TestServer::with_links(&all_sample_links()).await;
    let admin = server.admin_token().await;
    server
        .create_token(&admin, "metered", "pw", 5_000_000)
        .await;
    let (status, headers, body) = server.get_basic("/sub/links", "metered", "pw").await;
    assert_eq!(status, StatusCode::OK);
    let body_len = body.len() as u64;
    let userinfo = headers
        .get("subscription-userinfo")
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_owned();
    assert!(
        userinfo.contains(&format!("download={body_len}")),
        "{userinfo}"
    );
    let tokens = server.token_list(&admin).await;
    assert_eq!(find_token(&tokens, "metered")["used_bytes"], body_len);
    assert_eq!(find_token(&tokens, "metered")["requests"], 1);
}

#[tokio::test]
async fn unsupported_protocol_nodes_are_skipped() {
    let server = TestServer::with(|config| {
        config.subscriptions = vec![SubscriptionSource {
            name: "mixed".into(),
            links: vec![
                "unknownproto://fake@example.invalid:443#skip-node".into(),
                VLESS_LINK.into(),
            ],
            url: String::new(),
            access_token: String::new(),
        }];
    })
    .await;
    let admin = server.admin_token().await;
    server.create_token(&admin, "mixed", "pw", 1_000_000).await;
    let (status, headers, body) = server.get_basic("/sub/c", "mixed", "pw").await;
    assert_eq!(status, StatusCode::OK);
    let text = body_text(&body);
    assert!(text.contains("edge-node"));
    assert!(!text.contains("skip-node"));
    assert_eq!(
        headers
            .get("x-vpn233-nodes")
            .and_then(|value| value.to_str().ok()),
        Some("1")
    );
    assert_eq!(
        headers
            .get("x-vpn233-skipped")
            .and_then(|value| value.to_str().ok()),
        Some("1")
    );
}

// ---------------------------------------------------------------------------
// Phase B format routing
// ---------------------------------------------------------------------------

/// `(route segment, content type, extension, canonical format id)`.
const FORMAT_ROUTES: &[(&str, &str, &str, &str)] = &[
    ("clash", "text/yaml; charset=utf-8", "yaml", "clash"),
    ("mihomo", "text/yaml; charset=utf-8", "yaml", "clash"),
    (
        "singbox",
        "application/json; charset=utf-8",
        "json",
        "singbox",
    ),
    (
        "sing-box",
        "application/json; charset=utf-8",
        "json",
        "singbox",
    ),
    ("v2ray", "text/plain; charset=utf-8", "txt", "v2ray"),
    ("v2rayn", "text/plain; charset=utf-8", "txt", "v2ray"),
    ("surge", "text/plain; charset=utf-8", "conf", "surge"),
    (
        "quantumultx",
        "text/plain; charset=utf-8",
        "conf",
        "quantumultx",
    ),
    ("qx", "text/plain; charset=utf-8", "conf", "quantumultx"),
    ("loon", "text/plain; charset=utf-8", "conf", "loon"),
    (
        "surfboard",
        "text/plain; charset=utf-8",
        "conf",
        "surfboard",
    ),
    ("stash", "text/yaml; charset=utf-8", "yaml", "stash"),
    ("hiddify", "text/plain; charset=utf-8", "txt", "v2ray"),
    ("shadowrocket", "text/plain; charset=utf-8", "txt", "v2ray"),
    ("nekobox", "text/plain; charset=utf-8", "txt", "v2ray"),
    ("links", "text/plain; charset=utf-8", "txt", "links"),
    ("base64", "text/plain; charset=utf-8", "txt", "v2ray"),
];

fn header_value(headers: &HeaderMap, name: &str) -> String {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_owned()
}

#[tokio::test]
async fn format_routes_serve_expected_payloads() {
    let server = TestServer::with_links(&all_sample_links()).await;
    let admin = server.admin_token().await;
    server
        .create_token(&admin, "formats", "pw", 10_000_000)
        .await;
    for (route, content_type, extension, canonical) in FORMAT_ROUTES {
        let (status, headers, body) = server
            .get_basic(&format!("/sub/{route}"), "formats", "pw")
            .await;
        assert_eq!(status, StatusCode::OK, "route {route}");
        assert_eq!(
            header_value(&headers, "content-type"),
            *content_type,
            "route {route}"
        );
        assert!(
            header_value(&headers, "content-disposition")
                .contains(&format!("filename=vpn233.{extension}")),
            "route {route} disposition: {}",
            header_value(&headers, "content-disposition")
        );
        assert_eq!(
            header_value(&headers, "x-vpn233-format"),
            *canonical,
            "route {route}"
        );
        let nodes = header_value(&headers, "x-vpn233-nodes")
            .parse::<usize>()
            .expect("nodes header is an integer");
        assert!(nodes >= 1, "route {route} has no nodes");
        let _skipped = header_value(&headers, "x-vpn233-skipped")
            .parse::<usize>()
            .expect("skipped header is an integer");
        assert!(header_value(&headers, "cache-control").contains("no-store"));
        assert!(header_value(&headers, "subscription-userinfo").contains("total=10000000"));
        let text = body_text(&body);
        assert!(!text.trim().is_empty(), "route {route} body is empty");
        match *extension {
            "yaml" => {
                let value: serde_yaml::Value =
                    serde_yaml::from_str(&text).unwrap_or_else(|_| panic!("route {route} YAML"));
                let proxies = value["proxies"].as_sequence().expect("proxies list");
                assert_eq!(proxies.len(), 4, "route {route}");
            }
            "json" => {
                let value: Value =
                    serde_json::from_str(&text).unwrap_or_else(|_| panic!("route {route} JSON"));
                assert!(value.get("outbounds").is_some(), "route {route}");
                assert!(text.contains("example.invalid"), "route {route}");
            }
            "txt" if *canonical == "v2ray" => {
                let decoded = BASE64
                    .decode(text.trim())
                    .unwrap_or_else(|_| panic!("route {route} base64"));
                let decoded = String::from_utf8(decoded).expect("decoded UTF-8");
                assert_eq!(decoded.lines().count(), 4, "route {route}");
            }
            "txt" => {
                assert_eq!(text.lines().count(), 4, "route {route}");
                assert!(text.contains("vless://"), "route {route}");
            }
            _ => {
                assert!(text.contains("example.invalid"), "route {route}");
            }
        }
    }
}

#[tokio::test]
async fn format_query_overrides_the_route_format() {
    let server = TestServer::with_links(&all_sample_links()).await;
    let admin = server.admin_token().await;
    server
        .create_token(&admin, "override", "pw", 10_000_000)
        .await;
    let (status, headers, body) = server
        .get_basic("/sub/clash?format=singbox", "override", "pw")
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        header_value(&headers, "content-type"),
        "application/json; charset=utf-8"
    );
    assert_eq!(header_value(&headers, "x-vpn233-format"), "singbox");
    serde_json::from_str::<Value>(&body_text(&body)).expect("sing-box JSON");
    let (status, headers, _) = server
        .get_basic("/sub?format=links", "override", "pw")
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        header_value(&headers, "content-type"),
        "text/plain; charset=utf-8"
    );
    assert_eq!(header_value(&headers, "x-vpn233-format"), "links");
}

#[tokio::test]
async fn unknown_format_is_rejected_with_400() {
    let server = TestServer::with_links(&all_sample_links()).await;
    let admin = server.admin_token().await;
    server
        .create_token(&admin, "unknown", "pw", 10_000_000)
        .await;
    for uri in [
        "/sub?format=bogus",
        "/sub/clash?format=bogus",
        "/sub/bogus-format",
    ] {
        let (status, _, _) = server.get_basic(uri, "unknown", "pw").await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "uri {uri}");
    }
}

#[tokio::test]
async fn default_format_supports_every_canonical_value() {
    for id in [
        "clash",
        "singbox",
        "v2ray",
        "links",
        "surge",
        "quantumultx",
        "loon",
        "surfboard",
        "stash",
    ] {
        let server = TestServer::with(move |config| {
            config.server.default_format = id.into();
            config.subscriptions = vec![SubscriptionSource {
                name: "alpha".into(),
                links: all_sample_links(),
                url: String::new(),
                access_token: String::new(),
            }];
        })
        .await;
        let admin = server.admin_token().await;
        server
            .create_token(&admin, "default", "pw", 10_000_000)
            .await;
        let (status, headers, _) = server.get_basic("/sub", "default", "pw").await;
        assert_eq!(status, StatusCode::OK, "default_format {id}");
        assert_eq!(
            header_value(&headers, "x-vpn233-format"),
            id,
            "default_format {id}"
        );
    }
}

#[tokio::test]
async fn telemetry_headers_report_nodes_and_skipped() {
    let server = TestServer::with(|config| {
        config.subscriptions = vec![SubscriptionSource {
            name: "mixed".into(),
            links: vec![VLESS_LINK.into(), TROJAN_LINK.into()],
            url: String::new(),
            access_token: String::new(),
        }];
    })
    .await;
    let admin = server.admin_token().await;
    server
        .create_token(&admin, "telemetry", "pw", 10_000_000)
        .await;
    let (status, headers, _) = server.get_basic("/sub/surge", "telemetry", "pw").await;
    assert_eq!(status, StatusCode::OK);
    // Surge cannot represent vless, so exactly one parsed node is skipped.
    assert_eq!(header_value(&headers, "x-vpn233-nodes"), "2");
    assert_eq!(header_value(&headers, "x-vpn233-skipped"), "1");
    assert_eq!(header_value(&headers, "x-vpn233-rendered"), "1");
    let (_, headers, _) = server.get_basic("/sub/clash", "telemetry", "pw").await;
    assert_eq!(header_value(&headers, "x-vpn233-nodes"), "2");
    assert_eq!(header_value(&headers, "x-vpn233-skipped"), "0");
}

#[tokio::test]
async fn remote_mihomo_feed_preserves_proxy_options_and_drops_remote_configuration() {
    let payload = r#"proxies:
  - name: remote-node
    type: vless
    server: example.invalid
    port: 443
    uuid: change-me
    tls: true
    udp: true
    network: ws
    ws-opts:
      path: /ws
      max-early-data: 1024
      headers: { Host: front.example.invalid }
proxy-providers:
  private: { url: 'https://must-not-fetch.example.invalid' }
external-controller: 0.0.0.0:9090
"#;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let feed = Router::new().route("/feed", axum::routing::get(move || async move { payload }));
    let task = tokio::spawn(async move {
        axum::serve(listener, feed).await.unwrap();
    });
    let server = TestServer::with(move |config| {
        config.subscriptions = vec![SubscriptionSource {
            name: "remote".into(),
            url: format!("http://{address}/feed"),
            links: vec![],
            access_token: String::new(),
        }];
    })
    .await;
    let admin = server.admin_token().await;
    server
        .create_token(&admin, "structured", "pw", 1_000_000)
        .await;
    let (status, headers, body) = server.get_basic("/sub/mihomo", "structured", "pw").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(header_value(&headers, "x-vpn233-rendered"), "1");
    let body = body_text(&body);
    assert!(body.contains("max-early-data: 1024"));
    assert!(body.contains("udp: true"));
    assert!(!body.contains("must-not-fetch"));
    assert!(!body.contains("external-controller"));
    let (_, _, body) = server.admin_get("/api/nodes", &admin).await;
    assert_eq!(json_body(&body).as_array().unwrap().len(), 1);
    let (_, _, body) = server.admin_get("/api/refresh", &admin).await;
    assert_eq!(json_body(&body)["nodes"], 1);
    task.abort();
}

#[tokio::test]
async fn structured_and_base64_inline_payloads_are_parsed_and_deduplicated() {
    let payload = r#"{"outbounds":[{"type":"trojan","tag":"remote","server":"example.invalid","server_port":443,"password":"change-me","tls":{"enabled":true,"server_name":"tls.example.invalid"},"transport":{"type":"httpupgrade","host":"front.example.invalid","path":"/ws"}}]}"#;
    let encoded = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(VLESS_LINK);
    let server = TestServer::with_links(&[payload.into(), payload.into(), encoded]).await;
    let admin = server.admin_token().await;
    server
        .create_token(&admin, "mixed-structured", "pw", 1_000_000)
        .await;
    let (status, headers, body) = server
        .get_basic("/sub/singbox", "mixed-structured", "pw")
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(header_value(&headers, "x-vpn233-nodes"), "2");
    assert_eq!(header_value(&headers, "x-vpn233-rendered"), "2");
    let body = json_body(&body);
    let outbound = body["outbounds"]
        .as_array()
        .unwrap()
        .iter()
        .find(|node| node["tag"] == "remote")
        .unwrap();
    assert_eq!(outbound["transport"]["type"], "httpupgrade");
    assert_eq!(outbound["transport"]["host"], "front.example.invalid");
}

#[tokio::test]
async fn structured_inventory_payloads_can_be_selected_and_removed_independently() {
    let payload = "proxies:\n- name: WG synthetic\n  type: wireguard\n  private-key: change-me\n  peers:\n  - {server: example.invalid, port: 51820, public-key: change-me, allowed-ips: [0.0.0.0/0]}\n- {name: Mieru synthetic, type: mieru, server: example.invalid, port-range: 2090-2099, transport: TCP, username: change-me, password: change-me}";
    let server = TestServer::with_links(&[payload.into()]).await;
    let admin = server.admin_token().await;
    let (status, _, body) = server.admin_get("/api/nodes", &admin).await;
    assert_eq!(status, StatusCode::OK);
    let nodes = json_body(&body);
    let nodes = nodes.as_array().unwrap();
    assert_eq!(nodes.len(), 2);
    assert_eq!(nodes[0]["endpoint"], "example.invalid:51820");
    assert_eq!(nodes[1]["endpoint"], "example.invalid:2090-2099");
    let selections: Vec<&str> = nodes
        .iter()
        .map(|node| node["payload"].as_str().unwrap())
        .collect();
    assert_ne!(selections[0], selections[1]);
    for selection in &selections {
        assert!(!selection.contains('\n'));
        let (parsed, errors) = vpn233_core_lib_rust::parse_subscription(selection);
        assert!(errors.is_empty());
        assert_eq!(parsed.len(), 1);
    }
    let (status, _, body) = server.admin_post("/api/admin/groups", &admin, &json!({
        "group_id": "structured-inline", "name": "Structured inline", "node_links": [payload],
    })).await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(json_body(&body)["group"]["node_count"], 2);
    let (status, _, body) = server
        .admin_post(
            "/api/admin/groups",
            &admin,
            &json!({
                "group_id": "structured-group", "name": "Structured", "node_links": selections,
            }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(json_body(&body)["group"]["node_count"], 2);
    server
        .create_account_link_with_group(&admin, "structured-group-token", "pw", "structured-group")
        .await;
    let (status, headers, body) = server
        .get_basic("/sub/mihomo", "structured-group-token", "pw")
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(header_value(&headers, "x-vpn233-rendered"), "2");
    let yaml: serde_yaml::Value = serde_yaml::from_slice(&body).unwrap();
    assert_eq!(yaml["proxies"][0]["peers"][0]["port"], 51820);
    assert_eq!(yaml["proxies"][1]["port-range"], "2090-2099");
    let (status, _, _) = server
        .admin_patch(
            "/api/admin/groups/structured-group",
            &admin,
            &json!({
                "node_links": [selections[1]],
            }),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let (_, headers, body) = server
        .get_basic("/sub/mihomo", "structured-group-token", "pw")
        .await;
    assert_eq!(header_value(&headers, "x-vpn233-rendered"), "1");
    let yaml: serde_yaml::Value = serde_yaml::from_slice(&body).unwrap();
    assert_eq!(yaml["proxies"][0]["type"], "mieru");
}

#[tokio::test]
async fn corrupted_persisted_stores_fail_closed_without_overwriting_data() {
    for filename in [
        "usage.json",
        "accounts.json",
        "groups.json",
        "providers.json",
    ] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(filename);
        let original = b"private-data-change-me invalid JSON";
        tokio::fs::write(&path, original).await.unwrap();
        let mut config = Config::default();
        config.server.data_dir = dir.path().to_string_lossy().into_owned();
        assert!(init_state(config).await.is_err(), "{filename}");
        assert_eq!(tokio::fs::read(path).await.unwrap(), original);
    }
}

#[tokio::test]
async fn expired_token_subscription_returns_401() {
    let server = TestServer::with_links(&all_sample_links()).await;
    let admin = server.admin_token().await;
    let (status, _, _) = server
        .admin_post(
            "/api/admin/tokens",
            &admin,
            &json!({
                "token": "expired-sub",
                "password": "pw",
                "limit_bytes": 1000,
                "expires_at": 1,
            }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED);
    let (status, _, _) = server.get("/sub/links?token=expired-sub&password=pw").await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

// ---------------------------------------------------------------------------
// Public format capabilities and security headers
// ---------------------------------------------------------------------------

#[tokio::test]
async fn formats_endpoint_is_public_and_cacheable() {
    let server = TestServer::new().await;
    let (status, headers, body) = server.get("/api/v1/formats").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        headers
            .get(header::CACHE_CONTROL)
            .and_then(|value| value.to_str().ok()),
        Some("public, max-age=300")
    );
    let value = json_body(&body);
    let formats = value["formats"].as_array().expect("formats");
    assert!(formats.iter().any(|format| format["id"] == "clash"));
    assert!(formats.iter().any(|format| format["recommended"] == true));
    let protocols = value["protocols"].as_array().expect("protocols");
    assert!(protocols.iter().any(|protocol| protocol == "vless"));
    for format in formats {
        for field in ["id", "name", "content_type", "extension", "protocols"] {
            assert!(!format[field].is_null(), "{field} missing from {format}");
        }
    }
}

#[tokio::test]
async fn security_headers_are_present_on_every_response() {
    let server = TestServer::new().await;
    for path in ["/api/status", "/api/v1/formats"] {
        let (_, headers, _) = server.get(path).await;
        let header_value = |name: &str| {
            headers
                .get(name)
                .and_then(|value| value.to_str().ok())
                .map(str::to_owned)
        };
        assert_eq!(
            header_value("x-content-type-options").as_deref(),
            Some("nosniff"),
            "path {path}"
        );
        assert_eq!(
            header_value("x-frame-options").as_deref(),
            Some("DENY"),
            "path {path}"
        );
        assert_eq!(
            header_value("referrer-policy").as_deref(),
            Some("no-referrer"),
            "path {path}"
        );
        assert_eq!(
            header_value("permissions-policy").as_deref(),
            Some("interest-cohort=()"),
            "path {path}"
        );
        let csp = header_value("content-security-policy").unwrap_or_default();
        assert!(csp.contains("frame-ancestors 'none'"), "path {path}");
        assert!(csp.contains("default-src 'self'"), "path {path}");
    }
}

// ---------------------------------------------------------------------------
// Persistence and legacy data compatibility
// ---------------------------------------------------------------------------

#[tokio::test]
async fn state_persists_tokens_across_restarts() {
    let dir = tempfile::tempdir().expect("temp dir");
    let password = "persisted-password";
    {
        let server = TestServer::with_dir(dir.path(), |_| {}).await;
        let admin = server.admin_token().await;
        let (status, _, _) = server
            .admin_post(
                "/api/admin/tokens",
                &admin,
                &json!({
                    "token": "persisted",
                    "password": password,
                    "limit_bytes": 10_000,
                    "label": "Persisted",
                }),
            )
            .await;
        assert_eq!(status, StatusCode::CREATED);
    }
    let server = TestServer::with_dir(dir.path(), |_| {}).await;
    let admin = server.admin_token().await;
    let tokens = server.token_list(&admin).await;
    let token = find_token(&tokens, "persisted");
    assert_eq!(token["label"], "Persisted");
    assert_eq!(token["limit_bytes"], 10_000);
}

#[tokio::test]
async fn persisted_providers_are_reloaded() {
    let dir = tempfile::tempdir().expect("temp dir");
    {
        let server = TestServer::with_dir(dir.path(), |_| {}).await;
        server
            .register_provider("provider-persisted", &[VLESS_LINK.to_owned()])
            .await;
    }
    let server = TestServer::with_dir(dir.path(), |_| {}).await;
    let admin = server.admin_token().await;
    let (status, _, body) = server.admin_get("/api/admin/providers", &admin).await;
    assert_eq!(status, StatusCode::OK);
    let value = json_body(&body);
    let providers = value["providers"].as_array().expect("providers");
    assert_eq!(providers.len(), 1);
    assert_eq!(providers[0]["provider_id"], "provider-persisted");
}

#[tokio::test]
async fn legacy_usage_json_loads_with_defaults() {
    let dir = tempfile::tempdir().expect("temp dir");
    let legacy = json!({
        "tokens": {
            "legacy-token": {
                "password_salt": "",
                "password_hash": "",
                "limit_bytes": 1234,
                "used_bytes": 0,
                "requests": 0,
                "enabled": true,
                "expires_at": null,
                "last_seen": null
            }
        }
    });
    std::fs::write(dir.path().join("usage.json"), legacy.to_string()).expect("write legacy usage");
    let server = TestServer::with_dir(dir.path(), |_| {}).await;
    let admin = server.admin_token().await;
    let tokens = server.token_list(&admin).await;
    let token = find_token(&tokens, "legacy-token");
    assert_eq!(token["quota_period"], "lifetime");
    assert_eq!(token["account_id"], "legacy");
    assert_eq!(token["group_id"], "all");
    assert!(token["created_at"].as_u64().unwrap_or_default() > 0);
}

#[tokio::test]
async fn default_accounts_and_groups_are_materialized() {
    let server = TestServer::new().await;
    let admin = server.admin_token().await;
    let (status, _, body) = server.admin_get("/api/admin/accounts", &admin).await;
    assert_eq!(status, StatusCode::OK);
    let accounts = json_body(&body);
    assert!(accounts["accounts"]
        .as_array()
        .expect("accounts")
        .iter()
        .any(|account| account["account_id"] == "legacy" && account["enabled"] == true));
    let (status, _, body) = server.admin_get("/api/admin/groups", &admin).await;
    assert_eq!(status, StatusCode::OK);
    let groups = json_body(&body);
    let all = groups["groups"]
        .as_array()
        .expect("groups")
        .iter()
        .find(|group| group["group_id"] == "all")
        .expect("all group");
    assert_eq!(all["protected"], true);
}

#[tokio::test]
async fn config_defaults_are_stable() {
    let config = Config::default();
    assert_eq!(config.server.host, "0.0.0.0");
    assert_eq!(config.server.http_port, 23444);
    assert_eq!(config.server.default_format, "clash");
    assert_eq!(config.server.provider_stale_after_secs, 120);
    assert_eq!(config.server.traffic_timezone_offset_hours, 8);
    assert!(config.subscriptions.is_empty());
}

#[tokio::test]
async fn invalid_credentials_never_leak_secret_material() {
    let server = TestServer::new().await;
    let (status, _, body) = server.login(ADMIN_USER, "wrong", None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert!(body.is_empty(), "error body must stay empty");
}

#[tokio::test]
async fn access_token_jwt_survives_a_restart() {
    let dir = tempfile::tempdir().expect("temp dir");
    let (token, access_token) = {
        let server = TestServer::with_dir(dir.path(), |config| {
            config.subscriptions = vec![SubscriptionSource {
                name: "alpha".into(),
                links: all_sample_links(),
                url: String::new(),
                access_token: String::new(),
            }];
        })
        .await;
        let admin = server.admin_token().await;
        server
            .create_token(&admin, "restart-jwt", "pw", 10_000_000)
            .await;
        let (status, _, body) = server
            .admin_post("/api/admin/tokens/restart-jwt/link", &admin, &json!({}))
            .await;
        assert_eq!(status, StatusCode::OK);
        let access_token = json_body(&body)["access_token"]
            .as_str()
            .expect("access token")
            .to_owned();
        ("restart-jwt".to_owned(), access_token)
    };
    let server = TestServer::with_dir(dir.path(), |_| {}).await;
    let (status, _, _) = server
        .get(&format!(
            "/sub/links?token={token}&access_token={access_token}"
        ))
        .await;
    assert_eq!(status, StatusCode::OK);
}
