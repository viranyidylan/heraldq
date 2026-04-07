use graphile_worker::Job;
use sqlx::PgPool;

use crate::{SendWebhook, WebhookRequest};

pub async fn insert(pool: &PgPool, webhook: SendWebhook, job: Job) -> Result<(), sqlx::Error> {
    let id = webhook.id;
    let tenant_id = webhook.tenant_id;
    let destination = webhook.destination;
    let payload = serde_json::json!(webhook.payload);
    let enqueued_at = job.created_at();
    
    sqlx::query!(
        r#"INSERT INTO dead_letter_queue (id, tenant_id, destination, payload, enqueued_at) 
        VALUES ($1, $2, $3, $4, $5)"#,
        id,
        tenant_id,
        destination,
        payload,
        enqueued_at
    )
    .execute(pool)
    .await?;

    Ok(())
}

pub async fn list_page(
    pool: &PgPool,
    limit: i64,
    offset: i64,
) -> Result<Vec<WebhookRequest>, sqlx::Error> {
    let messages = sqlx::query_as(
        "SELECT destination, payload FROM dead_letter_queue ORDER BY abandoned_at DESC, id DESC LIMIT $1 OFFSET $2",
    )
    .bind(limit)
    .bind(offset)
    .fetch_all(pool)
    .await?;

    Ok(messages)
}