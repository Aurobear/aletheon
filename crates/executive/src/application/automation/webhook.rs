//! Webhook event receiver and HMAC verification.

use hmac::{Hmac, Mac};
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

/// An incoming webhook event.
#[derive(Debug, Clone)]
pub struct WebhookEvent {
    /// The event type identifier (e.g. "push", "deploy", "alert").
    pub event_type: String,
    /// Raw JSON payload.
    pub payload: serde_json::Value,
    /// Optional HMAC-SHA256 signature header value.
    pub signature: Option<String>,
}

/// Verify an HMAC-SHA256 signature against a raw payload and shared secret.
///
/// The signature is expected in `hex` format.  Returns `true` when valid.
pub fn verify_hmac(payload: &[u8], secret: &str, signature: &str) -> bool {
    let Ok(mut mac) = HmacSha256::new_from_slice(secret.as_bytes()) else {
        return false;
    };
    mac.update(payload);
    let expected = hex_encode(&mac.finalize().into_bytes());
    // Constant-time comparison via hmac crate
    let Ok(mut mac2) = HmacSha256::new_from_slice(secret.as_bytes()) else {
        return false;
    };
    mac2.update(payload);
    mac2.verify_slice(signature.as_bytes()).is_ok()
        || constant_time_eq(expected.as_bytes(), signature.as_bytes())
}

/// Match an incoming event against a list of subscribed event types.
///
/// A subscription entry of `"*"` matches every event.
pub fn matches_event_type(event_type: &str, subscriptions: &[String]) -> bool {
    subscriptions.iter().any(|s| s == "*" || s == event_type)
}

// -- helpers ------------------------------------------------------------------

fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

// -- Tests --------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hmac_verify_valid() {
        let payload = b"hello world";
        let secret = "s3cret";
        // Pre-compute expected HMAC-SHA256 hex digest
        let expected = compute_hmac_hex(payload, secret);
        assert!(verify_hmac(payload, secret, &expected));
    }

    #[test]
    fn hmac_verify_invalid() {
        let payload = b"hello world";
        let secret = "s3cret";
        assert!(!verify_hmac(payload, secret, "deadbeef"));
    }

    #[test]
    fn hmac_verify_wrong_secret() {
        let payload = b"hello world";
        let secret = "s3cret";
        let sig = compute_hmac_hex(payload, secret);
        assert!(!verify_hmac(payload, "wrong", &sig));
    }

    #[test]
    fn event_type_matching() {
        let subs = vec!["push".to_string(), "deploy".to_string()];
        assert!(matches_event_type("push", &subs));
        assert!(matches_event_type("deploy", &subs));
        assert!(!matches_event_type("delete", &subs));
    }

    #[test]
    fn event_type_wildcard() {
        let subs = vec!["*".to_string()];
        assert!(matches_event_type("anything", &subs));
    }

    /// Helper to compute HMAC hex for tests.
    fn compute_hmac_hex(payload: &[u8], secret: &str) -> String {
        let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).unwrap();
        mac.update(payload);
        hex_encode(&mac.finalize().into_bytes())
    }
}
