//! Administrator APIs plus the authenticated `/api/status`, `/api/nodes`, and
//! `/api/refresh` endpoints.

use std::collections::BTreeMap;

use axum::{
    extract::{Path, Query, State},
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use uuid::Uuid;

use crate::auth::{
    admin_authorized, hash_subscription_password, issue_admin_jwt, issue_subscription_jwt,
    verify_admin_credentials, ClientIp,
};
use crate::models::{
    default_all_group_id, default_legacy_account_id, AccountRecord, NodeInfo, QuotaPeriod,
    SubscriptionGroup, UsageRecord,
};
use crate::quota::{
    expire_tokens, expiry_for_duration, next_reset_at, quota_period_key, refresh_quota,
};
use crate::registry::normalize_links;
use crate::state::{save_accounts, save_groups, save_providers, save_usage, AppState};
use crate::subscription::{parse_source_payloads, sanitize_subscription_filename, sources};
use crate::util::unix_now;

#[derive(Debug, Deserialize)]
pub(crate) struct AdminLogin {
    username: String,
    password: String,
}

pub(crate) async fn admin_login(
    State(state): State<AppState>,
    client: ClientIp,
    Json(input): Json<AdminLogin>,
) -> Response {
    let now = unix_now();
    if let Some(retry_after) = state.login_limiter.retry_after(client.0, now) {
        let mut response =
            (StatusCode::TOO_MANY_REQUESTS, "too many login attempts").into_response();
        if let Ok(value) = HeaderValue::from_str(&retry_after.to_string()) {
            response.headers_mut().insert(header::RETRY_AFTER, value);
        }
        return response;
    }
    let config = state.config.read().await;
    if !verify_admin_credentials(&config.server, &input.username, &input.password) {
        drop(config);
        state.login_limiter.record_failure(client.0, now);
        return StatusCode::UNAUTHORIZED.into_response();
    }
    state.login_limiter.record_success(client.0);
    let now = unix_now();
    match issue_admin_jwt(&config.server, now) {
        Ok(token) => Json(json!({
            "token": token,
            "user": config.server.admin_user,
            "expires_at": now.saturating_add(crate::auth::ADMIN_JWT_TTL_SECS),
        }))
        .into_response(),
        Err(error) => {
            tracing::error!(%error, "admin JWT issuance failed");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

pub(crate) async fn status(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(_query): Query<std::collections::HashMap<String, String>>,
) -> Response {
    if !admin_authorized(&state, &headers).await {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    Json(json!({"ok": true, "service": "vpn233-subscribe-server", "version": env!("CARGO_PKG_VERSION")})).into_response()
}

pub(crate) async fn nodes(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<std::collections::HashMap<String, String>>,
) -> Response {
    if !admin_authorized(&state, &headers).await {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let links = sources(&state, query.get("group").map(String::as_str)).await;
    let (parsed, _errors) = parse_source_payloads(&links);
    let result: Vec<NodeInfo> = parsed
        .iter()
        .filter_map(|node| {
            let peers = node.get("peers").and_then(serde_yaml::Value::as_sequence);
            let first_peer = peers.and_then(|peers| peers.first());
            let server = node
                .get("server")
                .and_then(serde_yaml::Value::as_str)
                .or_else(|| first_peer?.get("server")?.as_str())?;
            let range = node.get("port-range").and_then(serde_yaml::Value::as_str);
            let port = vpn233_core_lib_rust::nodes::node_u16(node, "port")
                .or_else(|| u16::try_from(first_peer?.get("port")?.as_u64()?).ok())
                .or_else(|| range?.split_once('-')?.0.parse::<u16>().ok())?;
            let endpoint = if let Some(peers) = peers {
                peers
                    .iter()
                    .filter_map(|peer| {
                        Some(format_endpoint(
                            peer.get("server")?.as_str()?,
                            &peer.get("port")?.as_u64()?.to_string(),
                        ))
                    })
                    .collect::<Vec<_>>()
                    .join(", ")
            } else {
                format_endpoint(
                    server,
                    &range.map(str::to_owned).unwrap_or_else(|| port.to_string()),
                )
            };
            let payload =
                serde_json::to_string(&[vpn233_core_lib_rust::nodes::strip_internal_keys(node)])
                    .ok()?;
            Some(NodeInfo {
                name: node.get("name")?.as_str()?.into(),
                protocol: node.get("type")?.as_str()?.into(),
                server: server.into(),
                port,
                endpoint,
                payload,
                link: node
                    .get(vpn233_core_lib_rust::link::LINK_KEY)
                    .and_then(serde_yaml::Value::as_str)
                    .map(str::to_owned)
                    .or_else(|| vpn233_core_lib_rust::render_link(node).ok())
                    .unwrap_or_default(),
            })
        })
        .collect();
    Json(result).into_response()
}

fn format_endpoint(server: &str, port: &str) -> String {
    if server.contains(':') {
        format!("[{server}]:{port}")
    } else {
        format!("{server}:{port}")
    }
}

fn payload_node_count(payloads: &[String]) -> usize {
    let sources = payloads
        .iter()
        .map(|payload| (String::new(), payload.clone()))
        .collect::<Vec<_>>();
    parse_source_payloads(&sources).0.len()
}

pub(crate) async fn refresh(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(_query): Query<std::collections::HashMap<String, String>>,
) -> Response {
    if !admin_authorized(&state, &headers).await {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let (nodes, errors) = parse_source_payloads(&sources(&state, None).await);
    Json(json!({"ok": true, "nodes": nodes.len(), "skipped": errors.len()})).into_response()
}

#[derive(Debug, Serialize)]
pub(crate) struct ProviderView {
    provider_id: String,
    name: String,
    version: String,
    addresses: Vec<crate::models::ProviderAddress>,
    node_count: usize,
    enabled: bool,
    online: bool,
    registered_at: u64,
    last_seen: u64,
    heartbeat_interval_secs: u64,
}

pub(crate) async fn admin_providers(State(state): State<AppState>, headers: HeaderMap) -> Response {
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
            node_count: payload_node_count(&provider.node_links),
            enabled: provider.enabled,
            online: provider.enabled && provider.last_seen.saturating_add(stale_after) > now,
            registered_at: provider.registered_at,
            last_seen: provider.last_seen,
            heartbeat_interval_secs: provider.heartbeat_interval_secs,
        })
        .collect::<Vec<_>>();
    Json(json!({"providers": views, "stale_after_secs": stale_after})).into_response()
}

pub(crate) async fn admin_disable_provider(
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

#[derive(Debug, Serialize)]
pub(crate) struct TokenView {
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

pub(crate) async fn admin_tokens(State(state): State<AppState>, headers: HeaderMap) -> Response {
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
            tracing::warn!(%error, "usage persistence failed");
        }
    }
    Json(json!({"tokens": views})).into_response()
}

#[derive(Debug, Deserialize)]
pub(crate) struct CreateToken {
    token: String,
    password: String,
    limit_bytes: u64,
    #[serde(default)]
    expires_at: Option<u64>,
    #[serde(default)]
    duration: Option<crate::models::AccessDuration>,
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

pub(crate) async fn admin_create_token(
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
        account_id,
        group_id,
        label: normalize_label(input.label),
        daily: BTreeMap::new(),
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

pub(crate) async fn admin_disable_token(
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

pub(crate) async fn admin_delete_token(
    State(state): State<AppState>,
    Path(token): Path<String>,
    headers: HeaderMap,
) -> Response {
    if !admin_authorized(&state, &headers).await {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let mut usage = state.usage.write().await;
    if usage.tokens.remove(&token).is_none() {
        return StatusCode::NOT_FOUND.into_response();
    }
    drop(usage);
    if let Err(error) = save_usage(&state).await {
        return (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()).into_response();
    }
    StatusCode::NO_CONTENT.into_response()
}

pub(crate) async fn admin_rotate_token(
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

pub(crate) async fn admin_subscription_link(
    State(state): State<AppState>,
    Path(token): Path<String>,
    headers: HeaderMap,
) -> Response {
    if !admin_authorized(&state, &headers).await {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let (created_at, filename) = {
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
        (
            record.created_at,
            sanitize_subscription_filename(&record.label, "yaml"),
        )
    };
    let config = state.config.read().await;
    match issue_subscription_jwt(&config.server, &token, created_at) {
        Ok(access_token) => Json(json!({
            "token": token,
            "access_token": access_token,
            "filename": filename,
        }))
        .into_response(),
        Err(error) => {
            tracing::error!(%error, "subscription JWT issuance failed");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

#[derive(Debug, Deserialize)]
pub(crate) struct RenewToken {
    #[serde(default)]
    expires_at: Option<u64>,
    #[serde(default)]
    extend_seconds: Option<u64>,
    #[serde(default)]
    duration: Option<crate::models::AccessDuration>,
    #[serde(default, alias = "period")]
    quota_period: Option<QuotaPeriod>,
    #[serde(default)]
    limit_bytes: Option<u64>,
    #[serde(default, alias = "otherUid")]
    other_uid: Option<String>,
    #[serde(default)]
    reset_usage: bool,
}

pub(crate) fn normalize_other_uid(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().chars().take(256).collect::<String>())
        .filter(|value| !value.is_empty())
}

pub(crate) async fn admin_renew_token(
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

pub(crate) async fn admin_reset_token(
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

#[derive(Debug, Serialize)]
pub(crate) struct AccountView {
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
pub(crate) struct CreateAccount {
    #[serde(default)]
    account_id: Option<String>,
    username: String,
    #[serde(default)]
    display_name: Option<String>,
    #[serde(default, alias = "otherUid")]
    other_uid: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct UpdateAccount {
    #[serde(default)]
    display_name: Option<String>,
    #[serde(default)]
    enabled: Option<bool>,
    #[serde(default, alias = "otherUid")]
    other_uid: Option<String>,
}

pub(crate) fn normalize_identifier(value: &str, fallback: &str) -> String {
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

pub(crate) fn normalize_label(value: Option<String>) -> String {
    value.unwrap_or_default().trim().chars().take(128).collect()
}

pub(crate) async fn admin_accounts(State(state): State<AppState>, headers: HeaderMap) -> Response {
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

pub(crate) async fn admin_create_account(
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
        created_at: unix_now(),
        deleted_at: None,
    };
    accounts.accounts.insert(account_id.clone(), record.clone());
    drop(accounts);
    if let Err(error) = save_accounts(&state).await {
        return (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()).into_response();
    }
    (StatusCode::CREATED, Json(json!({"account": record}))).into_response()
}

pub(crate) async fn admin_update_account(
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

pub(crate) async fn admin_delete_account(
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
pub(crate) struct AccountLinkView {
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

pub(crate) fn link_view(token: &str, record: &UsageRecord, now: u64) -> AccountLinkView {
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

pub(crate) async fn admin_account_links(
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
pub(crate) struct CreateAccountLink {
    token: String,
    password: String,
    limit_bytes: u64,
    #[serde(default)]
    expires_at: Option<u64>,
    #[serde(default)]
    duration: Option<crate::models::AccessDuration>,
    #[serde(default, alias = "period")]
    quota_period: QuotaPeriod,
    #[serde(default)]
    group_id: Option<String>,
    #[serde(default, alias = "otherUid")]
    other_uid: Option<String>,
    #[serde(default)]
    label: Option<String>,
}

pub(crate) async fn admin_create_account_link(
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
        daily: BTreeMap::new(),
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
pub(crate) struct UpdateAccountLink {
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

pub(crate) async fn admin_update_account_link(
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

pub(crate) async fn admin_delete_account_link(
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

pub(crate) async fn admin_rotate_account_link(
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
pub(crate) struct GroupView {
    group_id: String,
    name: String,
    node_links: Vec<String>,
    node_count: usize,
    protected: bool,
    created_at: u64,
}

#[derive(Debug, Deserialize)]
pub(crate) struct CreateGroup {
    #[serde(default)]
    group_id: Option<String>,
    name: String,
    #[serde(default)]
    node_links: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct UpdateGroup {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    node_links: Option<Vec<String>>,
}

pub(crate) async fn admin_groups(State(state): State<AppState>, headers: HeaderMap) -> Response {
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
            parse_source_payloads(&sources(&state, None).await).0.len()
        } else {
            payload_node_count(&group.node_links)
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

pub(crate) async fn admin_create_group(
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
    let mut view = serde_json::to_value(&record).unwrap_or_default();
    view["node_count"] = json!(payload_node_count(&record.node_links));
    (StatusCode::CREATED, Json(json!({"group": view}))).into_response()
}

pub(crate) async fn admin_update_group(
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

pub(crate) async fn admin_delete_group(
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

#[cfg(test)]
mod tests {
    use super::*;

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

        let now = unix_now();
        let record = UsageRecord {
            password_salt: String::new(),
            password_hash: String::new(),
            limit_bytes: 100,
            used_bytes: 0,
            requests: 0,
            enabled: true,
            expires_at: None,
            last_seen: None,
            created_at: now,
            quota_period: QuotaPeriod::Lifetime,
            period_key: None,
            other_uid: Some("order-123".into()),
            disabled_reason: None,
            retired_at: None,
            account_id: default_legacy_account_id(),
            group_id: default_all_group_id(),
            label: String::new(),
            daily: BTreeMap::new(),
        };
        let view = TokenView::from_pair("demo", &record, now);
        let json = serde_json::to_value(view).expect("token view should serialize");
        assert_eq!(json["other_uid"], json["otherUid"]);
    }

    #[test]
    fn identifiers_are_normalized() {
        assert_eq!(normalize_identifier("  my user  ", "fallback"), "myuser");
        assert_eq!(normalize_identifier("  !@#  ", "fallback"), "fallback");
        assert_eq!(
            normalize_label(Some("  home profile  ".into())),
            "home profile"
        );
    }
}
