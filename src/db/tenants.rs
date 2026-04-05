use chrono::{DateTime, Utc};
use sqlx::PgPool;

pub async fn create_tenant(pool: &PgPool, name: &str) -> Result<uuid::Uuid, sqlx::Error> {
    let row = sqlx::query!(
        "INSERT INTO tenants (name, tier) VALUES ($1, 'free') RETURNING id",
        name,
    )
    .fetch_one(pool)
    .await?;

    Ok(row.id)
}

pub async fn create_api_key(
    pool: &PgPool,
    tenant_id: uuid::Uuid,
    key_hash: &str,
    name: &str,
    expires_at: DateTime<Utc>,
) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "INSERT INTO api_keys (tenant_id, key_hash, name, expires_at) VALUES ($1, $2, $3, $4)",
        tenant_id,
        key_hash,
        name,
        expires_at,
    )
    .execute(pool)
    .await?;

    Ok(())
}
