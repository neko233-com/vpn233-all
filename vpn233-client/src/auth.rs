//! Constant-time credential checks and Basic authentication parsing.

use axum::http::{header, HeaderMap};
use base64::Engine;
use subtle::ConstantTimeEq;

/// Compare two strings without leaking their contents through timing.
pub fn constant_time_eq(provided: &str, expected: &str) -> bool {
    provided.as_bytes().ct_eq(expected.as_bytes()).into()
}

/// Compare a username/password pair in constant time.
pub fn credentials_match(
    provided_user: &str,
    provided_password: &str,
    expected_user: &str,
    expected_password: &str,
) -> bool {
    let user_ok = constant_time_eq(provided_user, expected_user);
    let password_ok = constant_time_eq(provided_password, expected_password);
    user_ok & password_ok
}

/// Decode `Basic <base64(user:password)>` into its credentials.
pub fn decode_basic(value: &str) -> Option<(String, String)> {
    let encoded = value
        .strip_prefix("Basic ")
        .or_else(|| value.strip_prefix("basic "))?;
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(encoded.trim())
        .ok()?;
    let text = String::from_utf8(decoded).ok()?;
    let (user, password) = text.split_once(':')?;
    Some((user.to_owned(), password.to_owned()))
}

/// Check the `Authorization: Basic ...` header against the expected credentials.
pub fn basic_auth_matches(headers: &HeaderMap, user: &str, password: &str) -> bool {
    let Some(value) = headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
    else {
        return false;
    };
    let Some((provided_user, provided_password)) = decode_basic(value) else {
        return false;
    };
    credentials_match(&provided_user, &provided_password, user, password)
}

/// Check a raw `Proxy-Authorization` header value against the expected credentials.
pub fn proxy_authorization_matches(value: Option<&str>, user: &str, password: &str) -> bool {
    let Some(value) = value else {
        return false;
    };
    let Some((provided_user, provided_password)) = decode_basic(value) else {
        return false;
    };
    credentials_match(&provided_user, &provided_password, user, password)
}

/// Encode `user:password` as a Basic authentication header value (tests/tools).
pub fn encode_basic(user: &str, password: &str) -> String {
    format!(
        "Basic {}",
        base64::engine::general_purpose::STANDARD.encode(format!("{user}:{password}"))
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basic_credentials_roundtrip() {
        let encoded = encode_basic("alice", "s3cret");
        assert_eq!(
            decode_basic(&encoded),
            Some(("alice".to_owned(), "s3cret".to_owned()))
        );
        assert_eq!(decode_basic("Bearer token"), None);
        assert_eq!(decode_basic("Basic not-base64!!"), None);
        assert_eq!(decode_basic("Basic"), None);
    }

    #[test]
    fn constant_time_check_rejects_wrong_values() {
        assert!(credentials_match("alice", "s3cret", "alice", "s3cret"));
        assert!(!credentials_match("alice", "wrong", "alice", "s3cret"));
        assert!(!credentials_match("bob", "s3cret", "alice", "s3cret"));
        assert!(!constant_time_eq("abc", "abcd"));
    }

    #[test]
    fn proxy_authorization_header_is_checked() {
        let header = encode_basic("alice", "s3cret");
        assert!(proxy_authorization_matches(
            Some(&header),
            "alice",
            "s3cret"
        ));
        assert!(!proxy_authorization_matches(
            Some(&header),
            "alice",
            "other"
        ));
        assert!(!proxy_authorization_matches(None, "alice", "s3cret"));
    }
}
