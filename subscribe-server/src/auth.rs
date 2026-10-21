//! Authentication: JWT issuance, constant-time credential checks, password
//! hashing, client IP extraction, and login failure rate limiting.

use std::collections::HashMap;
use std::convert::Infallible;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Mutex;

use anyhow::Result;
use argon2::{
    password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
    Argon2,
};
use axum::{
    extract::{ConnectInfo, FromRequestParts},
    http::{header, request::Parts, HeaderMap},
};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use jsonwebtoken::{decode, encode, Algorithm, DecodingKey, EncodingKey, Header, Validation};
use rand_core::OsRng;
use serde::{Deserialize, Serialize};

use crate::config::ServerConfig;
use crate::models::UsageRecord;
use crate::state::AppState;
use crate::util::constant_time_eq_str;

pub(crate) const ADMIN_JWT_TTL_SECS: u64 = 365 * 24 * 60 * 60;
pub(crate) const SUBSCRIPTION_JWT_TTL_SECS: u64 = 10 * 365 * 24 * 60 * 60;

/// Maximum failed logins allowed inside [`LOGIN_FAILURE_WINDOW_SECS`].
pub(crate) const LOGIN_MAX_FAILURES: u32 = 10;
/// Sliding window length for failed login accounting, in seconds.
pub(crate) const LOGIN_FAILURE_WINDOW_SECS: u64 = 60;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub(crate) struct AdminClaims {
    pub(crate) sub: String,
    pub(crate) iat: u64,
    pub(crate) exp: u64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub(crate) struct SubscriptionClaims {
    pub(crate) sub: String,
    pub(crate) token: String,
    pub(crate) iat: u64,
    pub(crate) exp: u64,
}

fn admin_jwt_secret(config: &ServerConfig) -> Vec<u8> {
    format!(
        "vpn233-admin-jwt-v1:{}:{}",
        config.admin_user, config.admin_password
    )
    .into_bytes()
}

pub(crate) fn issue_admin_jwt(
    config: &ServerConfig,
    now: u64,
) -> Result<String, jsonwebtoken::errors::Error> {
    crate::jwt_crypto::install();
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

pub(crate) fn verify_admin_jwt(config: &ServerConfig, token: &str) -> bool {
    crate::jwt_crypto::install();
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

pub(crate) fn issue_subscription_jwt(
    config: &ServerConfig,
    token: &str,
    created_at: u64,
) -> Result<String, jsonwebtoken::errors::Error> {
    crate::jwt_crypto::install();
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

pub(crate) fn verify_subscription_jwt(
    config: &ServerConfig,
    token: &str,
    access_token: &str,
) -> bool {
    crate::jwt_crypto::install();
    let validation = Validation::new(Algorithm::HS256);
    decode::<SubscriptionClaims>(
        access_token,
        &DecodingKey::from_secret(&subscription_jwt_secret(config)),
        &validation,
    )
    .is_ok_and(|data| data.claims.sub == "vpn233-subscription" && data.claims.token == token)
}

/// Compares admin credentials in constant time (both fields always compared).
pub(crate) fn verify_admin_credentials(
    config: &ServerConfig,
    username: &str,
    password: &str,
) -> bool {
    let user_ok = constant_time_eq_str(username, &config.admin_user);
    let password_ok = constant_time_eq_str(password, &config.admin_password);
    user_ok & password_ok
}

/// Authorization check shared by every admin API handler.
pub(crate) async fn admin_authorized(state: &AppState, headers: &HeaderMap) -> bool {
    let config = state.config.read().await;
    let Some(value) = headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
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
    let Some((username, password)) = credentials.split_once(':') else {
        return false;
    };
    verify_admin_credentials(&config.server, username, password)
}

/// Hashes a subscription password with Argon2, returning `(salt, hash)`.
pub(crate) fn hash_subscription_password(password: &str) -> Result<(String, String)> {
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

/// Verifies a subscription password against its stored Argon2 hash.
pub(crate) fn verify_password(password: &str, expected_hash: &str) -> bool {
    PasswordHash::new(expected_hash).ok().is_some_and(|hash| {
        Argon2::default()
            .verify_password(password.as_bytes(), &hash)
            .is_ok()
    })
}

/// Checks the token/password pair of a subscription record.
pub(crate) fn verify_record_password(record: &UsageRecord, password: &str) -> bool {
    verify_password(password, &record.password_hash)
}

/// Argon2 hash for provider heartbeat tokens (never stored in clear text).
pub(crate) fn hash_heartbeat_token(token: &str) -> Result<String> {
    let salt = SaltString::generate(&mut OsRng);
    Ok(Argon2::default()
        .hash_password(token.as_bytes(), &salt)
        .map_err(|error| anyhow::anyhow!("heartbeat token hashing failed: {error:?}"))?
        .to_string())
}

pub(crate) fn verify_heartbeat_token(token: &str, hash: &str) -> bool {
    verify_password(token, hash)
}

#[derive(Debug, Clone, Copy)]
struct FailureWindow {
    window_start: u64,
    failures: u32,
}

/// In-memory failed-login tracker keyed by client IP.
///
/// Kept intentionally small and non-persistent: a restart clears the counters,
/// which is acceptable for a login throttle.
#[derive(Debug, Default)]
pub(crate) struct LoginRateLimiter {
    failures: Mutex<HashMap<IpAddr, FailureWindow>>,
}

impl LoginRateLimiter {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<IpAddr, FailureWindow>> {
        self.failures
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Returns the remaining lockout time in seconds when the IP is throttled.
    pub(crate) fn retry_after(&self, ip: IpAddr, now: u64) -> Option<u64> {
        let mut failures = self.lock();
        let window = failures.get(&ip)?;
        let expires_at = window
            .window_start
            .saturating_add(LOGIN_FAILURE_WINDOW_SECS);
        if now >= expires_at {
            failures.remove(&ip);
            return None;
        }
        (window.failures >= LOGIN_MAX_FAILURES).then(|| expires_at - now)
    }

    /// Records one failed login inside the current window.
    pub(crate) fn record_failure(&self, ip: IpAddr, now: u64) {
        let mut failures = self.lock();
        let window = failures.entry(ip).or_insert(FailureWindow {
            window_start: now,
            failures: 0,
        });
        if now
            >= window
                .window_start
                .saturating_add(LOGIN_FAILURE_WINDOW_SECS)
        {
            window.window_start = now;
            window.failures = 0;
        }
        window.failures = window.failures.saturating_add(1);
    }

    /// Clears the failure counter after a successful login.
    pub(crate) fn record_success(&self, ip: IpAddr) {
        self.lock().remove(&ip);
    }
}

/// Client IP used for login throttling.
///
/// Reads axum's [`ConnectInfo`] (populated by `run`) when present so forwarded
/// headers from untrusted clients are ignored in production. Falls back to
/// proxy headers only for embedded/test servers that have no connect info.
#[derive(Debug, Clone, Copy)]
pub(crate) struct ClientIp(pub(crate) IpAddr);

impl<S> FromRequestParts<S> for ClientIp
where
    S: Send + Sync,
{
    type Rejection = Infallible;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        if let Some(connect_info) = parts.extensions.get::<ConnectInfo<SocketAddr>>() {
            return Ok(Self(connect_info.0.ip()));
        }
        for name in ["x-forwarded-for", "x-real-ip"] {
            let Some(value) = parts
                .headers
                .get(name)
                .and_then(|value| value.to_str().ok())
            else {
                continue;
            };
            let candidate = value.split(',').next().unwrap_or_default().trim();
            if let Ok(ip) = candidate.parse::<IpAddr>() {
                return Ok(Self(ip));
            }
        }
        Ok(Self(IpAddr::V4(Ipv4Addr::UNSPECIFIED)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_server_config() -> ServerConfig {
        ServerConfig {
            admin_user: "root".into(),
            admin_password: "secret".into(),
            ..ServerConfig::default()
        }
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
    fn admin_credentials_are_compared_exactly() {
        let config = test_server_config();
        assert!(verify_admin_credentials(&config, "root", "secret"));
        assert!(!verify_admin_credentials(&config, "root", "Secret"));
        assert!(!verify_admin_credentials(&config, "admin", "secret"));
    }

    #[test]
    fn login_limiter_blocks_after_threshold() {
        let limiter = LoginRateLimiter::new();
        let ip: IpAddr = "203.0.113.9".parse().unwrap();
        let now = 1_700_000_000;
        for _ in 0..LOGIN_MAX_FAILURES {
            assert!(limiter.retry_after(ip, now).is_none());
            limiter.record_failure(ip, now);
        }
        assert!(limiter.retry_after(ip, now).is_some());
        assert!(limiter
            .retry_after(ip, now + LOGIN_FAILURE_WINDOW_SECS)
            .is_none());
        limiter.record_failure(ip, now + LOGIN_FAILURE_WINDOW_SECS);
        assert!(limiter
            .retry_after(ip, now + LOGIN_FAILURE_WINDOW_SECS)
            .is_none());
        limiter.record_success(ip);
        assert!(limiter.retry_after(ip, now).is_none());
    }

    #[test]
    fn password_hashes_verify_and_reject() {
        let (salt, hash) = hash_subscription_password("hunter2").unwrap();
        assert!(!salt.is_empty());
        assert!(verify_password("hunter2", &hash));
        assert!(!verify_password("hunter3", &hash));
    }
}
