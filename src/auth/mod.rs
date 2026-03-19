use axum::extract::{FromRef, FromRequestParts};
use sha2::{Digest, Sha256};
use sqlx::PgPool;

use crate::error::AppError;

pub struct ValidApiKey {
    api_key_id: String,
    tenant_id: String,
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

        // extract the header
        let auth_header = parts
            .headers
            .get(http::header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .ok_or(AppError::Unauthorized)?;

        // query the db
        let auth_token = auth_header
            .strip_prefix("Bearer ")
            .ok_or(AppError::Unauthorized)?;

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
                tenant_id: row.tenant_id.to_string(),
            }),
            None => Err(AppError::Unauthorized),
        }
    }
}
