use axum::extract::{FromRef, FromRequestParts};
use sha2::{Digest, Sha256};
use sqlx::PgPool;

use crate::error::AppError;

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

        // if auth_header is in the db
        let hash = hex::encode(Sha256::digest(auth_token.as_bytes()));
        let res = sqlx::query!(
            "SELECT id, tenant_id FROM api_keys WHERE key_hash = $1",
            hash
        )
        .fetch_optional(&pg_pool)
        .await?;

        // return the result
        match res {
            Some(row) => Ok(ValidApiKey {
                api_key_id: row.id.to_string(),
                tenant_id: row.tenant_id,
            }),
            None => Err(AppError::Unauthorized),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_auth_header_is_unauthorized() {
        let headers = http::HeaderMap::new();
        assert!(matches!(extract_bearer_token(&headers), Err(AppError::Unauthorized)));
    }

    #[test]
    fn non_bearer_scheme_is_unauthorized() {
        let mut headers = http::HeaderMap::new();
        headers.insert(
            http::header::AUTHORIZATION,
            http::HeaderValue::from_static("Basic dXNlcjpwYXNz"),
        );
        assert!(matches!(extract_bearer_token(&headers), Err(AppError::Unauthorized)));
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
