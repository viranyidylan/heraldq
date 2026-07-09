use graphile_worker::Job;
use sqlx::PgPool;

use crate::SendWebhook;

pub async fn insert(
    pool: &PgPool,
    webhook: SendWebhook,
    job: &Job,
    status_code: Option<i16>,
) -> Result<(), sqlx::Error> {
    let id = webhook.id;
    let tenant_id = webhook.tenant_id;
    let destination = webhook.destination;
    let payload = serde_json::json!(webhook.payload);
    let enqueued_at = job.created_at();
    let attempts = job.attempts();

    sqlx::query!(
        r#"INSERT INTO webhook_deliveries (
            id, tenant_id, destination, payload, enqueued_at, status_code, attempts
            )
        VALUES ($1, $2, $3, $4, $5, $6, $7)"#,
        id,
        tenant_id,
        destination,
        payload,
        enqueued_at,
        status_code,
        attempts,
    )
    .execute(pool)
    .await?;

    Ok(())
}
