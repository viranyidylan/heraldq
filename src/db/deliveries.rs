use chrono::{DateTime, Utc};
use graphile_worker::Job;
use sqlx::PgPool;
use uuid::Uuid;

use crate::SendWebhook;

pub struct DeliveryRecord {
    pub id: String,
    pub destination: String,
    pub status_code: Option<i16>,
    pub attempts: Option<i16>,
    pub enqueued_at: DateTime<Utc>,
    pub delivered_at: DateTime<Utc>,
}

pub async fn find_by_id(
    pool: &PgPool,
    tenant_id: Uuid,
    webhook_id: &str,
) -> Result<Option<DeliveryRecord>, sqlx::Error> {
    sqlx::query_as!(
        DeliveryRecord,
        r#"SELECT id, destination, status_code, attempts, enqueued_at, delivered_at
        FROM webhook_deliveries WHERE tenant_id = $1 AND id = $2"#,
        tenant_id,
        webhook_id,
    )
    .fetch_optional(pool)
    .await
}

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
