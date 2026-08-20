//! Paddle webhook signature verification.
//!
//! Paddle signs the raw request body with the destination's secret and sends
//! the result in the `Paddle-Signature` header:
//!
//! ```text
//! Paddle-Signature: ts=1695877337;h1=v1:c10a6d5c9e91a2b1f0d3...
//! ```
//!
//! The signed payload is `"{ts}:{raw_body}"` (the timestamp from the header,
//! a colon, then the exact bytes Paddle sent), HMAC-SHA256, hex-encoded, and
//! prefixed with `v1:`. Verification recomputes the MAC over the bytes we
//! actually received — the body must never be parsed/re-serialized first.

use chrono::{DateTime, Utc};
use hmac::{Hmac, Mac};
use sha2::Sha256;

/// Signatures older than this are rejected. Paddle retries carry a fresh
/// timestamp, so the window only needs to absorb clock skew between Paddle
/// and us, not delivery delay.
const MAX_SKEW: chrono::Duration = chrono::Duration::minutes(10);

type HmacSha256 = Hmac<Sha256>;

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum SignatureError {
    #[error("malformed Paddle-Signature header")]
    MalformedHeader,
    #[error("signature timestamp missing or not a number")]
    BadTimestamp,
    #[error("signature timestamp outside allowed window")]
    StaleTimestamp,
    #[error("signature does not match body")]
    Mismatch,
}

/// Verify the `Paddle-Signature` header against the raw request body.
///
/// `header` is the value of the `Paddle-Signature` header, `raw_body` the
/// unparsed request bytes, `secret` the webhook secret of the notification
/// destination Paddle delivered against.
pub fn verify_signature(
    header: &str,
    raw_body: &[u8],
    secret: &str,
    now: DateTime<Utc>,
) -> Result<(), SignatureError> {
    let (ts, h1) = parse_header(header)?;

    let signed_at = DateTime::from_timestamp(ts, 0).ok_or(SignatureError::BadTimestamp)?;
    if (now - signed_at).abs() > MAX_SKEW {
        return Err(SignatureError::StaleTimestamp);
    }

    // HMAC-SHA256 over "{ts}:{body}", hex-encoded, "v1:"-prefixed.
    let mut mac =
        HmacSha256::new_from_slice(secret.as_bytes()).expect("HMAC accepts any key length");
    mac.update(ts.to_string().as_bytes());
    mac.update(b":");
    mac.update(raw_body);

    let provided = h1
        .strip_prefix("v1:")
        .ok_or(SignatureError::MalformedHeader)?;
    let provided = hex::decode(provided).map_err(|_| SignatureError::MalformedHeader)?;
    // Constant-time comparison — verify_slice short-circuits nothing.
    mac.verify_slice(&provided)
        .map_err(|_| SignatureError::Mismatch)
}

/// Extract `(ts, h1)` from `ts=...;h1=...`. Key order is not guaranteed, and
/// unknown keys (future signature versions) are ignored rather than rejected.
fn parse_header(header: &str) -> Result<(i64, String), SignatureError> {
    let mut ts: Option<i64> = None;
    let mut h1: Option<String> = None;

    for pair in header.split(';') {
        let Some((key, value)) = pair.split_once('=') else {
            continue;
        };
        match key.trim() {
            "ts" => ts = value.trim().parse::<i64>().ok(),
            "h1" => h1 = Some(value.trim().to_owned()),
            _ => {}
        }
    }

    match (ts, h1) {
        (Some(ts), Some(h1)) if !h1.is_empty() => Ok((ts, h1)),
        _ => Err(SignatureError::MalformedHeader),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    const SECRET: &str = "pdl_ntfset_test";
    const BODY: &str = r#"{"event_type":"subscription.created","event_id":"evt_1"}"#;

    fn now() -> DateTime<Utc> {
        Utc.timestamp_opt(1_700_000_000, 0).unwrap()
    }

    /// Build a header exactly like Paddle does for the given body and secret.
    fn sign(ts: i64, body: &str, secret: &str) -> String {
        let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).unwrap();
        mac.update(format!("{ts}:").as_bytes());
        mac.update(body.as_bytes());
        format!("ts={ts};h1=v1:{}", hex::encode(mac.finalize().into_bytes()))
    }

    #[test]
    fn valid_signature_passes() {
        let header = sign(1_700_000_000, BODY, SECRET);
        assert_eq!(
            verify_signature(&header, BODY.as_bytes(), SECRET, now()),
            Ok(())
        );
    }

    #[test]
    fn tampered_body_fails() {
        let header = sign(1_700_000_000, BODY, SECRET);
        let tampered = br#"{"event_type":"subscription.created","event_id":"evt_2"}"#;
        assert_eq!(
            verify_signature(&header, tampered, SECRET, now()),
            Err(SignatureError::Mismatch)
        );
    }

    #[test]
    fn wrong_secret_fails() {
        let header = sign(1_700_000_000, BODY, "other-secret");
        assert_eq!(
            verify_signature(&header, BODY.as_bytes(), SECRET, now()),
            Err(SignatureError::Mismatch)
        );
    }

    #[test]
    fn stale_timestamp_fails() {
        let stale = now() - chrono::Duration::hours(1);
        let header = sign(stale.timestamp(), BODY, SECRET);
        assert_eq!(
            verify_signature(&header, BODY.as_bytes(), SECRET, now()),
            Err(SignatureError::StaleTimestamp)
        );
    }

    #[test]
    fn timestamp_skew_inside_window_passes() {
        let header = sign(now().timestamp() - 9 * 60, BODY, SECRET);
        assert_eq!(
            verify_signature(&header, BODY.as_bytes(), SECRET, now()),
            Ok(())
        );
    }

    #[test]
    fn missing_signature_parts_fail() {
        for header in ["", "ts=1700000000", "h1=v1:abcd", "garbage", "ts=;h1="] {
            assert_eq!(
                verify_signature(header, BODY.as_bytes(), SECRET, now()),
                Err(SignatureError::MalformedHeader),
                "header {header:?} should be malformed"
            );
        }
    }

    #[test]
    fn non_hex_digest_is_malformed() {
        let header = "ts=1700000000;h1=v1:zz-not-hex";
        assert_eq!(
            verify_signature(header, BODY.as_bytes(), SECRET, now()),
            Err(SignatureError::MalformedHeader)
        );
    }

    #[test]
    fn missing_v1_prefix_is_malformed() {
        let header = sign(1_700_000_000, BODY, SECRET);
        let no_prefix = header.replace("h1=v1:", "h1=");
        assert_eq!(
            verify_signature(&no_prefix, BODY.as_bytes(), SECRET, now()),
            Err(SignatureError::MalformedHeader)
        );
    }

    #[test]
    fn unix_epoch_boundary_is_handled() {
        // now - epoch skew far exceeds the window; must be StaleTimestamp,
        // not a panic or overflow.
        let header = sign(0, BODY, SECRET);
        assert_eq!(
            verify_signature(&header, BODY.as_bytes(), SECRET, now()),
            Err(SignatureError::StaleTimestamp)
        );
    }
}
