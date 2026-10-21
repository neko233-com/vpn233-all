//! Quota accounting: period keys, automatic resets, and token expiry.

use crate::models::{AccessDuration, QuotaPeriod, UsageRecord, UsageStore};
use crate::util::{civil_date, days_in_month, month_index, month_key};

/// First day of the next quota cycle, if the token has a recurring period.
pub(crate) fn next_reset_at(record: &UsageRecord, now: u64) -> Option<u64> {
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

/// Expiry timestamp for a relative access duration.
pub(crate) fn expiry_for_duration(now: u64, duration: AccessDuration) -> u64 {
    match duration {
        AccessDuration::SevenDays => now.saturating_add(7 * 86_400),
        AccessDuration::OneMonth => crate::util::add_months(now, 1),
        AccessDuration::OneYear => crate::util::add_months(now, 12),
    }
}

/// Builds the identifier of the quota cycle that `timestamp` belongs to.
pub(crate) fn quota_period_key(record: &UsageRecord, timestamp: u64) -> Option<String> {
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

/// Resets usage counters when the token enters a new quota cycle.
pub(crate) fn refresh_quota(record: &mut UsageRecord, now: u64) -> bool {
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

/// Disables every token whose expiry has passed.
pub(crate) fn expire_tokens(store: &mut UsageStore, now: u64) -> bool {
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

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::util::{add_months, month_key, unix_now};

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
            account_id: "legacy".into(),
            group_id: "all".into(),
            label: String::new(),
            daily: BTreeMap::new(),
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
        assert_eq!(next_reset_at(&record, unix_now()), None);
    }

    #[test]
    fn expired_tokens_are_disabled_with_reason() {
        let mut store = UsageStore::default();
        let mut record = test_record(unix_now(), QuotaPeriod::Lifetime);
        record.expires_at = Some(100);
        store.tokens.insert("demo".into(), record);
        assert!(expire_tokens(&mut store, 200));
        let record = store.tokens.get("demo").expect("token");
        assert!(!record.enabled);
        assert_eq!(record.disabled_reason.as_deref(), Some("expired"));
        assert!(!expire_tokens(&mut store, 300));
    }
}
