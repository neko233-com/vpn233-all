//! Domain models and the on-disk JSON store layouts.
//!
//! Field names and defaults are part of the persistent data contract: they must
//! stay compatible with existing `usage.json`, `accounts.json`, `groups.json`,
//! and `providers.json` files.

use std::collections::{BTreeMap, HashMap};

use serde::{Deserialize, Serialize};

pub(crate) fn default_legacy_account_id() -> String {
    "legacy".into()
}

pub(crate) fn default_all_group_id() -> String {
    "all".into()
}

/// Quota reset period for a subscription token.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub(crate) enum QuotaPeriod {
    #[default]
    Lifetime,
    #[serde(rename = "calendar_month", alias = "monthly")]
    CalendarMonth,
    #[serde(rename = "monthly_from_creation", alias = "creation_day")]
    MonthlyFromCreation,
    Weekly,
    Daily,
}

impl QuotaPeriod {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Lifetime => "lifetime",
            Self::CalendarMonth => "calendar_month",
            Self::MonthlyFromCreation => "monthly_from_creation",
            Self::Weekly => "weekly",
            Self::Daily => "daily",
        }
    }
}

/// Convenience access duration accepted by token creation/renewal APIs.
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AccessDuration {
    SevenDays,
    OneMonth,
    OneYear,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct UsageStore {
    pub(crate) tokens: HashMap<String, UsageRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct UsageRecord {
    pub(crate) password_salt: String,
    pub(crate) password_hash: String,
    pub(crate) limit_bytes: u64,
    pub(crate) used_bytes: u64,
    pub(crate) requests: u64,
    pub(crate) enabled: bool,
    pub(crate) expires_at: Option<u64>,
    pub(crate) last_seen: Option<u64>,
    #[serde(default)]
    pub(crate) created_at: u64,
    #[serde(default)]
    pub(crate) quota_period: QuotaPeriod,
    #[serde(default)]
    pub(crate) period_key: Option<String>,
    #[serde(default)]
    pub(crate) other_uid: Option<String>,
    #[serde(default)]
    pub(crate) disabled_reason: Option<String>,
    #[serde(default)]
    pub(crate) retired_at: Option<u64>,
    #[serde(default = "default_legacy_account_id")]
    pub(crate) account_id: String,
    #[serde(default = "default_all_group_id")]
    pub(crate) group_id: String,
    #[serde(default)]
    pub(crate) label: String,
    #[serde(default)]
    pub(crate) daily: BTreeMap<String, TrafficDay>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct TrafficDay {
    pub(crate) upload_bytes: u64,
    pub(crate) download_bytes: u64,
    pub(crate) requests: u64,
    pub(crate) connections: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct AccountStore {
    pub(crate) accounts: HashMap<String, AccountRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct AccountRecord {
    pub(crate) account_id: String,
    pub(crate) username: String,
    pub(crate) display_name: String,
    pub(crate) enabled: bool,
    #[serde(default)]
    pub(crate) other_uid: Option<String>,
    pub(crate) created_at: u64,
    #[serde(default)]
    pub(crate) deleted_at: Option<u64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct GroupStore {
    pub(crate) groups: HashMap<String, SubscriptionGroup>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct SubscriptionGroup {
    pub(crate) group_id: String,
    pub(crate) name: String,
    #[serde(default)]
    pub(crate) node_links: Vec<String>,
    #[serde(default)]
    pub(crate) protected: bool,
    pub(crate) created_at: u64,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct NodeInfo {
    pub(crate) name: String,
    pub(crate) protocol: String,
    pub(crate) server: String,
    pub(crate) port: u16,
    pub(crate) endpoint: String,
    pub(crate) payload: String,
    pub(crate) link: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct ProviderAddress {
    pub(crate) kind: String,
    pub(crate) value: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct ProviderStore {
    pub(crate) providers: HashMap<String, ProviderRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct ProviderRecord {
    pub(crate) provider_id: String,
    pub(crate) name: String,
    pub(crate) version: String,
    pub(crate) addresses: Vec<ProviderAddress>,
    pub(crate) node_links: Vec<String>,
    pub(crate) enabled: bool,
    pub(crate) registered_at: u64,
    pub(crate) last_seen: u64,
    pub(crate) heartbeat_interval_secs: u64,
    pub(crate) heartbeat_token_hash: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_usage_records_default_missing_fields() {
        let value = serde_json::json!({
            "tokens": {
                "legacy-token": {
                    "password_salt": "",
                    "password_hash": "",
                    "limit_bytes": 100,
                    "used_bytes": 0,
                    "requests": 0,
                    "enabled": true,
                    "expires_at": null,
                    "last_seen": null
                }
            }
        })
        .to_string();
        let store: UsageStore = serde_json::from_str(&value).expect("legacy usage JSON loads");
        let record = store.tokens.get("legacy-token").expect("legacy token");
        assert_eq!(record.quota_period, QuotaPeriod::Lifetime);
        assert_eq!(record.account_id, "legacy");
        assert_eq!(record.group_id, "all");
        assert_eq!(record.label, "");
        assert!(record.daily.is_empty());
    }
}
