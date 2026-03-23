use graphile_worker::Job;
use sqlx::{PgPool};

use crate::SendWebhook;

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