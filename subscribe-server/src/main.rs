use anyhow::{Context, Result};
use argon2::{
    password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
    Argon2,
};
use axum::{
    extract::{Path, Query, State},
    http::{header, HeaderMap, HeaderValue, Method, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, patch, post},
    Json, Router,
};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use clap::{Parser, Subcommand};
use jsonwebtoken::{decode, encode, Algorithm, DecodingKey, EncodingKey, Header, Validation};
use rand_core::OsRng;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::process::Command as ProcessCommand;
use std::{
    collections::HashMap,
    collections::HashSet,
    net::SocketAddr,
    path::{Path as FsPath, PathBuf},
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};
use tokio::{fs, sync::RwLock};
use tower_http::cors::CorsLayer;
use tracing::{info, warn};
use url::Url;
use uuid::Uuid;
use vpn233_core_lib_rust::{boolean, proxy_node, render_clash_config, string, ProxyNode};

#[derive(Parser, Debug)]
#[command(name = "vpn233-subscribe-server", version)]
struct Args {
    #[arg(
        short,
        long,
        env = "VPN233_SUBSCRIBE_CONFIG",
        default_value = "subscribe-server/config.yaml"
    )]
    config: PathBuf,
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand, Debug)]
enum Command {
    Update,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct Config {
    server: ServerConfig,
    #[serde(default)]
    subscriptions: Vec<SubscriptionSource>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct ServerConfig {
    #[serde(default = "default_host")]
    host: String,
    #[serde(default = "default_port")]
    http_port: u16,
    #[serde(default = "default_format")]
    default_format: String,
    #[serde(default = "default_data_dir")]
    data_dir: String,
    #[serde(default = "default_admin_user")]
    admin_user: String,
    #[serde(default = "default_admin_password")]
    admin_password: String,
    #[serde(default = "default_admin_origins")]
    admin_origins: String,
    #[serde(default = "default_web_dir")]
    web_dir: String,
    #[serde(default)]
    provider_registry_token: String,
    #[serde(default = "default_provider_stale_after_secs")]
    provider_stale_after_secs: u64,
}

const ADMIN_JWT_TTL_SECS: u64 = 365 * 24 * 60 * 60;
const SUBSCRIPTION_JWT_TTL_SECS: u64 = 10 * 365 * 24 * 60 * 60;

#[derive(Debug, Clone, Deserialize, Serialize)]
struct AdminClaims {
    sub: String,
    iat: u64,
    exp: u64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct SubscriptionClaims {
    sub: String,
    token: String,
    iat: u64,
    exp: u64,
}

fn admin_jwt_secret(config: &ServerConfig) -> Vec<u8> {
    format!(
        "vpn233-admin-jwt-v1:{}:{}",
        config.admin_user, config.admin_password
    )
    .into_bytes()
}

fn issue_admin_jwt(config: &ServerConfig, now: u64) -> Result<String, jsonwebtoken::errors::Error> {
    let claims = AdminClaims {
        sub: config.admin_user.clone(),
        iat: now,
        exp: now.saturating_add(ADMIN_JWT_TTL_SECS),
    };
    encode(
        &Header::new(Algorithm::HS256),
        &claims,
        &EncodingKey::from_secret(&admin_jwt_secret(config)),
    )
}

fn verify_admin_jwt(config: &ServerConfig, token: &str) -> bool {
    let validation = Validation::new(Algorithm::HS256);
    decode::<AdminClaims>(
        token,
        &DecodingKey::from_secret(&admin_jwt_secret(config)),
        &validation,
    )
    .is_ok_and(|data| data.claims.sub == config.admin_user)
}

fn subscription_jwt_secret(config: &ServerConfig) -> Vec<u8> {
    format!(
        "vpn233-subscription-link-v1:{}:{}",
        config.admin_user, config.admin_password
    )
    .into_bytes()
}

fn issue_subscription_jwt(
    config: &ServerConfig,
    token: &str,
    created_at: u64,
) -> Result<String, jsonwebtoken::errors::Error> {
    let claims = SubscriptionClaims {
        sub: "vpn233-subscription".into(),
        token: token.into(),
        iat: created_at,
        exp: created_at.saturating_add(SUBSCRIPTION_JWT_TTL_SECS),
    };
    encode(
        &Header::new(Algorithm::HS256),
        &claims,
        &EncodingKey::from_secret(&subscription_jwt_secret(config)),
    )
}

fn verify_subscription_jwt(config: &ServerConfig, token: &str, access_token: &str) -> bool {
    let validation = Validation::new(Algorithm::HS256);
    decode::<SubscriptionClaims>(
        access_token,
        &DecodingKey::from_secret(&subscription_jwt_secret(config)),
        &validation,
    )
    .is_ok_and(|data| data.claims.sub == "vpn233-subscription" && data.claims.token == token)
}

fn default_host() -> String {
    "0.0.0.0".into()
}
fn default_port() -> u16 {
    23444
}
fn default_format() -> String {
    "clash".into()
}
fn default_data_dir() -> String {
    "./subscribe-server/data".into()
}
fn default_admin_user() -> String {
    "root".into()
}
fn default_admin_password() -> String {
    "root".into()
}
fn default_admin_origins() -> String {
    "http://127.0.0.1:4173,http://localhost:4173".into()
}
fn default_web_dir() -> String {
    "./admin-web/dist".into()
}
fn default_provider_stale_after_secs() -> u64 {
    120
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct SubscriptionSource {
    name: String,
    #[serde(default)]
    links: Vec<String>,
    #[serde(default)]
    url: String,
    #[serde(default)]
    access_token: String,
}

#[derive(Clone)]
struct AppState {
    config: Arc<RwLock<Config>>,
    client: reqwest::Client,
    usage: Arc<RwLock<UsageStore>>,
    providers: Arc<RwLock<ProviderStore>>,
    accounts: Arc<RwLock<AccountStore>>,
    groups: Arc<RwLock<GroupStore>>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct UsageStore {
    tokens: HashMap<String, UsageRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct UsageRecord {
    password_salt: String,
    password_hash: String,
    limit_bytes: u64,
    used_bytes: u64,
    requests: u64,
    enabled: bool,
    expires_at: Option<u64>,
    last_seen: Option<u64>,
    #[serde(default)]
    created_at: u64,
    #[serde(default)]
    quota_period: QuotaPeriod,
    #[serde(default)]
    period_key: Option<String>,
    #[serde(default)]
    other_uid: Option<String>,
    #[serde(default)]
    disabled_reason: Option<String>,
    #[serde(default)]
    retired_at: Option<u64>,
    #[serde(default = "default_legacy_account_id")]
    account_id: String,
    #[serde(default = "default_all_group_id")]
    group_id: String,
    #[serde(default)]
    label: String,
}

fn default_legacy_account_id() -> String {
    "legacy".into()
}

fn default_all_group_id() -> String {
    "all".into()
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct AccountStore {
    accounts: HashMap<String, AccountRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct AccountRecord {
    account_id: String,
    username: String,
    display_name: String,
    enabled: bool,
    #[serde(default)]
    other_uid: Option<String>,
    created_at: u64,
    #[serde(default)]
    deleted_at: Option<u64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct GroupStore {
    groups: HashMap<String, SubscriptionGroup>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SubscriptionGroup {
    group_id: String,
    name: String,
    #[serde(default)]
    node_links: Vec<String>,
    #[serde(default)]
    protected: bool,
    created_at: u64,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
enum QuotaPeriod {
    #[default]
    Lifetime,
    #[serde(rename = "calendar_month", alias = "monthly")]
    CalendarMonth,
    #[serde(rename = "monthly_from_creation", alias = "creation_day")]
    MonthlyFromCreation,
    Weekly,
    Daily,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum AccessDuration {
    SevenDays,
    OneMonth,
    OneYear,
}

impl QuotaPeriod {
    fn as_str(self) -> &'static str {
        match self {
            Self::Lifetime => "lifetime",
            Self::CalendarMonth => "calendar_month",
            Self::MonthlyFromCreation => "monthly_from_creation",
            Self::Weekly => "weekly",
            Self::Daily => "daily",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
struct NodeInfo {
    name: String,
    protocol: String,
    server: String,
    port: u16,
    link: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ProviderAddress {
    kind: String,
    value: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct ProviderStore {
    providers: HashMap<String, ProviderRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ProviderRecord {
    provider_id: String,
    name: String,
    version: String,
    addresses: Vec<ProviderAddress>,
    node_links: Vec<String>,
    enabled: bool,
    registered_at: u64,
    last_seen: u64,
    heartbeat_interval_secs: u64,
    heartbeat_token_hash: String,
}

#[derive(Debug, Deserialize)]
struct RegistryRegister {
    provider_id: String,
    name: String,
    version: String,
    #[serde(default)]
    addresses: Vec<ProviderAddress>,
    #[serde(default)]
    node_links: Vec<String>,
    #[serde(default = "default_provider_heartbeat_secs")]
    heartbeat_interval_secs: u64,
}

#[derive(Debug, Deserialize)]
struct RegistryHeartbeat {
    provider_id: String,
    heartbeat_token: String,
    #[serde(default)]
    version: Option<String>,
    #[serde(default)]
    addresses: Option<Vec<ProviderAddress>>,
    #[serde(default)]
    node_links: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
struct RegistryIdentity {
    provider_id: String,
    heartbeat_token: String,
}

#[derive(Debug, Serialize)]
struct ProviderView {
    provider_id: String,
    name: String,
    version: String,
    addresses: Vec<ProviderAddress>,
    node_count: usize,
    enabled: bool,
    online: bool,
    registered_at: u64,
    last_seen: u64,
    heartbeat_interval_secs: u64,
}

fn default_provider_heartbeat_secs() -> u64 {
    30
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt().with_env_filter("info").init();
    let args = Args::parse();
    if matches!(args.command, Some(Command::Update)) {
        return update_self().await;
    }
    let config = load_config(&args.config).await?;
    let web_dir = config.server.web_dir.clone();
    let data_dir = PathBuf::from(&config.server.data_dir);
    fs::create_dir_all(&data_dir).await?;
    let usage = load_usage(&data_dir).await.unwrap_or_default();
    let accounts = load_accounts(&data_dir).await.unwrap_or_default();
    let groups = load_groups(&data_dir).await.unwrap_or_default();
    let providers = load_providers(&data_dir).await.unwrap_or_default();
    let origins = config.server.admin_origins.clone();
    let addr: SocketAddr = format!("{}:{}", config.server.host, config.server.http_port)
        .parse()
        .context("invalid subscribe listen address")?;
    let state = AppState {
        config: Arc::new(RwLock::new(config)),
        client: reqwest::Client::builder()
            .user_agent("vpn233-subscribe-server/2")
            .build()?,
        usage: Arc::new(RwLock::new(usage)),
        providers: Arc::new(RwLock::new(providers)),
        accounts: Arc::new(RwLock::new(accounts)),
        groups: Arc::new(RwLock::new(groups)),
    };
    let origins = origins
        .split(',')
        .filter_map(|origin| origin.trim().parse::<HeaderValue>().ok())
        .collect::<Vec<_>>();
    let cors = CorsLayer::new()
        .allow_origin(origins)
        .allow_methods([Method::GET, Method::POST, Method::PATCH, Method::DELETE])
        .allow_headers([header::AUTHORIZATION, header::CONTENT_TYPE]);
    let app = Router::new()
        .route("/sub", get(subscription_default))
        .route("/sub/c", get(subscription_clash))
        .route("/sub/c/{name}", get(subscription_clash_named))
        .route("/sub/links", get(subscription_links))
        .route("/sub/base64", get(subscription_base64))
        .route("/api/auth/login", post(admin_login))
        .route("/api/status", get(status))
        .route("/api/nodes", get(nodes))
        .route("/api/refresh", get(refresh))
        .route("/api/v1/registry/register", post(registry_register))
        .route("/api/v1/registry/heartbeat", post(registry_heartbeat))
        .route("/api/v1/registry/unregister", post(registry_unregister))
        .route("/api/admin/providers", get(admin_providers))
        .route(
            "/api/admin/providers/{provider_id}/disable",
            post(admin_disable_provider),
        )
        .route(
            "/api/admin/tokens",
            get(admin_tokens).post(admin_create_token),
        )
        .route(
            "/api/admin/accounts",
            get(admin_accounts).post(admin_create_account),
        )
        .route(
            "/api/admin/accounts/{account_id}",
            patch(admin_update_account).delete(admin_delete_account),
        )
        .route(
            "/api/admin/accounts/{account_id}/links",
            get(admin_account_links).post(admin_create_account_link),
        )
        .route(
            "/api/admin/accounts/{account_id}/links/{token}",
            patch(admin_update_account_link).delete(admin_delete_account_link),
        )
        .route(
            "/api/admin/accounts/{account_id}/links/{token}/rotate",
            post(admin_rotate_account_link),
        )
        .route(
            "/api/admin/groups",
            get(admin_groups).post(admin_create_group),
        )
        .route(
            "/api/admin/groups/{group_id}",
            patch(admin_update_group).delete(admin_delete_group),
        )
        .route("/api/admin/tokens/{token}", post(admin_disable_token))
        .route("/api/admin/tokens/{token}/renew", post(admin_renew_token))
        .route("/api/admin/tokens/{token}/reset", post(admin_reset_token))
        .route("/api/admin/tokens/{token}/rotate", post(admin_rotate_token))
        .route(
            "/api/admin/tokens/{token}/link",
            post(admin_subscription_link),
        )
        .fallback_service(tower_http::services::ServeDir::new(web_dir))
        .layer(cors)
        .with_state(state.clone());
    let listener = tokio::net::TcpListener::bind(addr).await?;
    info!(%addr, "vpn233 subscribe server listening");
    let reaper_state = state.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(60));
        loop {
            interval.tick().await;
            let now = unix_now();
            let changed = {
                let mut usage = reaper_state.usage.write().await;
                let mut changed = expire_tokens(&mut usage, now);
                for record in usage.tokens.values_mut() {
                    changed |= refresh_quota(record, now);
                }
                changed
            };
            if changed {
                if let Err(error) = save_usage(&reaper_state).await {
                    warn!(%error, "usage reaper persistence failed");
                }
            }
        }
    });
    axum::serve(listener, app).await?;
    Ok(())
}

async fn update_self() -> Result<()> {
    run_update()
}

fn run_update() -> Result<()> {
    let script = if cfg!(windows) {
        "update.ps1"
    } else {
        "update.sh"
    };
    if let Ok(path) = std::env::var("VPN233_UPDATE_SCRIPT") {
        let status = if cfg!(windows) {
            ProcessCommand::new("powershell")
                .args(["-NoProfile", "-File", &path])
                .status()?
        } else {
            ProcessCommand::new("sh").arg(path).status()?
        };
        anyhow::ensure!(status.success(), "update script failed with {status}");
        return Ok(());
    }
    let executable = std::env::current_exe()?;
    let root = executable
        .parent()
        .and_then(|path| path.parent())
        .context("cannot locate installation root")?;
    let path = root.join(script);
    anyhow::ensure!(path.exists(), "update script not found: {}", path.display());
    let status = if cfg!(windows) {
        ProcessCommand::new("powershell")
            .args(["-NoProfile", "-File"])
            .arg(path)
            .status()?
    } else {
        ProcessCommand::new("sh").arg(path).status()?
    };
    anyhow::ensure!(status.success(), "update script failed with {status}");
    Ok(())
}

async fn load_config(path: &PathBuf) -> Result<Config> {
    let mut config = if fs::try_exists(path).await? {
        serde_yaml::from_str::<Config>(&fs::read_to_string(path).await?)
            .context("invalid subscribe YAML")?
    } else {
        Config {
            server: ServerConfig {
                host: default_host(),
                http_port: default_port(),
                default_format: default_format(),
                data_dir: default_data_dir(),
                admin_user: default_admin_user(),
                admin_password: default_admin_password(),
                admin_origins: default_admin_origins(),
                web_dir: default_web_dir(),
                provider_registry_token: String::new(),
                provider_stale_after_secs: default_provider_stale_after_secs(),
            },
            subscriptions: vec![],
        }
    };
    if let Ok(user) = std::env::var("VPN233_ADMIN_USER") {
        config.server.admin_user = user;
    }
    if let Ok(password) = std::env::var("VPN233_ADMIN_PASSWORD") {
        config.server.admin_password = password;
    }
    if let Ok(origins) = std::env::var("VPN233_ADMIN_ORIGINS") {
        config.server.admin_origins = origins;
    }
    if let Ok(port) = std::env::var("VPN233_SUBSCRIBE_PORT") {
        config.server.http_port = port.parse().context("invalid VPN233_SUBSCRIBE_PORT")?;
    }
    if let Ok(dir) = std::env::var("VPN233_SUBSCRIBE_DATA_DIR") {
        config.server.data_dir = dir;
    }
    if let Ok(token) = std::env::var("VPN233_PROVIDER_REGISTRY_TOKEN") {
        config.server.provider_registry_token = token;
    }
    if let Ok(stale) = std::env::var("VPN233_PROVIDER_STALE_AFTER_SECS") {
        config.server.provider_stale_after_secs = stale
            .parse()
            .context("invalid VPN233_PROVIDER_STALE_AFTER_SECS")?;
    }
    Ok(config)
}

async fn load_usage(data_dir: &FsPath) -> Result<UsageStore> {
    let path = data_dir.join("usage.json");
    if !fs::try_exists(&path).await? {
        return Ok(UsageStore::default());
    }
    let mut store: UsageStore = serde_json::from_slice(&fs::read(path).await?)?;
    let now = unix_now();
    for record in store.tokens.values_mut() {
        if record.created_at == 0 {
            record.created_at = now;
        }
        if record.quota_period != QuotaPeriod::Lifetime && record.period_key.is_none() {
            record.period_key = quota_period_key(record, now);
        }
    }
    Ok(store)
}

async fn load_accounts(data_dir: &FsPath) -> Result<AccountStore> {
    let path = data_dir.join("accounts.json");
    let mut store = if fs::try_exists(&path).await? {
        serde_json::from_slice::<AccountStore>(&fs::read(&path).await?)?
    } else {
        AccountStore::default()
    };
    store
        .accounts
        .entry("legacy".into())
        .or_insert_with(|| AccountRecord {
            account_id: "legacy".into(),
            username: "legacy".into(),
            display_name: "Legacy subscriptions".into(),
            enabled: true,
            other_uid: None,
            created_at: unix_now(),
            deleted_at: None,
        });
    Ok(store)
}

async fn load_groups(data_dir: &FsPath) -> Result<GroupStore> {
    let path = data_dir.join("groups.json");
    let mut store = if fs::try_exists(&path).await? {
        serde_json::from_slice::<GroupStore>(&fs::read(&path).await?)?
    } else {
        GroupStore::default()
    };
    store
        .groups
        .entry("all".into())
        .or_insert_with(|| SubscriptionGroup {
            group_id: "all".into(),
            name: "all".into(),
            node_links: Vec::new(),
            protected: true,
            created_at: unix_now(),
        });
    if let Some(all) = store.groups.get_mut("all") {
        all.name = "all".into();
        all.protected = true;
        all.node_links.clear();
    }
    Ok(store)
}

async fn save_usage(state: &AppState) -> Result<()> {
    let dir = PathBuf::from(state.config.read().await.server.data_dir.clone());
    fs::create_dir_all(&dir).await?;
    let tmp = dir.join("usage.json.tmp");
    fs::write(&tmp, serde_json::to_vec_pretty(&*state.usage.read().await)?).await?;
    fs::rename(tmp, dir.join("usage.json")).await?;
    Ok(())
}

async fn load_providers(data_dir: &FsPath) -> Result<ProviderStore> {
    let path = data_dir.join("providers.json");
    if !fs::try_exists(&path).await? {
        return Ok(ProviderStore::default());
    }
    Ok(serde_json::from_slice(&fs::read(path).await?)?)
}

async fn save_providers(state: &AppState) -> Result<()> {
    let dir = PathBuf::from(state.config.read().await.server.data_dir.clone());
    fs::create_dir_all(&dir).await?;
    let tmp = dir.join("providers.json.tmp");
    fs::write(
        &tmp,
        serde_json::to_vec_pretty(&*state.providers.read().await)?,
    )
    .await?;
    fs::rename(tmp, dir.join("providers.json")).await?;
    Ok(())
}

async fn save_accounts(state: &AppState) -> Result<()> {
    let dir = PathBuf::from(state.config.read().await.server.data_dir.clone());
    fs::create_dir_all(&dir).await?;
    let tmp = dir.join("accounts.json.tmp");
    fs::write(
        &tmp,
        serde_json::to_vec_pretty(&*state.accounts.read().await)?,
    )
    .await?;
    fs::rename(tmp, dir.join("accounts.json")).await?;
    Ok(())
}

async fn save_groups(state: &AppState) -> Result<()> {
    let dir = PathBuf::from(state.config.read().await.server.data_dir.clone());
    fs::create_dir_all(&dir).await?;
    let tmp = dir.join("groups.json.tmp");
    fs::write(
        &tmp,
        serde_json::to_vec_pretty(&*state.groups.read().await)?,
    )
    .await?;
    fs::rename(tmp, dir.join("groups.json")).await?;
    Ok(())
}

async fn admin_authorized(state: &AppState, headers: &HeaderMap) -> bool {
    let config = state.config.read().await;
    let Some(value) = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
    else {
        return false;
    };
    if let Some(token) = value.strip_prefix("Bearer ") {
        return verify_admin_jwt(&config.server, token.trim());
    }
    let Some(encoded) = value.strip_prefix("Basic ") else {
        return false;
    };
    let Ok(decoded) = BASE64.decode(encoded) else {
        return false;
    };
    let Ok(credentials) = String::from_utf8(decoded) else {
        return false;
    };
    credentials
        == format!(
            "{}:{}",
            config.server.admin_user, config.server.admin_password
        )
}

async fn sources(state: &AppState, selected: Option<&str>) -> Vec<(String, String)> {
    let config = state.config.read().await.clone();
    let mut result = Vec::new();
    let mut seen_links = HashSet::new();
    for source in config
        .subscriptions
        .iter()
        .filter(|s| selected.map(|name| name == s.name).unwrap_or(true))
    {
        result.extend(
            source
                .links
                .iter()
                .filter(|link| !link.trim().is_empty())
                .filter(|link| seen_links.insert((*link).clone()))
                .map(|link| (source.name.clone(), link.clone())),
        );
        if !source.url.trim().is_empty() {
            let request = state.client.get(&source.url);
            let request = if source.access_token.trim().is_empty() {
                request
            } else {
                request.header("x-provider-token", source.access_token.trim())
            };
            match request.send().await.and_then(|r| r.error_for_status()) {
                Ok(response) => {
                    if let Ok(body) = response.text().await {
                        let decoded = BASE64
                            .decode(body.trim())
                            .ok()
                            .and_then(|bytes| String::from_utf8(bytes).ok())
                            .unwrap_or(body);
                        result.extend(
                            decoded
                                .lines()
                                .map(str::trim)
                                .filter(|line| line.contains("://"))
                                .filter(|link| seen_links.insert((*link).to_owned()))
                                .map(|link| (source.name.clone(), link.to_owned())),
                        );
                    }
                }
                Err(error) => {
                    warn!(source = %source.name, %error, "remote subscription fetch failed")
                }
            }
        }
    }
    let now = unix_now();
    let stale_after = config.server.provider_stale_after_secs;
    for provider in state.providers.read().await.providers.values() {
        if !provider.enabled
            || provider.last_seen.saturating_add(stale_after) <= now
            || selected.is_some_and(|name| name != provider.name)
        {
            continue;
        }
        result.extend(
            provider
                .node_links
                .iter()
                .filter(|link| !link.trim().is_empty())
                .filter(|link| seen_links.insert((*link).clone()))
                .map(|link| (provider.name.clone(), link.clone())),
        );
    }
    result
}

async fn group_sources(state: &AppState, group_id: &str) -> Vec<(String, String)> {
    if group_id == "all" {
        return sources(state, None).await;
    }
    let links = state
        .groups
        .read()
        .await
        .groups
        .get(group_id)
        .map(|group| group.node_links.clone())
        .unwrap_or_default();
    let mut seen = HashSet::new();
    links
        .into_iter()
        .filter(|link| seen.insert(link.clone()))
        .map(|link| (format!("group-{group_id}"), link))
        .collect()
}

async fn token_group_id(state: &AppState, token: &str) -> String {
    state
        .usage
        .read()
        .await
        .tokens
        .get(token)
        .map(|record| record.group_id.clone())
        .unwrap_or_else(default_all_group_id)
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn civil_date(timestamp: u64) -> (i64, u32, u32) {
    let days = timestamp / 86_400;
    let mut year = 1970_i64;
    let mut remaining = days as i64;
    loop {
        let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
        let year_days = if leap { 366 } else { 365 };
        if remaining < year_days {
            break;
        }
        remaining -= year_days;
        year += 1;
    }
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let month_days = [
        31_i64,
        if leap { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    let mut month = 0_usize;
    while month < month_days.len() && remaining >= month_days[month] {
        remaining -= month_days[month];
        month += 1;
    }
    (year, (month + 1) as u32, (remaining + 1) as u32)
}

fn month_key(timestamp: u64) -> String {
    let (year, month, _) = civil_date(timestamp);
    format!("{year:04}-{month:02}")
}

fn month_index(timestamp: u64) -> i64 {
    let (year, month, _) = civil_date(timestamp);
    year * 12 + month as i64 - 1
}

fn days_in_month(year: i64, month: u32) -> u32 {
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    match month {
        2 if leap => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

fn add_months(timestamp: u64, months: i64) -> u64 {
    let (_, _, day) = civil_date(timestamp);
    let index = month_index(timestamp) + months;
    let target_year = index.div_euclid(12);
    let target_month = (index.rem_euclid(12) + 1) as u32;
    let target_day = day.min(days_in_month(target_year, target_month));
    let target_date = timestamp_for_date(target_year, target_month, target_day);
    target_date + timestamp % 86_400
}

fn timestamp_for_date(year: i64, month: u32, day: u32) -> u64 {
    let mut days = 0_i64;
    for current_year in 1970..year {
        days += if current_year % 4 == 0 && (current_year % 100 != 0 || current_year % 400 == 0) {
            366
        } else {
            365
        };
    }
    for current_month in 1..month {
        days += days_in_month(year, current_month) as i64;
    }
    (days + day as i64 - 1) as u64 * 86_400
}

fn expiry_for_duration(now: u64, duration: AccessDuration) -> u64 {
    match duration {
        AccessDuration::SevenDays => now.saturating_add(7 * 86_400),
        AccessDuration::OneMonth => add_months(now, 1),
        AccessDuration::OneYear => add_months(now, 12),
    }
}

fn quota_period_key(record: &UsageRecord, timestamp: u64) -> Option<String> {
    match record.quota_period {
        QuotaPeriod::Lifetime => None,
        QuotaPeriod::CalendarMonth => Some(format!("calendar:{}", month_key(timestamp))),
        QuotaPeriod::MonthlyFromCreation => {
            let (created_year, created_month, created_day) = civil_date(record.created_at);
            let (current_year, current_month, current_day) = civil_date(timestamp);
            let mut cycle = month_index(timestamp) - month_index(record.created_at);
            if current_day < created_day.min(days_in_month(current_year, current_month)) {
                cycle -= 1;
            }
            Some(format!(
                "creation-month:{created_year:04}-{created_month:02}-{created_day:02}:{cycle}"
            ))
        }
        QuotaPeriod::Weekly => Some(format!(
            "weekly:{}",
            timestamp.saturating_sub(record.created_at) / (7 * 86_400)
        )),
        QuotaPeriod::Daily => Some(format!(
            "daily:{}",
            timestamp.saturating_sub(record.created_at) / 86_400
        )),
    }
}

fn refresh_quota(record: &mut UsageRecord, now: u64) -> bool {
    let Some(current) = quota_period_key(record, now) else {
        return false;
    };
    if record.period_key.as_deref() == Some(current.as_str()) {
        return false;
    }
    record.used_bytes = 0;
    record.requests = 0;
    record.period_key = Some(current);
    true
}

fn expire_tokens(store: &mut UsageStore, now: u64) -> bool {
    let mut changed = false;
    for record in store.tokens.values_mut() {
        if record.enabled && record.expires_at.is_some_and(|expires| expires <= now) {
            record.enabled = false;
            record.disabled_reason = Some("expired".into());
            record.retired_at = Some(now);
            changed = true;
        }
    }
    changed
}

fn next_reset_at(record: &UsageRecord, now: u64) -> Option<u64> {
    match record.quota_period {
        QuotaPeriod::Lifetime => None,
        QuotaPeriod::Daily => Some(
            record
                .created_at
                .saturating_add((now.saturating_sub(record.created_at) / 86_400 + 1) * 86_400),
        ),
        QuotaPeriod::Weekly => Some(record.created_at.saturating_add(
            (now.saturating_sub(record.created_at) / (7 * 86_400) + 1) * 7 * 86_400,
        )),
        QuotaPeriod::CalendarMonth | QuotaPeriod::MonthlyFromCreation => {
            let current = quota_period_key(record, now)?;
            let mut candidate = (now / 86_400 + 1) * 86_400;
            for _ in 0..370 {
                if quota_period_key(record, candidate).as_deref() != Some(current.as_str()) {
                    return Some(candidate);
                }
                candidate = candidate.saturating_add(86_400);
            }
            None
        }
    }
}

async fn token_allowed(
    state: &AppState,
    headers: &HeaderMap,
    query: &HashMap<String, String>,
) -> Option<String> {
    let basic = headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Basic "))
        .and_then(|value| BASE64.decode(value).ok())
        .and_then(|value| String::from_utf8(value).ok())
        .and_then(|value| {
            value
                .split_once(':')
                .map(|(token, password)| (token.to_owned(), password.to_owned()))
        });
    let candidate = query
        .get("token")
        .cloned()
        .or_else(|| {
            headers
                .get("x-access-token")
                .and_then(|v| v.to_str().ok())
                .map(str::to_owned)
        })
        .or_else(|| basic.as_ref().map(|(token, _)| token.clone()));
    let token = candidate?;
    let access_token = query.get("access_token").cloned().or_else(|| {
        headers
            .get("x-access-jwt")
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned)
    });
    let password = query
        .get("password")
        .cloned()
        .or_else(|| {
            headers
                .get("x-access-password")
                .and_then(|v| v.to_str().ok())
                .map(str::to_owned)
        })
        .or_else(|| basic.map(|(_, password)| password));
    if access_token.is_none() && password.is_none() {
        return None;
    }
    let jwt_authorized = if let Some(access_token) = access_token.as_deref() {
        let config = state.config.read().await;
        verify_subscription_jwt(&config.server, &token, access_token)
    } else {
        false
    };
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let mut usage = state.usage.write().await;
    let mut changed = expire_tokens(&mut usage, now);
    let (authorized, account_id) = if let Some(record) = usage.tokens.get_mut(&token) {
        changed |= refresh_quota(record, now);
        (
            record.enabled
                && record.expires_at.is_none_or(|expires| expires > now)
                && (jwt_authorized
                    || password.as_deref().is_some_and(|password| {
                        verify_password(password, &record.password_salt, &record.password_hash)
                    })),
            record.account_id.clone(),
        )
    } else {
        (false, default_legacy_account_id())
    };
    drop(usage);
    let account_enabled = state
        .accounts
        .read()
        .await
        .accounts
        .get(&account_id)
        .is_some_and(|account| account.enabled && account.deleted_at.is_none());
    let authorized = authorized && account_enabled;
    if changed {
        if let Err(error) = save_usage(state).await {
            warn!(%error, "usage persistence failed");
        }
    }
    authorized.then_some(token)
}

async fn charge_token(state: &AppState, token: &str, body_bytes: u64) -> bool {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let mut usage = state.usage.write().await;
    let mut changed = expire_tokens(&mut usage, now);
    let Some(record) = usage.tokens.get_mut(token) else {
        drop(usage);
        if changed {
            let _ = save_usage(state).await;
        }
        return false;
    };
    changed |= refresh_quota(record, now);
    if !record.enabled
        || record.expires_at.is_some_and(|expires| expires <= now)
        || record.used_bytes.saturating_add(body_bytes) > record.limit_bytes
    {
        drop(usage);
        if changed {
            let _ = save_usage(state).await;
        }
        return false;
    }
    record.used_bytes = record.used_bytes.saturating_add(body_bytes);
    record.requests = record.requests.saturating_add(1);
    record.last_seen = Some(now);
    drop(usage);
    if let Err(error) = save_usage(state).await {
        warn!(%error, "usage persistence failed");
    }
    true
}

async fn subscription_default(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<HashMap<String, String>>,
) -> Response {
    let format = state.config.read().await.server.default_format.clone();
    if format == "links" {
        return subscription_links(State(state), headers, Query(query)).await;
    }
    render_subscription(&state, &headers, &query, None).await
}

async fn subscription_clash(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<HashMap<String, String>>,
) -> Response {
    render_subscription(&state, &headers, &query, None).await
}
async fn subscription_clash_named(
    State(state): State<AppState>,
    Path(name): Path<String>,
    headers: HeaderMap,
    Query(query): Query<HashMap<String, String>>,
) -> Response {
    render_subscription(&state, &headers, &query, Some(name)).await
}

async fn render_subscription(
    state: &AppState,
    headers: &HeaderMap,
    query: &HashMap<String, String>,
    selected: Option<String>,
) -> Response {
    let Some(token) = token_allowed(state, headers, query).await else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    let group_id = token_group_id(state, &token).await;
    let links = if group_id == "all" {
        sources(state, selected.as_deref()).await
    } else {
        group_sources(state, &group_id).await
    };
    let nodes: Vec<ProxyNode> = links
        .iter()
        .enumerate()
        .filter_map(|(index, (_, link))| parse_link(link, index).ok())
        .collect();
    match render_clash_config(&nodes) {
        Ok(body) if charge_token(state, &token, body.len() as u64).await => {
            let filename = subscription_filename(query);
            subscription_response(state, &token, body, "text/yaml; charset=utf-8", &filename).await
        }
        Ok(_) => (StatusCode::TOO_MANY_REQUESTS, "subscription quota exceeded").into_response(),
        Err(error) => (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()).into_response(),
    }
}

async fn subscription_links(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<HashMap<String, String>>,
) -> Response {
    let Some(token) = token_allowed(&state, &headers, &query).await else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    let group_id = token_group_id(&state, &token).await;
    let links = if group_id == "all" {
        sources(&state, None).await
    } else {
        group_sources(&state, &group_id).await
    };
    let body = links
        .into_iter()
        .map(|(_, link)| link)
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";
    if !charge_token(&state, &token, body.len() as u64).await {
        return (StatusCode::TOO_MANY_REQUESTS, "subscription quota exceeded").into_response();
    }
    subscription_response(
        &state,
        &token,
        body,
        "text/plain; charset=utf-8",
        "vpn233.txt",
    )
    .await
}

async fn subscription_base64(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<HashMap<String, String>>,
) -> Response {
    let Some(token) = token_allowed(&state, &headers, &query).await else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    let group_id = token_group_id(&state, &token).await;
    let links = if group_id == "all" {
        sources(&state, None).await
    } else {
        group_sources(&state, &group_id).await
    };
    let body = links
        .into_iter()
        .map(|(_, link)| link)
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";
    if !charge_token(&state, &token, body.len() as u64).await {
        return (StatusCode::TOO_MANY_REQUESTS, "subscription quota exceeded").into_response();
    }
    subscription_response(
        &state,
        &token,
        format!("{}\n", BASE64.encode(body)),
        "text/plain; charset=utf-8",
        "vpn233.txt",
    )
    .await
}

fn subscription_filename(query: &HashMap<String, String>) -> String {
    let raw = query
        .get("filename")
        .or_else(|| query.get("profile"))
        .map(String::as_str)
        .unwrap_or("vpn233");
    let mut name: String = raw
        .trim()
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.' | ' ') {
                character
            } else {
                '_'
            }
        })
        .collect();
    name = name.trim_matches([' ', '.']).to_owned();
    if name.is_empty() {
        name = "vpn233".into();
    }
    if !name.to_ascii_lowercase().ends_with(".yaml") && !name.to_ascii_lowercase().ends_with(".yml")
    {
        name.push_str(".yaml");
    }
    name
}

async fn subscription_response(
    state: &AppState,
    token: &str,
    body: String,
    content_type: &'static str,
    filename: &str,
) -> Response {
    let usage = state.usage.read().await;
    let Some(record) = usage.tokens.get(token) else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    let mut headers = HeaderMap::new();
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_static(content_type));
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("no-store, private"),
    );
    headers.insert(header::PRAGMA, HeaderValue::from_static("no-cache"));
    let userinfo = format!(
        "upload=0; download={}; total={}; expire={}",
        record.used_bytes,
        record.limit_bytes,
        record.expires_at.unwrap_or(0)
    );
    if let Ok(value) = HeaderValue::from_str(&userinfo) {
        headers.insert("subscription-userinfo", value);
    }
    headers.insert("profile-update-interval", HeaderValue::from_static("3600"));
    if let Ok(value) = HeaderValue::from_str(&format!("attachment; filename=\"{filename}\"")) {
        headers.insert(header::CONTENT_DISPOSITION, value);
    }
    if let Ok(value) =
        HeaderValue::from_str(filename.trim_end_matches(".yaml").trim_end_matches(".yml"))
    {
        headers.insert("profile-title", value);
    }
    (headers, body).into_response()
}

#[derive(Debug, Deserialize)]
struct AdminLogin {
    username: String,
    password: String,
}

async fn admin_login(State(state): State<AppState>, Json(input): Json<AdminLogin>) -> Response {
    let config = state.config.read().await;
    if input.username != config.server.admin_user || input.password != config.server.admin_password
    {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let now = unix_now();
    match issue_admin_jwt(&config.server, now) {
        Ok(token) => Json(json!({
            "token": token,
            "user": config.server.admin_user,
            "expires_at": now.saturating_add(ADMIN_JWT_TTL_SECS),
        }))
        .into_response(),
        Err(error) => {
            tracing::error!(%error, "admin JWT issuance failed");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

async fn status(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(_query): Query<HashMap<String, String>>,
) -> Response {
    if !admin_authorized(&state, &headers).await {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    Json(json!({"ok": true, "service": "vpn233-subscribe-server", "version": env!("CARGO_PKG_VERSION")})).into_response()
}

async fn nodes(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<HashMap<String, String>>,
) -> Response {
    if !admin_authorized(&state, &headers).await {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let links = sources(&state, query.get("group").map(String::as_str)).await;
    let result: Vec<NodeInfo> = links
        .iter()
        .enumerate()
        .filter_map(|(index, (_, link))| {
            parse_link(link, index).ok().and_then(|n| {
                Some(NodeInfo {
                    name: n.get("name")?.as_str()?.into(),
                    protocol: n.get("type")?.as_str()?.into(),
                    server: n.get("server")?.as_str()?.into(),
                    port: n.get("port")?.as_u64()? as u16,
                    link: link.to_owned(),
                })
            })
        })
        .collect();
    Json(result).into_response()
}

async fn refresh(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(_query): Query<HashMap<String, String>>,
) -> Response {
    if !admin_authorized(&state, &headers).await {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    Json(json!({"ok": true, "nodes": sources(&state, None).await.len()})).into_response()
}

fn registry_token(headers: &HeaderMap) -> Option<&str> {
    headers
        .get("x-provider-registry-token")
        .and_then(|value| value.to_str().ok())
        .or_else(|| {
            headers
                .get(header::AUTHORIZATION)
                .and_then(|value| value.to_str().ok())
                .and_then(|value| value.strip_prefix("Bearer "))
        })
}

async fn registry_authorized(state: &AppState, headers: &HeaderMap) -> bool {
    let configured = state
        .config
        .read()
        .await
        .server
        .provider_registry_token
        .clone();
    !configured.is_empty() && registry_token(headers) == Some(configured.as_str())
}

fn hash_heartbeat_token(token: &str) -> Result<String> {
    let salt = SaltString::generate(&mut OsRng);
    Ok(Argon2::default()
        .hash_password(token.as_bytes(), &salt)
        .map_err(|error| anyhow::anyhow!("heartbeat token hashing failed: {error:?}"))?
        .to_string())
}

fn verify_heartbeat_token(token: &str, hash: &str) -> bool {
    PasswordHash::new(hash).ok().is_some_and(|parsed| {
        Argon2::default()
            .verify_password(token.as_bytes(), &parsed)
            .is_ok()
    })
}

fn valid_provider_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.'))
}

fn normalize_addresses(addresses: Vec<ProviderAddress>) -> Vec<ProviderAddress> {
    addresses
        .into_iter()
        .filter(|address| {
            matches!(address.kind.as_str(), "domain" | "ip" | "local")
                && !address.value.trim().is_empty()
                && address.value.len() <= 255
        })
        .map(|mut address| {
            address.kind = address.kind.trim().to_lowercase();
            address.value = address.value.trim().to_owned();
            address
        })
        .take(16)
        .collect()
}

fn normalize_links(links: Vec<String>) -> Vec<String> {
    links
        .into_iter()
        .map(|link| link.trim().to_owned())
        .filter(|link| link.contains("://") && link.len() <= 8192)
        .take(256)
        .collect()
}

async fn registry_register(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<RegistryRegister>,
) -> Response {
    if !registry_authorized(&state, &headers).await {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    if !valid_provider_id(&input.provider_id) || input.name.trim().is_empty() {
        return (StatusCode::BAD_REQUEST, "provider_id and name are required").into_response();
    }
    let provider_id = input.provider_id.clone();
    let heartbeat_interval_secs = input.heartbeat_interval_secs.clamp(10, 3600);
    if state
        .providers
        .read()
        .await
        .providers
        .get(&provider_id)
        .is_some_and(|provider| !provider.enabled)
    {
        return StatusCode::FORBIDDEN.into_response();
    }
    let heartbeat_token = Uuid::new_v4().to_string();
    let now = unix_now();
    let record = ProviderRecord {
        provider_id: provider_id.clone(),
        name: input.name.trim().chars().take(128).collect(),
        version: input.version.trim().chars().take(64).collect(),
        addresses: normalize_addresses(input.addresses),
        node_links: normalize_links(input.node_links),
        enabled: true,
        registered_at: now,
        last_seen: now,
        heartbeat_interval_secs,
        heartbeat_token_hash: match hash_heartbeat_token(&heartbeat_token) {
            Ok(value) => value,
            Err(error) => {
                return (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()).into_response()
            }
        },
    };
    state
        .providers
        .write()
        .await
        .providers
        .insert(provider_id.clone(), record);
    if let Err(error) = save_providers(&state).await {
        return (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()).into_response();
    }
    Json(json!({
        "provider_id": provider_id,
        "heartbeat_token": heartbeat_token,
        "heartbeat_interval_secs": heartbeat_interval_secs,
    }))
    .into_response()
}

async fn registry_heartbeat(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<RegistryHeartbeat>,
) -> Response {
    if !registry_authorized(&state, &headers).await {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let now = unix_now();
    let mut providers = state.providers.write().await;
    let Some(record) = providers.providers.get_mut(&input.provider_id) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    if !verify_heartbeat_token(&input.heartbeat_token, &record.heartbeat_token_hash) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    record.last_seen = now;
    if let Some(version) = input.version {
        record.version = version.trim().chars().take(64).collect();
    }
    if let Some(addresses) = input.addresses {
        record.addresses = normalize_addresses(addresses);
    }
    if let Some(links) = input.node_links {
        record.node_links = normalize_links(links);
    }
    drop(providers);
    if let Err(error) = save_providers(&state).await {
        return (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()).into_response();
    }
    Json(json!({"ok": true, "provider_id": input.provider_id, "last_seen": now})).into_response()
}

async fn registry_unregister(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<RegistryIdentity>,
) -> Response {
    if !registry_authorized(&state, &headers).await {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let mut providers = state.providers.write().await;
    let Some(record) = providers.providers.get_mut(&input.provider_id) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    if !verify_heartbeat_token(&input.heartbeat_token, &record.heartbeat_token_hash) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    record.enabled = false;
    record.last_seen = unix_now();
    drop(providers);
    if let Err(error) = save_providers(&state).await {
        return (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()).into_response();
    }
    StatusCode::NO_CONTENT.into_response()
}

async fn admin_providers(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if !admin_authorized(&state, &headers).await {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let stale_after = state.config.read().await.server.provider_stale_after_secs;
    let now = unix_now();
    let views = state
        .providers
        .read()
        .await
        .providers
        .values()
        .map(|provider| ProviderView {
            provider_id: provider.provider_id.clone(),
            name: provider.name.clone(),
            version: provider.version.clone(),
            addresses: provider.addresses.clone(),
            node_count: provider.node_links.len(),
            enabled: provider.enabled,
            online: provider.enabled && provider.last_seen.saturating_add(stale_after) > now,
            registered_at: provider.registered_at,
            last_seen: provider.last_seen,
            heartbeat_interval_secs: provider.heartbeat_interval_secs,
        })
        .collect::<Vec<_>>();
    Json(json!({"providers": views, "stale_after_secs": stale_after})).into_response()
}

async fn admin_disable_provider(
    State(state): State<AppState>,
    Path(provider_id): Path<String>,
    headers: HeaderMap,
) -> Response {
    if !admin_authorized(&state, &headers).await {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let mut providers = state.providers.write().await;
    let Some(record) = providers.providers.get_mut(&provider_id) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    record.enabled = false;
    drop(providers);
    if let Err(error) = save_providers(&state).await {
        return (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()).into_response();
    }
    StatusCode::NO_CONTENT.into_response()
}

async fn admin_tokens(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if !admin_authorized(&state, &headers).await {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let now = unix_now();
    let (views, changed) = {
        let mut usage = state.usage.write().await;
        let mut changed = expire_tokens(&mut usage, now);
        let views = usage
            .tokens
            .iter_mut()
            .map(|(token, record)| {
                changed |= refresh_quota(record, now);
                TokenView::from_pair(token, record, now)
            })
            .collect::<Vec<_>>();
        (views, changed)
    };
    if changed {
        if let Err(error) = save_usage(&state).await {
            warn!(%error, "usage persistence failed");
        }
    }
    Json(json!({"tokens": views})).into_response()
}

#[derive(Debug, Serialize)]
struct AccountView {
    account_id: String,
    username: String,
    display_name: String,
    enabled: bool,
    other_uid: Option<String>,
    created_at: u64,
    deleted_at: Option<u64>,
    links_count: usize,
    used_bytes: u64,
    limit_bytes: u64,
    requests: u64,
}

#[derive(Debug, Deserialize)]
struct CreateAccount {
    #[serde(default)]
    account_id: Option<String>,
    username: String,
    #[serde(default)]
    display_name: Option<String>,
    #[serde(default, alias = "otherUid")]
    other_uid: Option<String>,
}

#[derive(Debug, Deserialize)]
struct UpdateAccount {
    #[serde(default)]
    display_name: Option<String>,
    #[serde(default)]
    enabled: Option<bool>,
    #[serde(default, alias = "otherUid")]
    other_uid: Option<String>,
}

fn normalize_identifier(value: &str, fallback: &str) -> String {
    let normalized: String = value
        .trim()
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.'))
        .take(96)
        .collect();
    if normalized.is_empty() {
        fallback.into()
    } else {
        normalized
    }
}

fn normalize_label(value: Option<String>) -> String {
    value.unwrap_or_default().trim().chars().take(128).collect()
}

async fn admin_accounts(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if !admin_authorized(&state, &headers).await {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let usage = state.usage.read().await;
    let accounts = state.accounts.read().await;
    let views = accounts
        .accounts
        .values()
        .map(|account| {
            let records = usage
                .tokens
                .iter()
                .filter(|(_, record)| record.account_id == account.account_id);
            let mut view = AccountView {
                account_id: account.account_id.clone(),
                username: account.username.clone(),
                display_name: account.display_name.clone(),
                enabled: account.enabled,
                other_uid: account.other_uid.clone(),
                created_at: account.created_at,
                deleted_at: account.deleted_at,
                links_count: 0,
                used_bytes: 0,
                limit_bytes: 0,
                requests: 0,
            };
            for (_, record) in records {
                view.links_count += 1;
                view.used_bytes = view.used_bytes.saturating_add(record.used_bytes);
                view.limit_bytes = view.limit_bytes.saturating_add(record.limit_bytes);
                view.requests = view.requests.saturating_add(record.requests);
            }
            view
        })
        .collect::<Vec<_>>();
    Json(json!({"accounts": views})).into_response()
}

async fn admin_create_account(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<CreateAccount>,
) -> Response {
    if !admin_authorized(&state, &headers).await {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    if input.username.trim().is_empty() {
        return (StatusCode::BAD_REQUEST, "username is required").into_response();
    }
    let username = normalize_identifier(&input.username, "account");
    let account_id = normalize_identifier(
        input.account_id.as_deref().unwrap_or(&username),
        &format!("account-{}", Uuid::new_v4()),
    );
    let mut accounts = state.accounts.write().await;
    if accounts.accounts.contains_key(&account_id)
        || accounts
            .accounts
            .values()
            .any(|account| account.username == username)
    {
        return (StatusCode::CONFLICT, "account already exists").into_response();
    }
    let now = unix_now();
    let record = AccountRecord {
        account_id: account_id.clone(),
        username,
        display_name: input
            .display_name
            .unwrap_or_default()
            .trim()
            .chars()
            .take(128)
            .collect(),
        enabled: true,
        other_uid: normalize_other_uid(input.other_uid),
        created_at: now,
        deleted_at: None,
    };
    accounts.accounts.insert(account_id.clone(), record.clone());
    drop(accounts);
    if let Err(error) = save_accounts(&state).await {
        return (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()).into_response();
    }
    (StatusCode::CREATED, Json(json!({"account": record}))).into_response()
}

async fn admin_update_account(
    State(state): State<AppState>,
    Path(account_id): Path<String>,
    headers: HeaderMap,
    Json(input): Json<UpdateAccount>,
) -> Response {
    if !admin_authorized(&state, &headers).await {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    if account_id == "legacy" && input.enabled == Some(false) {
        return (StatusCode::BAD_REQUEST, "legacy account cannot be disabled").into_response();
    }
    let mut accounts = state.accounts.write().await;
    let Some(record) = accounts.accounts.get_mut(&account_id) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    if let Some(display_name) = input.display_name {
        record.display_name = display_name.trim().chars().take(128).collect();
    }
    if let Some(enabled) = input.enabled {
        record.enabled = enabled;
        if enabled {
            record.deleted_at = None;
        } else {
            record.deleted_at = Some(unix_now());
        }
    }
    if input.other_uid.is_some() {
        record.other_uid = normalize_other_uid(input.other_uid);
    }
    let response = record.clone();
    drop(accounts);
    if let Err(error) = save_accounts(&state).await {
        return (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()).into_response();
    }
    Json(json!({"account": response})).into_response()
}

async fn admin_delete_account(
    State(state): State<AppState>,
    Path(account_id): Path<String>,
    headers: HeaderMap,
) -> Response {
    if !admin_authorized(&state, &headers).await {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    if account_id == "legacy" {
        return (StatusCode::BAD_REQUEST, "legacy account cannot be deleted").into_response();
    }
    let mut accounts = state.accounts.write().await;
    let Some(record) = accounts.accounts.get_mut(&account_id) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    record.enabled = false;
    record.deleted_at = Some(unix_now());
    drop(accounts);
    if let Err(error) = save_accounts(&state).await {
        return (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()).into_response();
    }
    StatusCode::NO_CONTENT.into_response()
}

#[derive(Debug, Serialize)]
struct AccountLinkView {
    token: String,
    account_id: String,
    group_id: String,
    label: String,
    limit_bytes: u64,
    used_bytes: u64,
    requests: u64,
    enabled: bool,
    expires_at: Option<u64>,
    created_at: u64,
    quota_period: &'static str,
    next_reset_at: Option<u64>,
    other_uid: Option<String>,
    disabled_reason: Option<String>,
}

fn link_view(token: &str, record: &UsageRecord, now: u64) -> AccountLinkView {
    AccountLinkView {
        token: token.into(),
        account_id: record.account_id.clone(),
        group_id: record.group_id.clone(),
        label: record.label.clone(),
        limit_bytes: record.limit_bytes,
        used_bytes: record.used_bytes,
        requests: record.requests,
        enabled: record.enabled,
        expires_at: record.expires_at,
        created_at: record.created_at,
        quota_period: record.quota_period.as_str(),
        next_reset_at: next_reset_at(record, now),
        other_uid: record.other_uid.clone(),
        disabled_reason: record.disabled_reason.clone(),
    }
}

async fn admin_account_links(
    State(state): State<AppState>,
    Path(account_id): Path<String>,
    headers: HeaderMap,
) -> Response {
    if !admin_authorized(&state, &headers).await {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    if !state
        .accounts
        .read()
        .await
        .accounts
        .contains_key(&account_id)
    {
        return StatusCode::NOT_FOUND.into_response();
    }
    let now = unix_now();
    let usage = state.usage.read().await;
    let links = usage
        .tokens
        .iter()
        .filter(|(_, record)| record.account_id == account_id)
        .map(|(token, record)| link_view(token, record, now))
        .collect::<Vec<_>>();
    Json(json!({"links": links})).into_response()
}

#[derive(Debug, Deserialize)]
struct CreateAccountLink {
    token: String,
    password: String,
    limit_bytes: u64,
    #[serde(default)]
    expires_at: Option<u64>,
    #[serde(default)]
    duration: Option<AccessDuration>,
    #[serde(default, alias = "period")]
    quota_period: QuotaPeriod,
    #[serde(default)]
    group_id: Option<String>,
    #[serde(default, alias = "otherUid")]
    other_uid: Option<String>,
    #[serde(default)]
    label: Option<String>,
}

fn hash_subscription_password(password: &str) -> Result<(String, String)> {
    let salt = SaltString::generate(&mut OsRng).to_string();
    let hash = Argon2::default()
        .hash_password(
            password.as_bytes(),
            &SaltString::from_b64(&salt).expect("generated salt is valid"),
        )
        .map_err(|error| anyhow::anyhow!("password hashing failed: {error:?}"))?
        .to_string();
    Ok((salt, hash))
}

async fn admin_create_account_link(
    State(state): State<AppState>,
    Path(account_id): Path<String>,
    headers: HeaderMap,
    Json(input): Json<CreateAccountLink>,
) -> Response {
    if !admin_authorized(&state, &headers).await {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    if input.token.trim().is_empty() || input.password.is_empty() || input.limit_bytes == 0 {
        return (
            StatusCode::BAD_REQUEST,
            "token/password/limit_bytes are required",
        )
            .into_response();
    }
    if !state
        .accounts
        .read()
        .await
        .accounts
        .get(&account_id)
        .is_some_and(|account| account.enabled && account.deleted_at.is_none())
    {
        return (StatusCode::BAD_REQUEST, "account is not available").into_response();
    }
    if state
        .usage
        .read()
        .await
        .tokens
        .contains_key(input.token.trim())
    {
        return (StatusCode::CONFLICT, "token already exists").into_response();
    }
    let group_id = input.group_id.clone().unwrap_or_else(default_all_group_id);
    if !state.groups.read().await.groups.contains_key(&group_id) {
        return (StatusCode::BAD_REQUEST, "group does not exist").into_response();
    }
    let (salt, hash) = match hash_subscription_password(&input.password) {
        Ok(value) => value,
        Err(error) => {
            return (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()).into_response()
        }
    };
    let now = unix_now();
    let mut record = UsageRecord {
        password_salt: salt,
        password_hash: hash,
        limit_bytes: input.limit_bytes,
        used_bytes: 0,
        requests: 0,
        enabled: true,
        expires_at: input.expires_at.or_else(|| {
            input
                .duration
                .map(|duration| expiry_for_duration(now, duration))
        }),
        last_seen: None,
        created_at: now,
        quota_period: input.quota_period,
        period_key: None,
        other_uid: normalize_other_uid(input.other_uid),
        disabled_reason: None,
        retired_at: None,
        account_id: account_id.clone(),
        group_id,
        label: normalize_label(input.label),
    };
    record.period_key = quota_period_key(&record, now);
    let view = link_view(input.token.trim(), &record, now);
    state
        .usage
        .write()
        .await
        .tokens
        .insert(input.token.trim().into(), record);
    if let Err(error) = save_usage(&state).await {
        return (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()).into_response();
    }
    (
        StatusCode::CREATED,
        Json(json!({"link": view, "password": input.password})),
    )
        .into_response()
}

#[derive(Debug, Deserialize)]
struct UpdateAccountLink {
    #[serde(default)]
    group_id: Option<String>,
    #[serde(default, alias = "profileName")]
    label: Option<String>,
    #[serde(default)]
    limit_bytes: Option<u64>,
    #[serde(default)]
    expires_at: Option<u64>,
    #[serde(default, alias = "period")]
    quota_period: Option<QuotaPeriod>,
    #[serde(default, alias = "otherUid")]
    other_uid: Option<String>,
    #[serde(default)]
    enabled: Option<bool>,
    #[serde(default)]
    reset_usage: bool,
}

async fn admin_update_account_link(
    State(state): State<AppState>,
    Path((account_id, token)): Path<(String, String)>,
    headers: HeaderMap,
    Json(input): Json<UpdateAccountLink>,
) -> Response {
    if !admin_authorized(&state, &headers).await {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    if let Some(group_id) = input.group_id.as_ref() {
        if !state.groups.read().await.groups.contains_key(group_id) {
            return (StatusCode::BAD_REQUEST, "group does not exist").into_response();
        }
    }
    let now = unix_now();
    let mut usage = state.usage.write().await;
    let Some(record) = usage.tokens.get_mut(&token) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    if record.account_id != account_id {
        return StatusCode::NOT_FOUND.into_response();
    }
    let previous_period = record.quota_period;
    if let Some(group_id) = input.group_id {
        record.group_id = group_id;
    }
    if let Some(label) = input.label {
        record.label = normalize_label(Some(label));
    }
    if let Some(limit_bytes) = input.limit_bytes {
        if limit_bytes == 0 {
            return (
                StatusCode::BAD_REQUEST,
                "limit_bytes must be greater than zero",
            )
                .into_response();
        }
        record.limit_bytes = limit_bytes;
    }
    if input.expires_at.is_some() {
        record.expires_at = input.expires_at;
    }
    if let Some(period) = input.quota_period {
        record.quota_period = period;
    }
    if input.other_uid.is_some() {
        record.other_uid = normalize_other_uid(input.other_uid);
    }
    if let Some(enabled) = input.enabled {
        record.enabled = enabled;
        record.disabled_reason = (!enabled).then(|| "admin_disabled".into());
        record.retired_at = (!enabled).then_some(now);
    }
    if input.reset_usage || previous_period != record.quota_period {
        record.used_bytes = 0;
        record.requests = 0;
    }
    record.period_key = quota_period_key(record, now);
    let view = link_view(&token, record, now);
    drop(usage);
    if let Err(error) = save_usage(&state).await {
        return (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()).into_response();
    }
    Json(json!({"link": view})).into_response()
}

async fn admin_delete_account_link(
    State(state): State<AppState>,
    Path((account_id, token)): Path<(String, String)>,
    headers: HeaderMap,
) -> Response {
    if !admin_authorized(&state, &headers).await {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let mut usage = state.usage.write().await;
    let Some(record) = usage.tokens.get_mut(&token) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    if record.account_id != account_id {
        return StatusCode::NOT_FOUND.into_response();
    }
    record.enabled = false;
    record.disabled_reason = Some("admin_disabled".into());
    record.retired_at = Some(unix_now());
    drop(usage);
    if let Err(error) = save_usage(&state).await {
        return (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()).into_response();
    }
    StatusCode::NO_CONTENT.into_response()
}

async fn admin_rotate_account_link(
    State(state): State<AppState>,
    Path((account_id, token)): Path<(String, String)>,
    headers: HeaderMap,
) -> Response {
    if !admin_authorized(&state, &headers).await {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let password = Uuid::new_v4().simple().to_string();
    let (salt, hash) = match hash_subscription_password(&password) {
        Ok(value) => value,
        Err(error) => {
            return (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()).into_response()
        }
    };
    let now = unix_now();
    let mut usage = state.usage.write().await;
    let Some(record) = usage.tokens.get_mut(&token) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    if record.account_id != account_id {
        return StatusCode::NOT_FOUND.into_response();
    }
    record.password_salt = salt;
    record.password_hash = hash;
    record.enabled = true;
    record.disabled_reason = None;
    record.retired_at = None;
    let view = link_view(&token, record, now);
    drop(usage);
    if let Err(error) = save_usage(&state).await {
        return (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()).into_response();
    }
    Json(json!({"link": view, "password": password})).into_response()
}

#[derive(Debug, Serialize)]
struct GroupView {
    group_id: String,
    name: String,
    node_links: Vec<String>,
    node_count: usize,
    protected: bool,
    created_at: u64,
}

#[derive(Debug, Deserialize)]
struct CreateGroup {
    #[serde(default)]
    group_id: Option<String>,
    name: String,
    #[serde(default)]
    node_links: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct UpdateGroup {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    node_links: Option<Vec<String>>,
}

async fn admin_groups(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if !admin_authorized(&state, &headers).await {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let group_records = state
        .groups
        .read()
        .await
        .groups
        .values()
        .cloned()
        .collect::<Vec<_>>();
    let mut views = Vec::with_capacity(group_records.len());
    for group in group_records {
        let node_count = if group.group_id == "all" {
            sources(&state, None).await.len()
        } else {
            group.node_links.len()
        };
        views.push(GroupView {
            group_id: group.group_id.clone(),
            name: group.name.clone(),
            node_links: group.node_links.clone(),
            node_count,
            protected: group.protected,
            created_at: group.created_at,
        });
    }
    Json(json!({"groups": views})).into_response()
}

async fn admin_create_group(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<CreateGroup>,
) -> Response {
    if !admin_authorized(&state, &headers).await {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    if input.name.trim().is_empty() {
        return (StatusCode::BAD_REQUEST, "name is required").into_response();
    }
    let group_id = normalize_identifier(
        input.group_id.as_deref().unwrap_or(&input.name),
        &format!("group-{}", Uuid::new_v4()),
    );
    if group_id == "all" {
        return (StatusCode::BAD_REQUEST, "all is reserved").into_response();
    }
    let mut groups = state.groups.write().await;
    if groups.groups.contains_key(&group_id) {
        return (StatusCode::CONFLICT, "group already exists").into_response();
    }
    let record = SubscriptionGroup {
        group_id: group_id.clone(),
        name: input.name.trim().chars().take(128).collect(),
        node_links: normalize_links(input.node_links),
        protected: false,
        created_at: unix_now(),
    };
    groups.groups.insert(group_id, record.clone());
    drop(groups);
    if let Err(error) = save_groups(&state).await {
        return (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()).into_response();
    }
    (StatusCode::CREATED, Json(json!({"group": record}))).into_response()
}

async fn admin_update_group(
    State(state): State<AppState>,
    Path(group_id): Path<String>,
    headers: HeaderMap,
    Json(input): Json<UpdateGroup>,
) -> Response {
    if !admin_authorized(&state, &headers).await {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let mut groups = state.groups.write().await;
    let Some(record) = groups.groups.get_mut(&group_id) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    if record.protected {
        if input
            .name
            .as_deref()
            .is_some_and(|name| name.trim() != "all")
            || input
                .node_links
                .as_ref()
                .is_some_and(|links| !links.is_empty())
        {
            return (
                StatusCode::BAD_REQUEST,
                "all group is managed automatically",
            )
                .into_response();
        }
    } else {
        if let Some(name) = input.name {
            if name.trim().is_empty() {
                return (StatusCode::BAD_REQUEST, "name is required").into_response();
            }
            record.name = name.trim().chars().take(128).collect();
        }
        if let Some(node_links) = input.node_links {
            record.node_links = normalize_links(node_links);
        }
    }
    let response = record.clone();
    drop(groups);
    if let Err(error) = save_groups(&state).await {
        return (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()).into_response();
    }
    Json(json!({"group": response})).into_response()
}

async fn admin_delete_group(
    State(state): State<AppState>,
    Path(group_id): Path<String>,
    headers: HeaderMap,
) -> Response {
    if !admin_authorized(&state, &headers).await {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    if group_id == "all" {
        return (StatusCode::BAD_REQUEST, "all group cannot be deleted").into_response();
    }
    let mut groups = state.groups.write().await;
    if groups.groups.remove(&group_id).is_none() {
        return StatusCode::NOT_FOUND.into_response();
    }
    drop(groups);
    {
        let mut usage = state.usage.write().await;
        for record in usage
            .tokens
            .values_mut()
            .filter(|record| record.group_id == group_id)
        {
            record.group_id = default_all_group_id();
        }
    }
    if let Err(error) = save_groups(&state).await {
        return (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()).into_response();
    }
    if let Err(error) = save_usage(&state).await {
        return (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()).into_response();
    }
    StatusCode::NO_CONTENT.into_response()
}

#[derive(Debug, Serialize)]
struct TokenView {
    token: String,
    limit_bytes: u64,
    used_bytes: u64,
    requests: u64,
    enabled: bool,
    expires_at: Option<u64>,
    last_seen: Option<u64>,
    created_at: u64,
    quota_period: &'static str,
    next_reset_at: Option<u64>,
    other_uid: Option<String>,
    #[serde(rename = "otherUid")]
    other_uid_compat: Option<String>,
    disabled_reason: Option<String>,
    retired_at: Option<u64>,
    account_id: String,
    group_id: String,
    label: String,
}

impl TokenView {
    fn from_record(record: &UsageRecord, now: u64) -> Self {
        Self {
            token: String::new(),
            limit_bytes: record.limit_bytes,
            used_bytes: record.used_bytes,
            requests: record.requests,
            enabled: record.enabled,
            expires_at: record.expires_at,
            last_seen: record.last_seen,
            created_at: record.created_at,
            quota_period: record.quota_period.as_str(),
            next_reset_at: next_reset_at(record, now),
            other_uid: record.other_uid.clone(),
            other_uid_compat: record.other_uid.clone(),
            disabled_reason: record.disabled_reason.clone(),
            retired_at: record.retired_at,
            account_id: record.account_id.clone(),
            group_id: record.group_id.clone(),
            label: record.label.clone(),
        }
    }

    fn from_pair(token: &str, record: &UsageRecord, now: u64) -> Self {
        Self {
            token: token.to_owned(),
            ..Self::from_record(record, now)
        }
    }
}

#[derive(Debug, Deserialize)]
struct CreateToken {
    token: String,
    password: String,
    limit_bytes: u64,
    expires_at: Option<u64>,
    #[serde(default)]
    duration: Option<AccessDuration>,
    #[serde(default, alias = "period")]
    quota_period: QuotaPeriod,
    #[serde(default, alias = "otherUid")]
    other_uid: Option<String>,
    #[serde(default)]
    account_id: Option<String>,
    #[serde(default)]
    group_id: Option<String>,
    #[serde(default, alias = "profileName")]
    label: Option<String>,
}

async fn admin_create_token(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<CreateToken>,
) -> Response {
    if !admin_authorized(&state, &headers).await {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    if input.token.trim().is_empty() || input.password.is_empty() || input.limit_bytes == 0 {
        return (
            StatusCode::BAD_REQUEST,
            "token/password/limit_bytes are required",
        )
            .into_response();
    }
    let token_name = input.token.trim();
    if state.usage.read().await.tokens.contains_key(token_name) {
        return (StatusCode::CONFLICT, "token already exists").into_response();
    }
    let account_id = input
        .account_id
        .clone()
        .unwrap_or_else(default_legacy_account_id);
    let group_id = input.group_id.clone().unwrap_or_else(default_all_group_id);
    if !state
        .accounts
        .read()
        .await
        .accounts
        .get(&account_id)
        .is_some_and(|account| account.enabled && account.deleted_at.is_none())
    {
        return (StatusCode::BAD_REQUEST, "account is not available").into_response();
    }
    if !state.groups.read().await.groups.contains_key(&group_id) {
        return (StatusCode::BAD_REQUEST, "group does not exist").into_response();
    }
    let salt = SaltString::generate(&mut OsRng).to_string();
    let hash = match Argon2::default().hash_password(
        input.password.as_bytes(),
        &SaltString::from_b64(&salt).expect("generated salt is valid"),
    ) {
        Ok(value) => value.to_string(),
        Err(_) => {
            return (StatusCode::INTERNAL_SERVER_ERROR, "password hashing failed").into_response()
        }
    };
    let now = unix_now();
    let mut record = UsageRecord {
        password_salt: salt,
        password_hash: hash,
        limit_bytes: input.limit_bytes,
        used_bytes: 0,
        requests: 0,
        enabled: true,
        expires_at: input.expires_at.or_else(|| {
            input
                .duration
                .map(|duration| expiry_for_duration(now, duration))
        }),
        last_seen: None,
        created_at: now,
        quota_period: input.quota_period,
        period_key: None,
        other_uid: normalize_other_uid(input.other_uid),
        disabled_reason: None,
        retired_at: None,
        account_id,
        group_id,
        label: normalize_label(input.label),
    };
    record.period_key = quota_period_key(&record, now);
    state
        .usage
        .write()
        .await
        .tokens
        .insert(token_name.to_owned(), record);
    if let Err(error) = save_usage(&state).await {
        return (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()).into_response();
    }
    StatusCode::CREATED.into_response()
}

async fn admin_disable_token(
    State(state): State<AppState>,
    Path(token): Path<String>,
    headers: HeaderMap,
) -> Response {
    if !admin_authorized(&state, &headers).await {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    if let Some(record) = state.usage.write().await.tokens.get_mut(&token) {
        record.enabled = false;
        record.disabled_reason = Some("admin_disabled".into());
        record.retired_at = Some(unix_now());
    } else {
        return StatusCode::NOT_FOUND.into_response();
    }
    if let Err(error) = save_usage(&state).await {
        return (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()).into_response();
    }
    StatusCode::NO_CONTENT.into_response()
}

async fn admin_rotate_token(
    State(state): State<AppState>,
    Path(token): Path<String>,
    headers: HeaderMap,
) -> Response {
    if !admin_authorized(&state, &headers).await {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let password = Uuid::new_v4().simple().to_string();
    let (salt, hash) = match hash_subscription_password(&password) {
        Ok(value) => value,
        Err(error) => {
            return (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()).into_response()
        }
    };
    let mut usage = state.usage.write().await;
    let Some(record) = usage.tokens.get_mut(&token) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    record.password_salt = salt;
    record.password_hash = hash;
    record.enabled = true;
    record.disabled_reason = None;
    record.retired_at = None;
    drop(usage);
    if let Err(error) = save_usage(&state).await {
        return (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()).into_response();
    }
    Json(json!({"token": token, "password": password})).into_response()
}

async fn admin_subscription_link(
    State(state): State<AppState>,
    Path(token): Path<String>,
    headers: HeaderMap,
) -> Response {
    if !admin_authorized(&state, &headers).await {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let created_at = {
        let usage = state.usage.read().await;
        let Some(record) = usage.tokens.get(&token) else {
            return StatusCode::NOT_FOUND.into_response();
        };
        if !record.enabled
            || record
                .expires_at
                .is_some_and(|expires| expires <= unix_now())
        {
            return (StatusCode::CONFLICT, "token is not active").into_response();
        }
        record.created_at
    };
    let config = state.config.read().await;
    match issue_subscription_jwt(&config.server, &token, created_at) {
        Ok(access_token) => Json(json!({
            "token": token,
            "access_token": access_token,
            "filename": "vpn233",
        }))
        .into_response(),
        Err(error) => {
            tracing::error!(%error, "subscription JWT issuance failed");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

#[derive(Debug, Deserialize)]
struct RenewToken {
    #[serde(default)]
    expires_at: Option<u64>,
    #[serde(default)]
    extend_seconds: Option<u64>,
    #[serde(default)]
    duration: Option<AccessDuration>,
    #[serde(default, alias = "period")]
    quota_period: Option<QuotaPeriod>,
    #[serde(default)]
    limit_bytes: Option<u64>,
    #[serde(default, alias = "otherUid")]
    other_uid: Option<String>,
    #[serde(default)]
    reset_usage: bool,
}

fn normalize_other_uid(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().chars().take(256).collect::<String>())
        .filter(|value| !value.is_empty())
}

async fn admin_renew_token(
    State(state): State<AppState>,
    Path(token): Path<String>,
    headers: HeaderMap,
    Json(input): Json<RenewToken>,
) -> Response {
    if !admin_authorized(&state, &headers).await {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    if input.expires_at.is_none() && input.extend_seconds.is_none() && input.duration.is_none() {
        return (
            StatusCode::BAD_REQUEST,
            "expires_at or extend_seconds is required",
        )
            .into_response();
    }
    let now = unix_now();
    let mut usage = state.usage.write().await;
    let Some(record) = usage.tokens.get_mut(&token) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let previous_period = record.quota_period;
    if let Some(expires_at) = input.expires_at {
        record.expires_at = Some(expires_at);
    } else if let Some(extend_seconds) = input.extend_seconds {
        let base = record.expires_at.unwrap_or(now).max(now);
        record.expires_at = Some(base.saturating_add(extend_seconds));
    } else if let Some(duration) = input.duration {
        let base = record.expires_at.unwrap_or(now).max(now);
        record.expires_at = Some(expiry_for_duration(base, duration));
    }
    if let Some(period) = input.quota_period {
        record.quota_period = period;
    }
    if let Some(limit_bytes) = input.limit_bytes {
        if limit_bytes == 0 {
            return (
                StatusCode::BAD_REQUEST,
                "limit_bytes must be greater than zero",
            )
                .into_response();
        }
        record.limit_bytes = limit_bytes;
    }
    if input.other_uid.is_some() {
        record.other_uid = normalize_other_uid(input.other_uid);
    }
    record.enabled = true;
    record.disabled_reason = None;
    record.retired_at = None;
    if input.reset_usage || previous_period != record.quota_period {
        record.used_bytes = 0;
        record.requests = 0;
    }
    record.period_key = quota_period_key(record, now);
    drop(usage);
    if let Err(error) = save_usage(&state).await {
        return (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()).into_response();
    }
    Json(json!({"ok": true, "token": token})).into_response()
}

async fn admin_reset_token(
    State(state): State<AppState>,
    Path(token): Path<String>,
    headers: HeaderMap,
) -> Response {
    if !admin_authorized(&state, &headers).await {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let now = unix_now();
    let mut usage = state.usage.write().await;
    let Some(record) = usage.tokens.get_mut(&token) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    record.used_bytes = 0;
    record.requests = 0;
    record.period_key = quota_period_key(record, now);
    drop(usage);
    if let Err(error) = save_usage(&state).await {
        return (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()).into_response();
    }
    Json(json!({"ok": true, "token": token})).into_response()
}

fn parse_link(raw: &str, index: usize) -> Result<ProxyNode> {
    let parsed = Url::parse(raw).context("invalid node URL")?;
    let protocol = parsed.scheme().to_lowercase();
    let name = parsed
        .fragment()
        .map(|v| v.to_string())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| format!("vpn233-{index}"));
    let host = parsed.host_str().context("node has no host")?;
    let port = parsed.port().unwrap_or(443);
    let node = match protocol.as_str() {
        "vless" => {
            let mut n = proxy_node(&name, "vless", host, port);
            let params = parsed.query_pairs().collect::<HashMap<_, _>>();
            n.insert("uuid".into(), string(parsed.username()));
            let security = params.get("security").map(|v| v.as_ref()).unwrap_or("");
            n.insert("tls".into(), boolean(matches!(security, "tls" | "reality")));
            if let Some(network) = params.get("type") {
                n.insert("network".into(), string(network.as_ref()));
            }
            if let Some(flow) = params.get("flow") {
                n.insert("flow".into(), string(flow.as_ref()));
            }
            if let Some(sni) = params.get("sni").or_else(|| params.get("serverName")) {
                n.insert("servername".into(), string(sni.as_ref()));
            }
            if let Some(fp) = params.get("fp") {
                n.insert("client-fingerprint".into(), string(fp.as_ref()));
            }
            if let Some(path) = params.get("path") {
                n.insert(
                    "ws-opts".into(),
                    serde_yaml::to_value(serde_json::json!({"path": path.as_ref()}))?,
                );
            }
            if security == "reality" {
                let mut reality = serde_json::Map::new();
                if let Some(public_key) = params.get("pbk") {
                    reality.insert(
                        "public-key".into(),
                        serde_json::Value::String(public_key.to_string()),
                    );
                }
                if let Some(short_id) = params.get("sid") {
                    reality.insert(
                        "short-id".into(),
                        serde_json::Value::String(short_id.to_string()),
                    );
                }
                n.insert("reality-opts".into(), serde_yaml::to_value(reality)?);
            }
            n
        }
        "trojan" => {
            let mut n = proxy_node(&name, "trojan", host, port);
            n.insert("password".into(), string(parsed.username()));
            n
        }
        "ss" => {
            let mut n = proxy_node(&name, "ss", host, port);
            let user = BASE64
                .decode(parsed.username())
                .ok()
                .and_then(|b| String::from_utf8(b).ok())
                .unwrap_or_else(|| parsed.username().into());
            if let Some((cipher, password)) = user.split_once(':') {
                n.insert("cipher".into(), string(cipher));
                n.insert("password".into(), string(password));
            }
            n
        }
        "vmess" => {
            let value = parsed.path().trim_start_matches('/');
            let decoded = BASE64.decode(value).context("invalid vmess base64")?;
            let data: serde_json::Value =
                serde_json::from_slice(&decoded).context("invalid vmess JSON")?;
            let mut n = proxy_node(
                data.get("ps").and_then(|v| v.as_str()).unwrap_or(&name),
                "vmess",
                data.get("add").and_then(|v| v.as_str()).unwrap_or(host),
                data.get("port")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(port as u64) as u16,
            );
            if let Some(id) = data.get("id").and_then(|v| v.as_str()) {
                n.insert("uuid".into(), string(id));
            }
            n.insert(
                "cipher".into(),
                string(data.get("scy").and_then(|v| v.as_str()).unwrap_or("auto")),
            );
            n
        }
        _ => anyhow::bail!("unsupported node protocol: {protocol}"),
    };
    Ok(node)
}

fn verify_password(password: &str, _salt: &str, expected: &str) -> bool {
    PasswordHash::new(expected).ok().is_some_and(|hash| {
        Argon2::default()
            .verify_password(password.as_bytes(), &hash)
            .is_ok()
    })
}

#[cfg(test)]
mod tests {
    use super::{
        add_months, default_all_group_id, default_legacy_account_id, issue_subscription_jwt,
        month_key, parse_link, quota_period_key, refresh_quota, unix_now, verify_subscription_jwt,
        CreateToken, QuotaPeriod, ServerConfig, TokenView, UsageRecord,
    };

    fn test_record(created_at: u64, quota_period: QuotaPeriod) -> UsageRecord {
        UsageRecord {
            password_salt: String::new(),
            password_hash: String::new(),
            limit_bytes: 100,
            used_bytes: 80,
            requests: 3,
            enabled: true,
            expires_at: None,
            last_seen: None,
            created_at,
            quota_period,
            period_key: None,
            other_uid: None,
            disabled_reason: None,
            retired_at: None,
            account_id: default_legacy_account_id(),
            group_id: default_all_group_id(),
            label: String::new(),
        }
    }

    fn test_server_config() -> ServerConfig {
        ServerConfig {
            host: "127.0.0.1".into(),
            http_port: 23444,
            default_format: "clash".into(),
            data_dir: "data".into(),
            admin_user: "root".into(),
            admin_password: "secret".into(),
            admin_origins: String::new(),
            web_dir: "web".into(),
            provider_registry_token: String::new(),
            provider_stale_after_secs: 120,
        }
    }

    #[test]
    fn quota_periods_refresh_at_expected_boundaries() {
        let created = 1_704_067_200; // 2024-01-01 00:00:00 UTC
        assert_eq!(month_key(created), "2024-01");
        assert_eq!(month_key(add_months(created, 1)), "2024-02");

        let mut calendar = test_record(created, QuotaPeriod::CalendarMonth);
        calendar.period_key = quota_period_key(&calendar, created);
        assert!(!refresh_quota(&mut calendar, created + 20 * 86_400));
        assert!(refresh_quota(&mut calendar, add_months(created, 1)));
        assert_eq!(calendar.used_bytes, 0);
        assert_eq!(calendar.requests, 0);

        let mut creation = test_record(
            1_704_067_200 + 15 * 86_400,
            QuotaPeriod::MonthlyFromCreation,
        );
        let creation_time = creation.created_at;
        creation.period_key = quota_period_key(&creation, creation_time);
        assert!(!refresh_quota(&mut creation, creation_time + 20 * 86_400));
        assert!(refresh_quota(&mut creation, add_months(creation_time, 1)));
    }

    #[test]
    fn legacy_records_default_to_lifetime_quota() {
        let record = test_record(unix_now(), QuotaPeriod::default());
        assert_eq!(record.quota_period, QuotaPeriod::Lifetime);
        assert_eq!(quota_period_key(&record, unix_now()), None);
    }

    #[test]
    fn external_uid_accepts_camel_case_and_is_returned_both_ways() {
        let create: CreateToken = serde_json::from_value(serde_json::json!({
            "token": "demo",
            "password": "secret",
            "limit_bytes": 1,
            "otherUid": "order-123"
        }))
        .expect("camel-case external uid should deserialize");
        assert_eq!(create.other_uid.as_deref(), Some("order-123"));

        let record = test_record(unix_now(), QuotaPeriod::Lifetime);
        let view = TokenView::from_pair("demo", &record, unix_now());
        let json = serde_json::to_value(view).expect("token view should serialize");
        assert_eq!(json["other_uid"], json["otherUid"]);
    }

    #[test]
    fn subscription_jwt_is_stable_and_scoped_to_one_token() {
        let config = test_server_config();
        let first = issue_subscription_jwt(&config, "vpn233", 1_700_000_000).unwrap();
        let second = issue_subscription_jwt(&config, "vpn233", 1_700_000_000).unwrap();
        assert_eq!(first, second);
        assert!(verify_subscription_jwt(&config, "vpn233", &first));
        assert!(!verify_subscription_jwt(&config, "other-token", &first));
        let mut changed = config.clone();
        changed.admin_password = "changed".into();
        assert!(!verify_subscription_jwt(&changed, "vpn233", &first));
    }

    #[test]
    fn parses_vless_reality_parameters_for_mihomo() {
        let node = parse_link(
            "vless://11111111-1111-1111-1111-111111111111@example.invalid:443?security=reality&type=tcp&pbk=public-key&sid=short-id&sni=addons.mozilla.org&fp=chrome&flow=xtls-rprx-vision#edge",
            0,
        )
        .expect("valid vless reality link");
        assert_eq!(
            node.get("type").and_then(|value| value.as_str()),
            Some("vless")
        );
        assert_eq!(
            node.get("tls").and_then(|value| value.as_bool()),
            Some(true)
        );
        assert_eq!(
            node.get("servername").and_then(|value| value.as_str()),
            Some("addons.mozilla.org")
        );
        assert_eq!(
            node.get("client-fingerprint")
                .and_then(|value| value.as_str()),
            Some("chrome")
        );
        assert_eq!(
            node.get("flow").and_then(|value| value.as_str()),
            Some("xtls-rprx-vision")
        );
        let reality = node.get("reality-opts").expect("reality options");
        assert_eq!(
            reality.get("public-key").and_then(|value| value.as_str()),
            Some("public-key")
        );
        assert_eq!(
            reality.get("short-id").and_then(|value| value.as_str()),
            Some("short-id")
        );
    }
}
