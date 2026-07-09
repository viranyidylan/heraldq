use graphile_worker::Job;
use sqlx::PgPool;
use uuid::Uuid;

use crate::{SendWebhook, WebhookRequest};

pub async fn insert(
    pool: &PgPool,
    webhook: &SendWebhook,
    job: &Job,
    failure_reason: Option<String>,
    status_code: Option<i16>,
) -> Result<(), sqlx::Error> {
    let id = webhook.id.to_owned();
    let tenant_id = webhook.tenant_id;
    let destination = webhook.destination.to_owned();
    let payload = serde_json::json!(webhook.payload);
    let enqueued_at = job.created_at();
    let attempts = job.attempts();

    sqlx::query!(
        r#"INSERT INTO dead_letter_queue (
            id, tenant_id, destination, payload,
            enqueued_at, failure_reason, status_code, attempts
            )
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8)"#,
        id,
        tenant_id,
        destination,
        payload,
        enqueued_at,
        failure_reason,
        status_code,
        attempts
    )
    .execute(pool)
    .await?;

    Ok(())
}

pub async fn count_dlq_total(pool: &PgPool, tenant_id: Uuid) -> Result<i64, sqlx::Error> {
    let count = sqlx::query_scalar!(
        r#"SELECT COUNT(*) "count!" FROM dead_letter_queue WHERE tenant_id = $1"#,
        tenant_id,
    )
    .fetch_one(pool)
    .await?;

    Ok(count)
}

pub async fn list_page(
    pool: &PgPool,
    tenant_id: &Uuid,
    limit: i64,
    offset: i64,
) -> Result<Vec<WebhookRequest>, sqlx::Error> {
    let messages = sqlx::query_as(
        r#"SELECT destination, payload, failure_reason, status_code 
        FROM dead_letter_queue WHERE tenant_id = $1 
        ORDER BY abandoned_at DESC, id DESC LIMIT $2 OFFSET $3"#,
    )
    .bind(tenant_id)
    .bind(limit)
    .bind(offset)
    .fetch_all(pool)
    .await?;

    Ok(messages)
}
