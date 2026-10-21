//! Small shared helpers: clocks, civil-date math, and constant-time comparison.

use std::time::{SystemTime, UNIX_EPOCH};

use subtle::ConstantTimeEq;

/// Current UNIX timestamp in whole seconds.
pub(crate) fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// Compares two byte strings without short-circuiting on content mismatch.
///
/// Length differences still short-circuit, matching [`subtle::ConstantTimeEq`]
/// semantics for slices; callers that compare secrets should hash to a fixed
/// length first when the length itself is sensitive.
pub(crate) fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    left.ct_eq(right).into()
}

/// Constant-time string comparison for credentials.
pub(crate) fn constant_time_eq_str(left: &str, right: &str) -> bool {
    constant_time_eq(left.as_bytes(), right.as_bytes())
}

/// Converts a UNIX timestamp into a civil `(year, month, day)` tuple.
pub(crate) fn civil_date(timestamp: u64) -> (i64, u32, u32) {
    let days = timestamp / 86_400;
    let mut year = 1970_i64;
    let mut remaining = days as i64;
    loop {
        let leap = is_leap_year(year);
        let year_days = if leap { 366 } else { 365 };
        if remaining < year_days {
            break;
        }
        remaining -= year_days;
        year += 1;
    }
    let leap = is_leap_year(year);
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

fn is_leap_year(year: i64) -> bool {
    year % 4 == 0 && (year % 100 != 0 || year % 400 == 0)
}

/// Calendar day key (`YYYY-MM-DD`) using a fixed UTC offset in hours.
pub(crate) fn date_key(timestamp: u64, offset_hours: i32) -> String {
    let shifted = timestamp as i64 + i64::from(offset_hours) * 3_600;
    let (year, month, day) = civil_date(shifted as u64);
    format!("{year:04}-{month:02}-{day:02}")
}

/// Calendar month key (`YYYY-MM`) in UTC.
pub(crate) fn month_key(timestamp: u64) -> String {
    let (year, month, _) = civil_date(timestamp);
    format!("{year:04}-{month:02}")
}

/// Absolute month index used for month arithmetic.
pub(crate) fn month_index(timestamp: u64) -> i64 {
    let (year, month, _) = civil_date(timestamp);
    year * 12 + month as i64 - 1
}

/// Number of days in the given civil month.
pub(crate) fn days_in_month(year: i64, month: u32) -> u32 {
    match month {
        2 if is_leap_year(year) => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

/// Adds calendar months while keeping the original day-of-month and time.
pub(crate) fn add_months(timestamp: u64, months: i64) -> u64 {
    let (_, _, day) = civil_date(timestamp);
    let index = month_index(timestamp) + months;
    let target_year = index.div_euclid(12);
    let target_month = (index.rem_euclid(12) + 1) as u32;
    let target_day = day.min(days_in_month(target_year, target_month));
    timestamp_for_date(target_year, target_month, target_day) + timestamp % 86_400
}

fn timestamp_for_date(year: i64, month: u32, day: u32) -> u64 {
    let mut days = 0_i64;
    for current_year in 1970..year {
        days += if is_leap_year(current_year) { 366 } else { 365 };
    }
    for current_month in 1..month {
        days += days_in_month(year, current_month) as i64;
    }
    (days + day as i64 - 1) as u64 * 86_400
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn civil_date_matches_known_epochs() {
        assert_eq!(civil_date(0), (1970, 1, 1));
        assert_eq!(civil_date(1_704_067_200), (2024, 1, 1));
        assert_eq!(civil_date(1_709_164_800), (2024, 2, 29));
        assert_eq!(date_key(1_704_067_200, 8), "2024-01-01");
        assert_eq!(date_key(1_704_067_200, -12), "2023-12-31");
    }

    #[test]
    fn constant_time_helpers_compare_correctly() {
        assert!(constant_time_eq_str("secret", "secret"));
        assert!(!constant_time_eq_str("secret", "secreT"));
        assert!(!constant_time_eq_str("secret", "longer-secret"));
    }
}
