use chrono::{DateTime, Utc};

use crate::error::AppError;

// 3e — smoke test: valid code → 201; invalid code → 400; reused code → 400

pub async fn redeem_invite_code(
    // conn: &Transaction<'_, Postgres>,
    conn: &mut sqlx::PgConnection,
    invite_code: &str,
    tenant_id: uuid::Uuid,
) -> Result<String, AppError> {
    // update beta_codes
    // set redeemed_at, redeemed_by_tenant_id
    // where the code = sign_up code
    // and redeemed_at IS NULL and expires_at
    // returning CODE
    let result = sqlx::query!(
        r#"UPDATE beta_codes
            SET redeemed_at = NOW(),
                redeemed_by_tenant_id = $2
            WHERE code = $1
                AND redeemed_at IS NULL
                AND (expires_at IS NULL OR expires_at > NOW())
            RETURNING code"#,
        invite_code,
        tenant_id,
    )
    .fetch_optional(&mut *conn)
    .await?;

    if let Some(row) = result {
        return Ok(row.code);
    }

    // UPDATE matched zero rows — disambiguate: is the code missing/expired, or already redeemed?
    let existing = sqlx::query!(
        r#"SELECT redeemed_at FROM beta_codes WHERE code = $1"#,
        invite_code,
    )
    .fetch_optional(&mut *conn)
    .await?;

    match existing {
        Some(row) if row.redeemed_at.is_some() => Err(AppError::InviteCodeAlreadyRedeemed),
        _ => Err(AppError::InvalidInviteCode),
    }
}

pub async fn create_tenant(
    conn: &mut sqlx::PgConnection,
    name: &str,
    email: &str,
    hmac_secret: &str,
) -> Result<uuid::Uuid, AppError> {
    // Retention is set explicitly for the free tier rather than left to the
    // column defaults (30/90 days from migration 0009, which don't match the
    // free tier's promised 1/7). Tier changes rewrite both columns.
    let (delivered_days, dlq_days) = crate::tiers::retention_days("free");
    let row = sqlx::query!(
        r#"INSERT INTO tenants (name, tier, email, hmac_secret,
                delivered_retention_days, dlq_retention_days)
            VALUES ($1, 'free', $2, $3, $4, $5) RETURNING id"#,
        name,
        email,
        hmac_secret,
        delivered_days,
        dlq_days,
    )
    .fetch_one(conn)
    .await
    .map_err(|e| match &e {
        sqlx::Error::Database(db_err) if db_err.code().as_deref() == Some("23505") => {
            AppError::EmailAlreadyRegistered
        }
        _ => AppError::from(e),
    })?;

    Ok(row.id)
}

pub async fn get_hmac_secret(
    pool: &sqlx::PgPool,
    tenant_id: uuid::Uuid,
) -> Result<String, sqlx::Error> {
    // Consider caching the tenant's hmac secret depending on volume
    sqlx::query_scalar!(
        r#"SELECT hmac_secret FROM tenants WHERE id = $1"#,
        tenant_id,
    )
    .fetch_one(pool)
    .await
}

/// Look up a tenant by the lost-key recovery pair: the account email plus
/// the HMAC secret shown alongside the API key at signup. The secret is the
/// second factor only the tenant and heraldQ hold, so possession of it is
/// proof-of-account without any email round trip.
///
/// Returns `None` for any mismatch — callers must respond identically for
/// unknown emails and wrong secrets so recovery can't be used to probe
/// which addresses have accounts.
pub async fn find_tenant_for_recovery(
    pool: &sqlx::PgPool,
    email: &str,
    hmac_secret: &str,
) -> Result<Option<uuid::Uuid>, sqlx::Error> {
    let row = sqlx::query_scalar!(
        r#"SELECT id FROM tenants WHERE email = $1 AND hmac_secret = $2"#,
        email,
        hmac_secret,
    )
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

/// Deactivate every active key for a tenant. Rotation and recovery both
/// revoke first, then issue exactly one replacement in the same
/// transaction — there is never a moment with two live keys (rotation) or
/// with the lost key still valid (recovery).
pub async fn revoke_all_keys(
    conn: &mut sqlx::PgConnection,
    tenant_id: uuid::Uuid,
) -> Result<u64, sqlx::Error> {
    let result = sqlx::query!(
        "UPDATE api_keys SET is_active = false WHERE tenant_id = $1 AND is_active = true",
        tenant_id,
    )
    .execute(conn)
    .await?;
    Ok(result.rows_affected())
}

pub struct ApiKeyInfo {
    pub id: uuid::Uuid,
    pub name: String,
    pub created_at: chrono::DateTime<Utc>,
    pub expires_at: Option<chrono::DateTime<Utc>>,
    pub is_active: bool,
}

/// Key metadata for the authenticated tenant. Never returns hashes — there
/// is nothing a customer can do with them and no reason to move them.
pub async fn list_keys(
    pool: &sqlx::PgPool,
    tenant_id: uuid::Uuid,
) -> Result<Vec<ApiKeyInfo>, sqlx::Error> {
    let rows = sqlx::query!(
        r#"SELECT id, name, created_at, expires_at, is_active
            FROM api_keys
            WHERE tenant_id = $1
            ORDER BY created_at DESC"#,
        tenant_id,
    )
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|r| ApiKeyInfo {
            id: r.id,
            name: r.name.unwrap_or_else(|| "unnamed".into()),
            created_at: r.created_at,
            expires_at: r.expires_at,
            is_active: r.is_active,
        })
        .collect())
}

/// `expires_at` is `None` for keys that live as long as the account does.
pub async fn create_api_key(
    conn: &mut sqlx::PgConnection,
    tenant_id: uuid::Uuid,
    key_hash: &str,
    name: &str,
    expires_at: Option<DateTime<Utc>>,
) -> Result<(), sqlx::Error> {
    sqlx::query!(
        r#"INSERT INTO api_keys (tenant_id, key_hash, name, expires_at) 
            VALUES ($1, $2, $3, $4)"#,
        tenant_id,
        key_hash,
        name,
        expires_at,
    )
    .execute(conn)
    .await?;

    Ok(())
}
