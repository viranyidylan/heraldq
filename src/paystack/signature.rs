//! Paystack webhook signature verification.
//!
//! Paystack signs the raw request body with HMAC-SHA512, keyed by the
//! account's secret key, and sends the hex digest in the `x-paystack-signature`
//! header. Unlike Paddle's scheme there is no timestamp, so there is nothing
//! to stale-check — replay protection is entirely a matter of idempotent
//! handling on our side (see `sync::record_event`).

use hmac::{Hmac, Mac};
use sha2::Sha512;

type HmacSha512 = Hmac<Sha512>;

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum SignatureError {
    #[error("malformed x-paystack-signature header")]
    Malformed,
    #[error("signature does not match body")]
    Mismatch,
}

/// Verify the `x-paystack-signature` header against the raw request body.
///
/// `header` is the hex HMAC-SHA512 digest (128 chars), `raw_body` the
/// unparsed request bytes, `secret_key` the account secret key (`sk_...`).
pub fn verify_signature(
    header: &str,
    raw_body: &[u8],
    secret_key: &str,
) -> Result<(), SignatureError> {
    let header = header.trim();
    let provided = hex::decode(header).map_err(|_| SignatureError::Malformed)?;
    if provided.len() != 64 {
        // SHA-512 digests are 64 bytes; a different length can never match.
        return Err(SignatureError::Malformed);
    }

    let mut mac =
        HmacSha512::new_from_slice(secret_key.as_bytes()).expect("HMAC accepts any key length");
    mac.update(raw_body);

    // Constant-time comparison — verify_slice short-circuits nothing.
    mac.verify_slice(&provided)
        .map_err(|_| SignatureError::Mismatch)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECRET: &str = "sk_test_example";
    const BODY: &str = r#"{"event":"charge.success","data":{"reference":"ref_1"}}"#;

    /// Build a header exactly like Paystack does for the given body and key.
    fn sign(body: &str, key: &str) -> String {
        let mut mac = HmacSha512::new_from_slice(key.as_bytes()).unwrap();
        mac.update(body.as_bytes());
        hex::encode(mac.finalize().into_bytes())
    }

    #[test]
    fn valid_signature_passes() {
        let header = sign(BODY, SECRET);
        assert_eq!(verify_signature(&header, BODY.as_bytes(), SECRET), Ok(()));
    }

    #[test]
    fn tampered_body_fails() {
        let header = sign(BODY, SECRET);
        let tampered = br#"{"event":"charge.success","data":{"reference":"ref_2"}}"#;
        assert_eq!(
            verify_signature(&header, tampered, SECRET),
            Err(SignatureError::Mismatch)
        );
    }

    #[test]
    fn wrong_key_fails() {
        let header = sign(BODY, "sk_test_other");
        assert_eq!(
            verify_signature(&header, BODY.as_bytes(), SECRET),
            Err(SignatureError::Mismatch)
        );
    }

    #[test]
    fn non_hex_header_is_malformed() {
        assert_eq!(
            verify_signature("zz-not-hex", BODY.as_bytes(), SECRET),
            Err(SignatureError::Malformed)
        );
    }

    #[test]
    fn wrong_length_header_is_malformed() {
        let header = sign(BODY, SECRET);
        let truncated = &header[..header.len() - 2];
        assert_eq!(
            verify_signature(truncated, BODY.as_bytes(), SECRET),
            Err(SignatureError::Malformed)
        );
        let padded = format!("{header}00");
        assert_eq!(
            verify_signature(&padded, BODY.as_bytes(), SECRET),
            Err(SignatureError::Malformed)
        );
    }

    #[test]
    fn empty_header_is_malformed() {
        assert_eq!(
            verify_signature("", BODY.as_bytes(), SECRET),
            Err(SignatureError::Malformed)
        );
    }
}
