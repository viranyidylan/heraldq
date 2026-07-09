use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

/// A webhook still in the queue: not yet delivered, not yet dead-lettered.
pub struct PendingJob {
    pub destination: String,
    pub attempts: i16,
    pub max_attempts: i16,
    pub last_error: Option<String>,
    pub created_at: DateTime<Utc>,
    pub run_at: DateTime<Utc>,
}

/// Look up an in-flight job by its job key — enqueue stamps the webhook ULID
/// into graphile's `key` column, which carries a unique index, so this is a
/// single-row index lookup instead of a payload scan over the whole (hot)
/// jobs table. The tenant check still reads the payload, but only on the one
/// row the key already selected — a tenant mismatch means the id belongs to
/// someone else and must 404, exactly like a missing row.
///
/// Reads graphile_worker._private_jobs because the public jobs view omits
/// payload — coupled to graphile internals; re-verify on upgrades (same
/// caveat as db::stats::count_jobs_queued_today).
pub async fn find_by_webhook_id(
    pool: &PgPool,
    tenant_id: Uuid,
    webhook_id: &str,
) -> Result<Option<PendingJob>, sqlx::Error> {
    sqlx::query_as!(
        PendingJob,
        r#"SELECT payload->>'destination' AS "destination!",
            attempts, max_attempts, last_error, created_at, run_at
        FROM graphile_worker._private_jobs
        WHERE key = $1 AND payload->>'tenant_id' = $2"#,
        webhook_id,
        tenant_id.to_string(),
    )
    .fetch_optional(pool)
    .await
}
