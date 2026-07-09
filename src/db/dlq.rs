use chrono::{DateTime, Utc};
use graphile_worker::Job;
use serde::Serialize;
use serde_json::Value;
use sqlx::PgPool;
use uuid::Uuid;

use crate::SendWebhook;

/// A dead-lettered webhook as shown to the tenant: everything they need to
/// diagnose the failure and decide what to do about it.
#[derive(Debug, Serialize)]
pub struct DlqEntry {
    pub id: String,
    pub destination: String,
    pub payload: Value,
    pub failure_reason: Option<String>,
    pub status_code: Option<i16>,
    pub attempts: Option<i16>,
    pub enqueued_at: DateTime<Utc>,
    pub abandoned_at: DateTime<Utc>,
}

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
) -> Result<Vec<DlqEntry>, sqlx::Error> {
    let messages = sqlx::query_as!(
        DlqEntry,
        r#"SELECT id, destination, payload, failure_reason, status_code, attempts,
            enqueued_at, abandoned_at
        FROM dead_letter_queue WHERE tenant_id = $1
        ORDER BY abandoned_at DESC, id DESC LIMIT $2 OFFSET $3"#,
        tenant_id,
        limit,
        offset,
    )
    .fetch_all(pool)
    .await?;

    Ok(messages)
}
