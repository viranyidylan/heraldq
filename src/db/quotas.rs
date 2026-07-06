use sqlx::PgPool;
use uuid::Uuid;

/// Atomically count one webhook against this tenant's current-month quota.
///
/// Returns `Ok(true)` if the request is within quota (the count was
/// incremented), `Ok(false)` if the monthly limit is already reached.
///
/// The check and increment happen in a single statement: the `WHERE` on the
/// `DO UPDATE` makes the increment conditional, so when the count is already at
/// `monthly_limit` the update never fires and zero rows come back. No
/// read-then-write race, no decrement-on-reject.
pub async fn increment_monthly_quota(
    pool: &PgPool,
    tenant_id: Uuid,
    monthly_limit: i64,
) -> Result<bool, sqlx::Error> {
    let row = sqlx::query!(
        r#"
        INSERT INTO tenant_monthly_quotas (tenant_id, month_start, webhook_count)
        VALUES ($1, DATE_TRUNC('month', NOW())::date, 1)
        ON CONFLICT (tenant_id, month_start)
        DO UPDATE SET webhook_count = tenant_monthly_quotas.webhook_count + 1
        WHERE tenant_monthly_quotas.webhook_count < $2
        RETURNING webhook_count
        "#,
        tenant_id,
        monthly_limit,
    )
    .fetch_optional(pool)
    .await?;

    Ok(row.is_some())
}
