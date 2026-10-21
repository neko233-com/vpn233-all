//! Subscription endpoints: authorization, node collection, format rendering,
//! byte accounting, and response headers.
//!
//! All format-specific parsing and rendering is delegated to
//! `vpn233-core-lib-rust`; this module owns authentication, billing, and the
//! HTTP contract (content type, filename, and telemetry headers).

use std::collections::{HashMap, HashSet};

use axum::{
    extract::{OriginalUri, Path, Query, State},
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use tracing::warn;
use url::Url;
use vpn233_core_lib_rust::{
    parse_subscription, render_subscription as render_subscription_body, LinkError, ProxyNode,
    SubscriptionFormat,
};

use crate::auth::{verify_record_password, verify_subscription_jwt};
use crate::config::SubscriptionSource;
use crate::models::{default_all_group_id, default_legacy_account_id};
use crate::quota::{expire_tokens, refresh_quota};
use crate::state::{save_usage, AppState, MAX_UPSTREAM_BODY_BYTES};
use crate::util::unix_now;

/// `GET /sub` renders the configured default format.
pub(crate) async fn subscription_default(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<HashMap<String, String>>,
) -> Response {
    render_subscription(&state, &headers, &query, None, None).await
}

/// `GET /sub/c` renders Clash YAML for every visible node.
pub(crate) async fn subscription_clash(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<HashMap<String, String>>,
) -> Response {
    render_subscription(
        &state,
        &headers,
        &query,
        None,
        Some(SubscriptionFormat::Clash),
    )
    .await
}

/// `GET /sub/c/{name}` renders Clash YAML for one named source or provider.
pub(crate) async fn subscription_clash_named(
    State(state): State<AppState>,
    Path(name): Path<String>,
    headers: HeaderMap,
    Query(query): Query<HashMap<String, String>>,
) -> Response {
    render_subscription(
        &state,
        &headers,
        &query,
        Some(name),
        Some(SubscriptionFormat::Clash),
    )
    .await
}

/// Handler shared by the explicitly registered `/sub/<format>` routes.
///
/// The format is resolved from the request path so every alias route
/// (`/sub/mihomo`, `/sub/sing-box`, ...) can share one implementation while
/// still being registered explicitly (avoiding wildcard route conflicts).
pub(crate) async fn subscription_format_route(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<HashMap<String, String>>,
    OriginalUri(uri): OriginalUri,
) -> Response {
    let Some(format) = route_format(uri.path()) else {
        return (StatusCode::BAD_REQUEST, "unknown subscription format").into_response();
    };
    render_subscription(&state, &headers, &query, None, Some(format)).await
}

/// Catch-all `/sub/{format}` route: unknown formats are a client error.
pub(crate) async fn subscription_any_format(
    State(state): State<AppState>,
    Path(format): Path<String>,
    headers: HeaderMap,
    Query(query): Query<HashMap<String, String>>,
) -> Response {
    let Some(format) = SubscriptionFormat::from_alias(&format) else {
        return (StatusCode::BAD_REQUEST, "unknown subscription format").into_response();
    };
    render_subscription(&state, &headers, &query, None, Some(format)).await
}

fn route_format(path: &str) -> Option<SubscriptionFormat> {
    let id = path.strip_prefix("/sub/")?;
    SubscriptionFormat::from_alias(id)
}

/// Renders one subscription format for an authorized token.
pub(crate) async fn render_subscription(
    state: &AppState,
    headers: &HeaderMap,
    query: &HashMap<String, String>,
    selected: Option<String>,
    route_format: Option<SubscriptionFormat>,
) -> Response {
    let format = match resolve_format(state, query, route_format).await {
        Ok(format) => format,
        Err(response) => return response,
    };
    let Some(token) = token_allowed(state, headers, query).await else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    let group_id = token_group_id(state, &token).await;
    let links = if group_id == "all" {
        sources(state, selected.as_deref()).await
    } else {
        group_sources(state, &group_id).await
    };
    let (nodes, parse_errors) = parse_source_payloads(&links);
    let skipped = parse_errors.len()
        + nodes
            .iter()
            .filter(|node| !format.supports_node(node))
            .count();
    let profile_name = subscription_profile_name(query);
    let body = match render_subscription_body(format, &nodes, &profile_name) {
        Ok(body) => body,
        Err(error) => {
            warn!(
                format = format.canonical_id(),
                error = %error,
                "subscription rendering failed"
            );
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                "subscription rendering failed",
            )
                .into_response();
        }
    };
    if !charge_token(state, &token, body.len() as u64).await {
        return (StatusCode::TOO_MANY_REQUESTS, "subscription quota exceeded").into_response();
    }
    let filename = subscription_filename(query, format.ext());
    let mut response =
        subscription_response(state, &token, body, format.content_type(), &filename).await;
    let telemetry = response.headers_mut();
    if let Ok(value) = HeaderValue::from_str(format.canonical_id()) {
        telemetry.insert("x-vpn233-format", value);
    }
    if let Ok(value) = HeaderValue::from_str(&nodes.len().to_string()) {
        telemetry.insert("x-vpn233-nodes", value);
    }
    let rendered_nodes = nodes
        .iter()
        .filter(|node| format.supports_node(node))
        .count();
    if let Ok(value) = HeaderValue::from_str(&rendered_nodes.to_string()) {
        telemetry.insert("x-vpn233-rendered", value);
    }
    if let Ok(value) = HeaderValue::from_str(&skipped.to_string()) {
        telemetry.insert("x-vpn233-skipped", value);
    }
    response
}

