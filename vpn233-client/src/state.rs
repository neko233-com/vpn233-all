//! Runtime state, traffic statistics and debounced persistence.

use crate::config::Config;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    net::IpAddr,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tokio::{
    io::{AsyncRead, AsyncWrite, AsyncWriteExt},
    sync::RwLock,
};
use tracing::warn;

/// Minimum interval between two stats file writes (debounce).
pub const STATS_PERSIST_INTERVAL: Duration = Duration::from_secs(1);
const MAX_DAILY_BUCKETS: usize = 400;
const MAX_DOMAIN_BUCKETS: usize = 2_000;

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct Stats {
    pub active_connections: usize,
    pub total_connections: u64,
    pub bytes_up: u64,
    pub bytes_down: u64,
    #[serde(default)]
    pub daily: BTreeMap<String, TrafficBucket>,
    /// Destination domains; always empty unless `record_domains` is enabled.
    #[serde(default)]
    pub domains: BTreeMap<String, DomainBucket>,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct TrafficBucket {
    pub upload_bytes: u64,
    pub download_bytes: u64,
    pub requests: u64,
    pub connections: u64,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct DomainBucket {
    pub upload_bytes: u64,
    pub download_bytes: u64,
    pub requests: u64,
    pub last_seen: u64,
}

/// Shared client state.
#[derive(Clone)]
pub struct AppState {
    config: Arc<RwLock<Config>>,
    stats: Arc<RwLock<Stats>>,
    stats_file: Arc<PathBuf>,
    stats_timezone_offset_hours: i32,
    last_persist: Arc<Mutex<Instant>>,
    persist_lock: Arc<tokio::sync::Mutex<()>>,
    persist_count: Arc<AtomicU64>,
}

impl AppState {
    /// Create state from a validated configuration, loading previous statistics.
    pub async fn new(config: Config) -> Result<Self> {
        let stats_file = config.stats_file.clone();
        let mut loaded_stats = load_stats(&stats_file).await?;
        loaded_stats.active_connections = 0;
        if !config.record_domains {
            // Honor opt-out even for a statistics file from an older version.
            loaded_stats.domains.clear();
        }
        let stats_timezone_offset_hours = config.stats_timezone_offset_hours;
        Ok(Self {
            config: Arc::new(RwLock::new(config)),
            stats: Arc::new(RwLock::new(loaded_stats)),
            stats_file: Arc::new(stats_file),
            stats_timezone_offset_hours,
            last_persist: Arc::new(Mutex::new(
                Instant::now() - STATS_PERSIST_INTERVAL - Duration::from_secs(1),
            )),
            persist_lock: Arc::new(tokio::sync::Mutex::new(())),
            persist_count: Arc::new(AtomicU64::new(0)),
        })
    }

    pub fn config(&self) -> &Arc<RwLock<Config>> {
        &self.config
    }

    pub async fn config_snapshot(&self) -> Config {
        self.config.read().await.clone()
    }

    pub async fn stats_snapshot(&self) -> Stats {
        self.stats.read().await.clone()
    }

    pub fn stats_file(&self) -> &Path {
        &self.stats_file
    }

    pub fn stats_timezone_offset_hours(&self) -> i32 {
        self.stats_timezone_offset_hours
    }

    /// Number of successful stats file writes (test/diagnostic helper).
    pub fn persist_count(&self) -> u64 {
        self.persist_count.load(Ordering::Relaxed)
    }

    /// Register a newly accepted proxy connection.
    pub async fn begin_connection(&self) {
        let mut stats = self.stats.write().await;
        stats.active_connections = stats.active_connections.saturating_add(1);
        stats.total_connections = stats.total_connections.saturating_add(1);
    }

    /// Register the end of a proxy connection.
    pub async fn end_connection(&self) {
        let mut stats = self.stats.write().await;
        stats.active_connections = stats.active_connections.saturating_sub(1);
    }

    /// Record transferred bytes for `host` (domain recording is opt-in).
    pub async fn record_usage(&self, host: &str, upload_bytes: u64, download_bytes: u64) {
        let (record_domains, timezone_offset_hours) = {
            let config = self.config.read().await;
            (config.record_domains, config.stats_timezone_offset_hours)
        };
        let now = unix_now();
        let mut stats = self.stats.write().await;
        stats.bytes_up = stats.bytes_up.saturating_add(upload_bytes);
        stats.bytes_down = stats.bytes_down.saturating_add(download_bytes);
        let day = stats
            .daily
            .entry(date_key(now, timezone_offset_hours))
            .or_default();
        day.upload_bytes = day.upload_bytes.saturating_add(upload_bytes);
        day.download_bytes = day.download_bytes.saturating_add(download_bytes);
        day.requests = day.requests.saturating_add(1);
        day.connections = day.connections.saturating_add(1);
        trim_map(&mut stats.daily, MAX_DAILY_BUCKETS);
        if record_domains {
            if let Some(domain) = normalize_destination(host) {
                let bucket = stats.domains.entry(domain).or_default();
                bucket.upload_bytes = bucket.upload_bytes.saturating_add(upload_bytes);
                bucket.download_bytes = bucket.download_bytes.saturating_add(download_bytes);
                bucket.requests = bucket.requests.saturating_add(1);
                bucket.last_seen = now;
            }
            trim_map(&mut stats.domains, MAX_DOMAIN_BUCKETS);
        }
        drop(stats);
        self.persist_if_due().await;
    }

    /// Persist stats when at least [`STATS_PERSIST_INTERVAL`] elapsed.
    pub async fn persist_if_due(&self) -> bool {
        match self.persist_locked(false).await {
            Ok(saved) => saved,
            Err(error) => {
                warn!(%error, "traffic stats persistence failed");
                false
            }
        }
    }

    /// Force a stats flush (used on shutdown and by tests).
    pub async fn flush(&self) -> Result<()> {
        self.persist_locked(true).await.map(|_saved| ())
    }

    async fn persist_locked(&self, force: bool) -> Result<bool> {
        let _guard = self.persist_lock.lock().await;
        {
            let last = self.last_persist.lock().expect("stats persist clock");
            if !force && last.elapsed() < STATS_PERSIST_INTERVAL {
                return Ok(false);
            }
        }
        *self.last_persist.lock().expect("stats persist clock") = Instant::now();
        self.save_stats().await?;
        self.persist_count.fetch_add(1, Ordering::Relaxed);
        Ok(true)
    }

    async fn save_stats(&self) -> Result<()> {
        let snapshot = self.stats.read().await.clone();
        let serialized = serde_json::to_vec_pretty(&snapshot).context("serialize traffic stats")?;
        if let Some(parent) = self.stats_file.parent() {
            if !parent.as_os_str().is_empty() {
                tokio::fs::create_dir_all(parent)
                    .await
                    .context("create stats directory")?;
            }
        }
        let temporary = temporary_stats_path(&self.stats_file);
        tokio::fs::write(&temporary, &serialized)
            .await
            .context("write traffic stats")?;
        restrict_permissions(&temporary).await?;
        tokio::fs::rename(&temporary, self.stats_file.as_ref())
            .await
            .context("replace traffic stats")?;
        restrict_permissions(&self.stats_file).await?;
        Ok(())
    }
}

async fn load_stats(path: &Path) -> Result<Stats> {
    if !tokio::fs::try_exists(path).await.unwrap_or(false) {
        return Ok(Stats::default());
    }
    let raw = tokio::fs::read(path).await.context("read traffic stats")?;
    let stats = serde_json::from_slice::<Stats>(&raw).context("parse traffic stats")?;
    Ok(stats)
}

fn temporary_stats_path(path: &Path) -> PathBuf {
    path.with_extension("json.tmp")
}

/// Restrict the statistics file to the current user (best effort on Windows).
async fn restrict_permissions(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let permissions = std::fs::Permissions::from_mode(0o600);
        tokio::fs::set_permissions(path, permissions)
            .await
            .context("restrict stats file permissions")?;
    }
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

fn trim_map<T>(map: &mut BTreeMap<String, T>, limit: usize) {
    while map.len() > limit {
        let Some(first) = map.keys().next().cloned() else {
            break;
        };
        map.remove(&first);
    }
}

/// Relay bytes in both directions, recording upload/download traffic.
pub async fn relay<A, B>(left: &mut A, right: &mut B, state: &AppState, host: &str) -> Result<()>
where
    A: AsyncRead + AsyncWrite + Unpin,
    B: AsyncRead + AsyncWrite + Unpin,
{
    let (mut left_read, mut left_write) = tokio::io::split(left);
    let (mut right_read, mut right_write) = tokio::io::split(right);
    let upload = async {
        let bytes = tokio::io::copy(&mut left_read, &mut right_write).await?;
        right_write.shutdown().await?;
        Ok::<u64, std::io::Error>(bytes)
    };
    let download = async {
        let bytes = tokio::io::copy(&mut right_read, &mut left_write).await?;
        left_write.shutdown().await?;
        Ok::<u64, std::io::Error>(bytes)
    };
    let (up, down) = tokio::try_join!(upload, download)?;
    state.record_usage(host, up, down).await;
    Ok(())
}

pub fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// Fixed-offset civil date key (`YYYY-MM-DD`) for daily traffic buckets.
pub fn date_key(timestamp: u64, offset_hours: i32) -> String {
    let shifted = timestamp as i64 + i64::from(offset_hours) * 3_600;
    let days = shifted.div_euclid(86_400);
    let z = days + 719_468;
    let era = (if z >= 0 { z } else { z - 146_096 }) / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let mut year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let month_part = (5 * doy + 2) / 153;
    let day = doy - (153 * month_part + 2) / 5 + 1;
    let month = month_part + if month_part < 10 { 3 } else { -9 };
    year += i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}")
}

/// Normalize a destination host into a statistics bucket key.
pub fn normalize_destination(host: &str) -> Option<String> {
    let value = host.trim().trim_end_matches('.').to_ascii_lowercase();
    if value.is_empty() || value == "localhost" {
        return None;
    }
    if value.parse::<IpAddr>().is_ok() {
        return Some(format!("[IP] {value}"));
    }
    Some(value.chars().take(253).collect())
}

/// Split `host:port`, tolerating bracketed IPv6 and a missing port.
pub fn split_host_port(value: &str, default_port: u16) -> (String, u16) {
    let value = value.trim();
    if let Some(rest) = value.strip_prefix('[') {
        if let Some((host, tail)) = rest.split_once(']') {
            let port = tail
                .strip_prefix(':')
                .and_then(|port| port.parse().ok())
                .unwrap_or(default_port);
            return (host.to_owned(), port);
        }
    }
    match value.rsplit_once(':') {
        Some((host, port)) if !host.is_empty() => match port.parse::<u16>() {
            Ok(port) => (host.to_owned(), port),
            Err(_) => (value.to_owned(), default_port),
        },
        _ => (value.to_owned(), default_port),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn date_key_uses_a_fixed_offset() {
        assert_eq!(date_key(0, 0), "1970-01-01");
        assert_eq!(date_key(0, 8), "1970-01-01");
        assert_eq!(date_key(1_700_000_000, 0), "2023-11-14");
        assert_eq!(date_key(1_700_000_000, 8), "2023-11-15");
        assert_eq!(date_key(1_700_000_000, -8), "2023-11-14");
    }

    #[test]
    fn destination_normalization_handles_ips_and_localhost() {
        assert_eq!(normalize_destination("localhost"), None);
        assert_eq!(normalize_destination(""), None);
        assert_eq!(normalize_destination("  .  "), None);
        assert_eq!(
            normalize_destination("Example.COM."),
            Some("example.com".to_owned())
        );
        assert_eq!(
            normalize_destination("203.0.113.10"),
            Some("[IP] 203.0.113.10".to_owned())
        );
        let long = format!("{}.invalid", "a".repeat(300));
        assert_eq!(
            normalize_destination(&long).map(|value| value.len()),
            Some(253)
        );
    }

    #[test]
    fn split_host_port_is_tolerant() {
        assert_eq!(
            split_host_port("example.invalid", 80),
            ("example.invalid".to_owned(), 80)
        );
        assert_eq!(
            split_host_port("example.invalid:8080", 80),
            ("example.invalid".to_owned(), 8080)
        );
        assert_eq!(
            split_host_port("[2001:db8::1]:1080", 80),
            ("2001:db8::1".to_owned(), 1080)
        );
        assert_eq!(
            split_host_port("[2001:db8::1]", 80),
            ("2001:db8::1".to_owned(), 80)
        );
        assert_eq!(
            split_host_port("example.invalid:not-a-port", 80),
            ("example.invalid:not-a-port".to_owned(), 80)
        );
    }

    #[test]
    fn unix_now_is_reasonable() {
        assert!(unix_now() > 1_600_000_000);
    }
}
