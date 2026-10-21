//! Application state, persistence for the JSON data stores, and the usage
//! reaper task.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use axum::http::HeaderValue;
use tokio::{fs, sync::RwLock};
use tracing::warn;

use crate::auth::LoginRateLimiter;
use crate::config::Config;
use crate::models::{
    AccountRecord, AccountStore, GroupStore, ProviderStore, SubscriptionGroup, UsageStore,
};
use crate::quota::{expire_tokens, refresh_quota};
use crate::util::unix_now;

/// Upper bound accepted for one upstream subscription response body.
pub(crate) const MAX_UPSTREAM_BODY_BYTES: usize = 8 * 1024 * 1024;
/// Timeout applied to every outbound HTTP request made by the server.
pub(crate) const UPSTREAM_TIMEOUT: Duration = Duration::from_secs(10);

/// Shared, cloneable application state.
///
/// The persisted stores are kept behind async locks; the config is reloaded
/// only at startup but stays behind a lock for API symmetry with the previous
/// implementation.
#[derive(Clone)]
pub struct AppState {
    pub(crate) config: Arc<RwLock<Config>>,
    pub(crate) client: reqwest::Client,
    pub(crate) usage: Arc<RwLock<UsageStore>>,
    pub(crate) providers: Arc<RwLock<ProviderStore>>,
    pub(crate) accounts: Arc<RwLock<AccountStore>>,
    pub(crate) groups: Arc<RwLock<GroupStore>>,
    pub(crate) login_limiter: Arc<LoginRateLimiter>,
    pub(crate) cors_origins: Arc<Vec<HeaderValue>>,
    pub(crate) web_dir: Arc<PathBuf>,
}

/// Loads every persisted store and builds the shared state.
pub async fn init_state(config: Config) -> Result<AppState> {
    let data_dir = PathBuf::from(&config.server.data_dir);
    let cors_origins = parse_origins(&config.server.admin_origins);
    let web_dir = Arc::new(PathBuf::from(&config.server.web_dir));
    fs::create_dir_all(&data_dir)
        .await
        .context("cannot create subscribe data directory")?;
    let usage = load_usage(&data_dir)
        .await
        .context("cannot load subscription usage store")?;
    let accounts = load_accounts(&data_dir)
        .await
        .context("cannot load accounts store")?;
    let groups = load_groups(&data_dir)
        .await
        .context("cannot load subscription groups store")?;
    let providers = load_providers(&data_dir)
        .await
        .context("cannot load provider store")?;
    let client = reqwest::Client::builder()
        .user_agent("vpn233-subscribe-server/2")
        .timeout(UPSTREAM_TIMEOUT)
        .build()
        .context("cannot build upstream HTTP client")?;
    Ok(AppState {
        config: Arc::new(RwLock::new(config)),
        client,
        usage: Arc::new(RwLock::new(usage)),
        providers: Arc::new(RwLock::new(providers)),
        accounts: Arc::new(RwLock::new(accounts)),
        groups: Arc::new(RwLock::new(groups)),
        login_limiter: Arc::new(LoginRateLimiter::new()),
        cors_origins: Arc::new(cors_origins),
        web_dir,
    })
}

fn parse_origins(raw: &str) -> Vec<HeaderValue> {
    raw.split(',')
        .filter_map(|origin| origin.trim().parse::<HeaderValue>().ok())
        .collect()
}

/// Data directory currently configured for the state.
pub(crate) async fn data_dir(state: &AppState) -> PathBuf {
    PathBuf::from(state.config.read().await.server.data_dir.clone())
}

pub(crate) async fn load_usage(data_dir: &Path) -> Result<UsageStore> {
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
        if record.quota_period != crate::models::QuotaPeriod::Lifetime
            && record.period_key.is_none()
        {
            record.period_key = crate::quota::quota_period_key(record, now);
        }
    }
    Ok(store)
}

pub(crate) async fn load_accounts(data_dir: &Path) -> Result<AccountStore> {
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

pub(crate) async fn load_groups(data_dir: &Path) -> Result<GroupStore> {
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

pub(crate) async fn load_providers(data_dir: &Path) -> Result<ProviderStore> {
    let path = data_dir.join("providers.json");
    if !fs::try_exists(&path).await? {
        return Ok(ProviderStore::default());
    }
    Ok(serde_json::from_slice(&fs::read(path).await?)?)
}

async fn write_json_atomic<T: serde::Serialize>(
    state: &AppState,
    file: &str,
    value: &T,
) -> Result<()> {
    let dir = data_dir(state).await;
    fs::create_dir_all(&dir).await?;
    let tmp = dir.join(format!("{file}.tmp"));
    fs::write(&tmp, serde_json::to_vec_pretty(value)?).await?;
    // Runtime data contains password hashes and subscription secrets: keep it
    // private to the service account on Unix deployments.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600)).await?;
    }
    fs::rename(tmp, dir.join(file)).await?;
    Ok(())
}

pub(crate) async fn save_usage(state: &AppState) -> Result<()> {
    let value = state.usage.read().await;
    write_json_atomic(state, "usage.json", &*value).await
}

pub(crate) async fn save_providers(state: &AppState) -> Result<()> {
    let value = state.providers.read().await;
    write_json_atomic(state, "providers.json", &*value).await
}

pub(crate) async fn save_accounts(state: &AppState) -> Result<()> {
    let value = state.accounts.read().await;
    write_json_atomic(state, "accounts.json", &*value).await
}

pub(crate) async fn save_groups(state: &AppState) -> Result<()> {
    let value = state.groups.read().await;
    write_json_atomic(state, "groups.json", &*value).await
}

/// Periodic background task: revoke expired tokens and roll quota periods.
pub(crate) fn spawn_usage_reaper(state: AppState) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(60));
        loop {
            interval.tick().await;
            let now = unix_now();
            let changed = {
                let mut usage = state.usage.write().await;
                let mut changed = expire_tokens(&mut usage, now);
                for record in usage.tokens.values_mut() {
                    changed |= refresh_quota(record, now);
                }
                changed
            };
            if changed {
                if let Err(error) = save_usage(&state).await {
                    warn!(%error, "usage reaper persistence failed");
                }
            }
        }
    });
}
