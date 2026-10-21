//! Provider registry endpoints: registration, heartbeat, and unregister.

use axum::{
    extract::State,
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde::Deserialize;
use serde_json::json;
use uuid::Uuid;

use crate::auth::{hash_heartbeat_token, verify_heartbeat_token};
use crate::models::{ProviderAddress, ProviderRecord};
use crate::state::{save_providers, AppState};
use crate::util::{constant_time_eq_str, unix_now};

#[derive(Debug, Deserialize)]
pub(crate) struct RegistryRegister {
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
pub(crate) struct RegistryHeartbeat {
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
pub(crate) struct RegistryIdentity {
    provider_id: String,
    heartbeat_token: String,
}

fn default_provider_heartbeat_secs() -> u64 {
    30
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
    let configured: String = state
        .config
        .read()
        .await
        .server
        .provider_registry_token
        .clone();
    let Some(presented) = registry_token(headers) else {
        return false;
    };
    if configured.is_empty() {
        return false;
    }
    constant_time_eq_str(presented, &configured)
}

pub(crate) async fn registry_register(
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

pub(crate) async fn registry_heartbeat(
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

pub(crate) async fn registry_unregister(
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

pub(crate) fn valid_provider_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.'))
}

pub(crate) fn normalize_addresses(addresses: Vec<ProviderAddress>) -> Vec<ProviderAddress> {
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

pub(crate) fn normalize_links(links: Vec<String>) -> Vec<String> {
    links
        .into_iter()
        .map(|link| link.trim().to_owned())
        .filter(|link| {
            if link.len() > 8192 {
                return false;
            }
            let (nodes, errors) = vpn233_core_lib_rust::parse_subscription(link);
            !nodes.is_empty() && errors.is_empty()
        })
        .take(256)
        .collect()
}