async fn resolve_format(
    state: &AppState,
    query: &HashMap<String, String>,
    route_format: Option<SubscriptionFormat>,
) -> Result<SubscriptionFormat, Response> {
    if let Some(raw) = query.get("format") {
        return match SubscriptionFormat::from_alias(raw) {
            Some(format) => Ok(format),
            None => Err((StatusCode::BAD_REQUEST, "unknown subscription format").into_response()),
        };
    }
    if let Some(format) = route_format {
        return Ok(format);
    }
    let configured = state.config.read().await.server.default_format.clone();
    match SubscriptionFormat::from_alias(&configured) {
        Some(format) => Ok(format),
        None => {
            warn!(
                configured = %configured,
                "unknown default_format configured; falling back to clash"
            );
            Ok(SubscriptionFormat::Clash)
        }
    }
}

/// Resolves the subscription token presented via query, headers, or Basic auth.
pub(crate) async fn token_allowed(
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
                .and_then(|value| value.to_str().ok())
                .map(str::to_owned)
        })
        .or_else(|| basic.as_ref().map(|(token, _)| token.clone()));
    let token = candidate?;
    let access_token = query.get("access_token").cloned().or_else(|| {
        headers
            .get("x-access-jwt")
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned)
    });
    let password = query
        .get("password")
        .cloned()
        .or_else(|| {
            headers
                .get("x-access-password")
                .and_then(|value| value.to_str().ok())
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
    let now = unix_now();
    let mut usage = state.usage.write().await;
    let mut changed = expire_tokens(&mut usage, now);
    let (authorized, account_id) = if let Some(record) = usage.tokens.get_mut(&token) {
        changed |= refresh_quota(record, now);
        (
            record.enabled
                && record.expires_at.is_none_or(|expires| expires > now)
                && (jwt_authorized
                    || password
                        .as_deref()
                        .is_some_and(|password| verify_record_password(record, password))),
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

/// Charges `body_bytes` against the token quota; `false` means quota exceeded.
pub(crate) async fn charge_token(state: &AppState, token: &str, body_bytes: u64) -> bool {
    let now = unix_now();
    let timezone_offset_hours = state
        .config
        .read()
        .await
        .server
        .traffic_timezone_offset_hours;
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
    let day = record
        .daily
        .entry(crate::util::date_key(now, timezone_offset_hours))
        .or_default();
    day.download_bytes = day.download_bytes.saturating_add(body_bytes);
    day.requests = day.requests.saturating_add(1);
    day.connections = day.connections.saturating_add(1);
    while record.daily.len() > 400 {
        let Some(first) = record.daily.keys().next().cloned() else {
            break;
        };
        record.daily.remove(&first);
    }
    drop(usage);
    if let Err(error) = save_usage(state).await {
        warn!(%error, "usage persistence failed");
    }
    true
}

/// Collects `(source name, node link)` pairs from config and provider registry.
pub(crate) async fn sources(state: &AppState, selected: Option<&str>) -> Vec<(String, String)> {
    let config = state.config.read().await.clone();
    let mut result = Vec::new();
    let mut seen_links = HashSet::new();
    for source in config
        .subscriptions
        .iter()
        .filter(|source| selected.map(|name| name == source.name).unwrap_or(true))
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
            if let Some(text) = fetch_remote_source(state, source).await {
                // Preserve structured node options; conversion through URI
                // strings would discard fields such as UDP and TLS settings.
                if seen_links.insert(text.clone()) {
                    result.push((source.name.clone(), text));
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

/// Parses source payloads once and de-duplicates identical node definitions.
/// The source payloads and parse errors are never written to logs.
pub(crate) fn parse_source_payloads(
    sources: &[(String, String)],
) -> (Vec<ProxyNode>, Vec<LinkError>) {
    let mut nodes = Vec::new();
    let mut errors = Vec::new();
    let mut seen = HashSet::new();
    for (_, payload) in sources {
        let (parsed, failed) = parse_subscription(payload);
        errors.extend(failed);
        for node in parsed {
            let identity =
                serde_json::to_string(&vpn233_core_lib_rust::nodes::strip_internal_keys(&node))
                    .unwrap_or_default();
            if seen.insert(identity) {
                nodes.push(node);
            }
        }
    }
    (nodes, errors)
}

/// Fetches one upstream subscription with scheme, timeout, size, and encoding
/// hardening. Errors never log the full URL (it may carry credentials).
async fn fetch_remote_source(state: &AppState, source: &SubscriptionSource) -> Option<String> {
    let parsed = match Url::parse(source.url.trim()) {
        Ok(url) => url,
        Err(_) => {
            warn!("upstream subscription URL is invalid");
            return None;
        }
    };
    if !matches!(parsed.scheme(), "http" | "https") {
        warn!("unsupported upstream subscription scheme");
        return None;
    }
    let request = state.client.get(parsed);
    let request = if source.access_token.trim().is_empty() {
        request
    } else {
        request.header("x-provider-token", source.access_token.trim())
    };
    let response = match request.send().await {
        Ok(response) => response,
        Err(_) => {
            warn!("upstream subscription fetch failed");
            return None;
        }
    };
    let response = match response.error_for_status() {
        Ok(response) => response,
        Err(_) => {
            warn!("upstream subscription returned an error status");
            return None;
        }
    };
    let body = match read_limited_body(response).await {
        Ok(body) => body,
        Err(_) => {
            warn!("upstream subscription body rejected");
            return None;
        }
    };
    match String::from_utf8(body) {
        Ok(text) => Some(text),
        Err(_) => {
            warn!("upstream subscription body is not valid UTF-8");
            None
        }
    }
}

async fn read_limited_body(mut response: reqwest::Response) -> anyhow::Result<Vec<u8>> {
    if let Some(length) = response.content_length() {
        if length > MAX_UPSTREAM_BODY_BYTES as u64 {
            anyhow::bail!("body exceeds {MAX_UPSTREAM_BODY_BYTES} bytes");
        }
    }
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        if body.len().saturating_add(chunk.len()) > MAX_UPSTREAM_BODY_BYTES {
            anyhow::bail!("body exceeds {MAX_UPSTREAM_BODY_BYTES} bytes");
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

/// Links belonging to one explicit node group (`all` is handled by [`sources`]).
pub(crate) async fn group_sources(state: &AppState, group_id: &str) -> Vec<(String, String)> {
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

/// Group id bound to a subscription token.
pub(crate) async fn token_group_id(state: &AppState, token: &str) -> String {
    state
        .usage
        .read()
        .await
        .tokens
        .get(token)
        .map(|record| record.group_id.clone())
        .unwrap_or_else(default_all_group_id)
}

/// Requested profile name before extension handling.
pub(crate) fn subscription_profile_name(query: &HashMap<String, String>) -> String {
    query
        .get("filename")
        .or_else(|| query.get("profile"))
        .map(String::as_str)
        .unwrap_or("vpn233")
        .to_owned()
}

/// Builds the response filename from `filename`/`profile` query parameters.
pub(crate) fn subscription_filename(query: &HashMap<String, String>, extension: &str) -> String {
    sanitize_subscription_filename(&subscription_profile_name(query), extension)
}

/// Sanitizes a user supplied filename: ASCII only, no path separators, and a
/// forced subscription extension.
pub(crate) fn sanitize_subscription_filename(raw: &str, extension: &str) -> String {
    let mut name: String = raw
        .trim()
        .trim_matches(['"', '\'', '\\'])
        .trim()
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.') {
                character
            } else {
                '_'
            }
        })
        .collect();
    name = name.trim_matches(['.', '_']).to_owned();
    if name.is_empty() {
        name = "vpn233".into();
    }
    let suffix = format!(".{extension}");
    let lower = name.to_ascii_lowercase();
    if !lower.ends_with(&suffix) {
        for known in [".yaml", ".json", ".txt", ".conf"] {
            if lower.ends_with(known) {
                name.truncate(name.len() - known.len());
                break;
            }
        }
        name.push_str(&suffix);
    }
    name
}

/// Renders the common subscription headers around an already charged body.
pub(crate) async fn subscription_response(
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
    if let Ok(value) = HeaderValue::from_str(&format!("attachment; filename={filename}")) {
        headers.insert(header::CONTENT_DISPOSITION, value);
    }
    if let Ok(value) =
        HeaderValue::from_str(filename.rsplit_once('.').map_or(filename, |(stem, _)| stem))
    {
        headers.insert("profile-title", value);
    }
    (headers, body).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subscription_filenames_are_safe_for_clash_import() {
        let mut query = HashMap::new();
        query.insert("filename".into(), "\"my home.yaml\"".into());
        let filename = subscription_filename(&query, "yaml");
        assert_eq!(filename, "my_home.yaml");
        assert!(!format!("attachment; filename={filename}").contains('"'));
        assert_eq!(
            sanitize_subscription_filename("家庭配置", "yaml"),
            "vpn233.yaml"
        );
        assert_eq!(
            sanitize_subscription_filename("../x", "yaml"),
            "x.yaml",
            "path traversal must be stripped"
        );
        assert_eq!(
            sanitize_subscription_filename("config.json", "yaml"),
            "config.yaml"
        );
        assert_eq!(
            sanitize_subscription_filename("app.conf", "conf"),
            "app.conf"
        );
    }

    #[test]
    fn route_formats_resolve_aliases() {
        assert_eq!(route_format("/sub/clash"), Some(SubscriptionFormat::Clash));
        assert_eq!(route_format("/sub/mihomo"), Some(SubscriptionFormat::Clash));
        assert_eq!(
            route_format("/sub/sing-box"),
            Some(SubscriptionFormat::SingBox)
        );
        assert_eq!(
            route_format("/sub/hiddify"),
            Some(SubscriptionFormat::V2Ray)
        );
        assert_eq!(
            route_format("/sub/qx"),
            Some(SubscriptionFormat::QuantumultX)
        );
        assert_eq!(route_format("/sub/unknown-format"), None);
        assert_eq!(route_format("/sub"), None);
    }
}
