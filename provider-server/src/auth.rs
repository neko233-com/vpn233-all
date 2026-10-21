//! Session handling, login rate limiting and constant-time comparisons.

use crate::config::Config;
use std::collections::{HashMap, VecDeque};
use subtle::ConstantTimeEq;

/// An authenticated admin session.
#[derive(Debug, Clone)]
pub struct Session {
    pub username: String,
    pub expires_at: u64,
}

impl Session {
    pub fn is_expired(&self, now: u64) -> bool {
        self.expires_at <= now
    }
}

/// In-memory authentication state: sessions plus per-client login failures.
#[derive(Debug, Default)]
pub struct AuthState {
    sessions: HashMap<String, Session>,
    failures: HashMap<String, VecDeque<u64>>,
}

impl AuthState {
    /// Create a session token with the configured TTL.
    pub fn create_session(&mut self, username: &str, ttl_secs: u64, now: u64) -> String {
        let token = uuid::Uuid::new_v4().to_string();
        self.sessions.insert(
            token.clone(),
            Session {
                username: username.to_owned(),
                expires_at: now.saturating_add(ttl_secs.max(1)),
            },
        );
        token
    }

    /// Return the session for `token` when it is still valid.
    pub fn validate(&mut self, token: &str, now: u64) -> Option<&Session> {
        let expired = self
            .sessions
            .get(token)
            .map(|session| session.is_expired(now))
            .unwrap_or(false);
        if expired {
            self.sessions.remove(token);
            return None;
        }
        self.sessions.get(token)
    }

    pub fn remove(&mut self, token: &str) {
        self.sessions.remove(token);
    }

    pub fn session_count(&self) -> usize {
        self.sessions.len()
    }

    /// Remove all expired sessions.
    pub fn cleanup(&mut self, now: u64) {
        self.sessions.retain(|_, session| !session.is_expired(now));
    }

    /// True when `key` already exceeded the failure budget inside the window.
    pub fn is_rate_limited(&mut self, key: &str, now: u64, limit: u64) -> bool {
        let window = self.failure_window(key, now);
        window.len() as u64 >= limit
    }

    /// Record one failed login and return true when the budget is exhausted.
    pub fn record_failure(&mut self, key: &str, now: u64, limit: u64) -> bool {
        let window = self.failure_window(key, now);
        window.push_back(now);
        window.len() as u64 >= limit
    }

    /// Number of recent failures for `key` (test/diagnostic helper).
    pub fn failure_count(&mut self, key: &str, now: u64) -> usize {
        self.failure_window(key, now).len()
    }

    fn failure_window(&mut self, key: &str, now: u64) -> &mut VecDeque<u64> {
        let window = self.failures.entry(key.to_owned()).or_default();
        prune_window(window, now);
        window
    }
}

/// Drop failure timestamps older than the fixed login window.
fn prune_window(window: &mut VecDeque<u64>, now: u64) {
    let threshold = now.saturating_sub(crate::config::LOGIN_RATE_WINDOW_SECS);
    while window.front().is_some_and(|stamp| *stamp <= threshold) {
        window.pop_front();
    }
}

/// Compare two strings without leaking their contents through timing.
pub fn constant_time_eq(provided: &str, expected: &str) -> bool {
    // Subtle returns `Choice(0)` for different lengths and compares equal-length
    // inputs with a constant-time primitive.
    provided.as_bytes().ct_eq(expected.as_bytes()).into()
}

/// Compare a username/password pair in constant time.
pub fn credentials_match(provided_user: &str, provided_password: &str, expected: &Config) -> bool {
    let user_ok = constant_time_eq(provided_user, &expected.admin_user);
    let password_ok = constant_time_eq(provided_password, &expected.admin_password);
    user_ok & password_ok
}

/// Derive a stable per-client key for login rate limiting. Reverse proxies can
/// forward the original address through `X-Forwarded-For`.
pub fn client_key(headers: &axum::http::HeaderMap) -> String {
    if let Some(value) = headers.get("x-forwarded-for").and_then(|v| v.to_str().ok()) {
        if let Some(first) = value.split(',').next() {
            let first = first.trim();
            if !first.is_empty() {
                return first.to_owned();
            }
        }
    }
    "local".to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constant_time_equality_is_boolean_and_exact() {
        assert!(constant_time_eq("secret-value", "secret-value"));
        assert!(!constant_time_eq("secret-value", "secret-valuE"));
        assert!(!constant_time_eq("short", "a-much-longer-value"));
        assert!(constant_time_eq("", ""));
    }

    #[test]
    fn sessions_expire_after_ttl() {
        let mut auth = AuthState::default();
        let token = auth.create_session("root", 10, 1_000);
        assert!(auth.validate(&token, 1_005).is_some());
        assert!(auth.validate(&token, 1_010).is_none());
        assert_eq!(auth.session_count(), 0);
    }

    #[test]
    fn cleanup_removes_expired_sessions() {
        let mut auth = AuthState::default();
        let first = auth.create_session("root", 1, 100);
        let second = auth.create_session("root", 100, 100);
        auth.cleanup(101);
        assert!(auth.validate(&first, 101).is_none());
        assert!(auth.validate(&second, 101).is_some());
    }

    #[test]
    fn rate_limit_uses_the_failure_window() {
        let mut auth = AuthState::default();
        for index in 0..9 {
            assert!(!auth.record_failure("peer", 1_000 + index, 10));
        }
        assert!(auth.record_failure("peer", 1_009, 10));
        assert!(auth.is_rate_limited("peer", 1_010, 10));
        assert!(!auth.is_rate_limited("peer", 1_100, 10));
        assert_eq!(auth.failure_count("peer", 1_100), 0);
    }
}
