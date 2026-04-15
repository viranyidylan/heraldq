use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use uuid::Uuid;

use crate::{db::dlq::count_dlq_total, queue::TenantQueue};

pub async fn count_jobs_queued_today(
    pool: &PgPool,
    tenant_queue: &TenantQueue,
) -> Result<i64, sqlx::Error> {
    let count = sqlx::query_scalar!(
        r#"SELECT COUNT(*) as "count!" FROM graphile_worker.jobs WHERE created_at >= CURRENT_DATE AND queue_name = $1"#,
        tenant_queue.as_str(),
    )
    .fetch_one(pool)
    .await?;

    Ok(count)
}

pub async fn count_delivered_today(pool: &PgPool, tenant_id: Uuid) -> Result<i64, sqlx::Error> {
    let count = sqlx::query_scalar!(
        r#"SELECT COUNT(*) "count!" FROM webhook_deliveries WHERE delivered_at >= CURRENT_DATE AND tenant_id = $1"#,
        tenant_id,
    )
    .fetch_one(pool)
    .await?;

    Ok(count)
}

pub async fn count_failed_today(pool: &PgPool, tenant_id: Uuid) -> Result<i64, sqlx::Error> {
    let count = sqlx::query_scalar!(
        r#"SELECT COUNT(*) "count!" FROM dead_letter_queue WHERE abandoned_at >= CURRENT_DATE AND tenant_id = $1"#,
        tenant_id,
    )
    .fetch_one(pool)
    .await?;

    Ok(count)
}

#[derive(Deserialize, Serialize)]
pub struct WebhookActivitySummary {
    pub received_today: i64,
    pub queued: i64,
    pub delivered_today: i64,
    pub failed_today: i64,
    pub dlq_total: i64,
}

pub async fn activity_summary(
    pool: &PgPool,
    tenant_id: Uuid,
    tenant_queue: &TenantQueue,
) -> Result<WebhookActivitySummary, sqlx::Error> {
    let (queued, delivered_today, failed_today, dlq_total) = tokio::try_join!(
        count_jobs_queued_today(pool, tenant_queue),
        count_delivered_today(pool, tenant_id),
        count_failed_today(pool, tenant_id),
        count_dlq_total(pool, tenant_id),
    )?;

    Ok(WebhookActivitySummary {
        received_today: delivered_today + failed_today + queued,
        queued,
        delivered_today,
        failed_today,
        dlq_total,
    })
}
