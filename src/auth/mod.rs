use axum::extract::{FromRef, FromRequestParts};
use rand::{Rng, RngExt};
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use tracing::warn;

use crate::error::AppError;

pub fn generate_hmac_secret() -> String {
    let mut data = [0u8; 64];
    rand::rng().fill_bytes(&mut data);
    hex::encode(data)
}

/// Generate a prefixed API key: `sliq_` + 32 random hex chars (16 bytes).
pub fn generate_api_key() -> String {
    let bytes: [u8; 16] = rand::rng().random();
    format!("sliq_{}", hex::encode(bytes))
}

/// SHA-256 hash an API key, returning the hex-encoded digest.
pub fn hash_api_key(raw_key: &str) -> String {
    hex::encode(Sha256::digest(raw_key.as_bytes()))
}

fn extract_bearer_token(headers: &http::HeaderMap) -> Result<String, AppError> {
    let auth_header = headers
        .get(http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .ok_or(AppError::Unauthorized)?;

    auth_header
        .strip_prefix("Bearer ")
        .map(|t| t.to_owned())
        .ok_or(AppError::Unauthorized)
}

#[allow(dead_code)]
pub struct ValidApiKey {
    api_key_id: String,
    pub tenant_id: uuid::Uuid,
}

impl<S> FromRequestParts<S> for ValidApiKey
where
    PgPool: FromRef<S>,
    S: Send + Sync,
{
    type Rejection = AppError;

    async fn from_request_parts(
        parts: &mut http::request::Parts,
        state: &S,
    ) -> Result<Self, Self::Rejection> {
        let pg_pool = PgPool::from_ref(state);

        let auth_token = extract_bearer_token(&parts.headers)?;

        let hash = hex::encode(Sha256::digest(auth_token.as_bytes()));
        let row = sqlx::query!(
            "SELECT id, tenant_id, expires_at FROM api_keys WHERE key_hash = $1 AND is_active = true",
            hash
        )
        .fetch_optional(&pg_pool)
        .await?
        .ok_or(AppError::Unauthorized)?;

        if let Some(expires_at) = row.expires_at
            && expires_at < chrono::Utc::now()
        {
            warn!(api_key_id = %row.id, "Expired API key used");
            return Err(AppError::Unauthorized);
        }

        Ok(ValidApiKey {
            api_key_id: row.id.to_string(),
            tenant_id: row.tenant_id,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_auth_header_is_unauthorized() {
        let headers = http::HeaderMap::new();
        assert!(matches!(
            extract_bearer_token(&headers),
            Err(AppError::Unauthorized)
        ));
    }

    #[test]
    fn non_bearer_scheme_is_unauthorized() {
        let mut headers = http::HeaderMap::new();
        headers.insert(
            http::header::AUTHORIZATION,
            http::HeaderValue::from_static("Basic dXNlcjpwYXNz"),
        );
        assert!(matches!(
            extract_bearer_token(&headers),
            Err(AppError::Unauthorized)
        ));
    }

    #[test]
    fn hash_api_key_is_deterministic_and_correct_length() {
        let hash = hash_api_key("sliq_test123");
        assert_eq!(hash.len(), 64); // SHA-256 = 32 bytes = 64 hex chars
        assert_eq!(hash, hash_api_key("sliq_test123"));
    }

    #[test]
    fn bearer_token_is_extracted() {
        let mut headers = http::HeaderMap::new();
        headers.insert(
            http::header::AUTHORIZATION,
            http::HeaderValue::from_static("Bearer my-api-key-123"),
        );
        assert_eq!(extract_bearer_token(&headers).unwrap(), "my-api-key-123");
    }
}
