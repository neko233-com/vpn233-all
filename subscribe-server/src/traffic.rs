//! Traffic analytics: relays vpn233-client stats when configured, otherwise
//! aggregates the subscription server's own byte counters.

use std::collections::BTreeMap;

use axum::{
    extract::{Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tracing::warn;

use crate::auth::admin_authorized;
use crate::models::TrafficDay;
use crate::state::AppState;
use crate::util::unix_now;

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct ClientTrafficDay {
    #[serde(default)]
    upload_bytes: u64,
    #[serde(default)]
    download_bytes: u64,
    #[serde(default)]
    requests: u64,
    #[serde(default)]
    connections: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct ClientTrafficDomain {
    #[serde(default)]
    upload_bytes: u64,
    #[serde(default)]
    download_bytes: u64,
    #[serde(default)]
    requests: u64,
    #[serde(default)]
    last_seen: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct ClientTrafficStats {
    #[serde(default)]
    daily: BTreeMap<String, ClientTrafficDay>,
    #[serde(default)]
    domains: BTreeMap<String, ClientTrafficDomain>,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct TrafficDayView {
    date: String,
    upload_bytes: u64,
    download_bytes: u64,
    total_bytes: u64,
    requests: u64,
    connections: u64,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct TrafficDomainView {
    domain: String,
    upload_bytes: u64,
    download_bytes: u64,
    total_bytes: u64,
    requests: u64,
    last_seen: u64,
}

fn day_view(date: String, day: TrafficDay) -> TrafficDayView {
    TrafficDayView {
        date,
        upload_bytes: day.upload_bytes,
        download_bytes: day.download_bytes,
        total_bytes: day.upload_bytes.saturating_add(day.download_bytes),
        requests: day.requests,
        connections: day.connections,
    }
}

fn client_day_view(date: String, day: ClientTrafficDay) -> TrafficDayView {
    day_view(
        date,
        TrafficDay {
            upload_bytes: day.upload_bytes,
            download_bytes: day.download_bytes,
            requests: day.requests,
            connections: day.connections,
        },
    )
}

fn selected_days<T>(mut days: Vec<T>, limit: usize) -> Vec<T> {
    if days.len() > limit {
        let keep_from = days.len() - limit;
        days.drain(..keep_from);
    }
    days
}

fn traffic_response(
    source: &'static str,
    available: bool,
    note: &'static str,
    daily: Vec<TrafficDayView>,
    domains: Vec<TrafficDomainView>,
    range_days: usize,
) -> Response {
    let domain_count = domains.len();
    let upload_bytes = daily.iter().map(|day| day.upload_bytes).sum::<u64>();
    let download_bytes = daily.iter().map(|day| day.download_bytes).sum::<u64>();
    let requests = daily.iter().map(|day| day.requests).sum::<u64>();
    let connections = daily.iter().map(|day| day.connections).sum::<u64>();
    Json(json!({
        "source": source,
        "available": available,
        "note": note,
        "observed_at": unix_now(),
        "range_days": range_days,
        "daily": daily,
        "domains": domains,
        "summary": {
            "upload_bytes": upload_bytes,
            "download_bytes": download_bytes,
            "total_bytes": upload_bytes.saturating_add(download_bytes),
            "requests": requests,
            "connections": connections,
            "domains": domain_count,
        },
    }))
    .into_response()
}

fn client_domain_views(stats: &ClientTrafficStats) -> Vec<TrafficDomainView> {
    let mut domains = stats
        .domains
        .iter()
        .map(|(domain, item)| TrafficDomainView {
            domain: domain.clone(),
            upload_bytes: item.upload_bytes,
            download_bytes: item.download_bytes,
            total_bytes: item.upload_bytes.saturating_add(item.download_bytes),
            requests: item.requests,
            last_seen: item.last_seen,
        })
        .collect::<Vec<_>>();
    domains.sort_by(|left, right| {
        right
            .total_bytes
            .cmp(&left.total_bytes)
            .then_with(|| left.domain.cmp(&right.domain))
    });
    domains.truncate(100);
    domains
}

pub(crate) async fn admin_traffic(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<std::collections::HashMap<String, String>>,
) -> Response {
    if !admin_authorized(&state, &headers).await {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let range_days = query
        .get("days")
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(30)
        .clamp(1, 400);
    let client_config = state.config.read().await.server.clone();
    if !client_config.client_stats_url.trim().is_empty() {
        let request = state
            .client
            .get(client_config.client_stats_url.trim())
            .basic_auth(
                client_config.client_stats_user,
                Some(client_config.client_stats_password),
            );
        if let Ok(response) = request.send().await {
            if response.status().is_success() {
                if let Ok(stats) = response.json::<ClientTrafficStats>().await {
                    let domains = client_domain_views(&stats);
                    let daily = selected_days(
                        stats
                            .daily
                            .into_iter()
                            .map(|(date, day)| client_day_view(date, day))
                            .collect(),
                        range_days,
                    );
                    return traffic_response(
                        "client",
                        true,
                        "已接入 vpn233-client，域名按代理请求目标统计；HTTPS CONNECT 以目标主机名统计。",
                        daily,
                        domains,
                        range_days,
                    );
                }
            }
        }
        warn!("client traffic stats fetch failed; falling back to subscription usage");
    }

    let usage = state.usage.read().await;
    let mut aggregate = BTreeMap::<String, TrafficDay>::new();
    for record in usage.tokens.values() {
        for (date, day) in &record.daily {
            let target = aggregate.entry(date.clone()).or_default();
            target.upload_bytes = target.upload_bytes.saturating_add(day.upload_bytes);
            target.download_bytes = target.download_bytes.saturating_add(day.download_bytes);
            target.requests = target.requests.saturating_add(day.requests);
            target.connections = target.connections.saturating_add(day.connections);
        }
    }
    let daily = selected_days(
        aggregate
            .into_iter()
            .map(|(date, day)| day_view(date, day))
            .collect(),
        range_days,
    );
    traffic_response(
        "subscription_server",
        false,
        "当前仅统计订阅响应大小；要查看实际代理流量与 HTTP 域名，请配置 vpn233-client 统计接口。",
        daily,
        Vec::new(),
        range_days,
    )
}
