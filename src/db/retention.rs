//! Retention sweeper: deletes terminal rows older than each tenant's
//! configured retention window (tenants.delivered_retention_days /
//! dlq_retention_days, defaulted in migration 0009).
//!
//! Deletes run in fixed-size batches so a tenant with months of backlog
//! never holds a long row lock or bloats one giant transaction; the sweep
//! just loops until a batch comes back short. Each delete joins tenants at
//! execution time, so changing a tenant's retention takes effect on the
//! next sweep with no backfill step.

use sqlx::PgPool;
use std::time::Duration;
use tracing::{info, warn};

/// Rows deleted per statement. Small enough to keep each delete's lock
/// footprint and WAL burst modest, big enough that a nightly backlog
/// clears in a few iterations.
const BATCH_SIZE: i64 = 1_000;

/// One sweep per hour: retention is measured in days, so anything faster
/// buys nothing, and the off-cycle cost is one cheap index probe per table.
const SWEEP_INTERVAL: Duration = Duration::from_secs(60 * 60);

/// Delete one batch of delivered rows past their tenant's retention.
/// Returns the number of rows deleted; a short batch means the table is
/// clean and the sweep can stop.
pub async fn sweep_deliveries_batch(pool: &PgPool) -> Result<u64, sqlx::Error> {
    let result = sqlx::query!(
        r#"DELETE FROM webhook_deliveries
        WHERE id IN (
            SELECT wd.id FROM webhook_deliveries wd
            JOIN tenants t ON t.id = wd.tenant_id
            WHERE wd.delivered_at < now() - make_interval(days => t.delivered_retention_days)
            LIMIT $1
        )"#,
        BATCH_SIZE,
    )
    .execute(pool)
    .await?;
    Ok(result.rows_affected())
}

/// Delete one batch of dead-lettered rows past their tenant's retention.
pub async fn sweep_dlq_batch(pool: &PgPool) -> Result<u64, sqlx::Error> {
    let result = sqlx::query!(
        r#"DELETE FROM dead_letter_queue
        WHERE id IN (
            SELECT d.id FROM dead_letter_queue d
            JOIN tenants t ON t.id = d.tenant_id
            WHERE d.abandoned_at < now() - make_interval(days => t.dlq_retention_days)
            LIMIT $1
        )"#,
        BATCH_SIZE,
    )
    .execute(pool)
    .await?;
    Ok(result.rows_affected())
}

/// Run one full sweep of both tables, batching until each is clean.
/// Returns (delivered_rows, dlq_rows) deleted, for the log line and tests.
pub async fn sweep_once(pool: &PgPool) -> Result<(u64, u64), sqlx::Error> {
    let mut delivered = 0u64;
    loop {
        let n = sweep_deliveries_batch(pool).await?;
        delivered += n;
        if n < BATCH_SIZE as u64 {
            break;
        }
    }

    let mut dlq = 0u64;
    loop {
        let n = sweep_dlq_batch(pool).await?;
        dlq += n;
        if n < BATCH_SIZE as u64 {
            break;
        }
    }

    Ok((delivered, dlq))
}

/// Spawn the hourly sweeper. A failed sweep logs and waits for the next
/// tick rather than exiting — same posture as the backpressure poller: a
/// silently dead background task is worse than a noisy skipped cycle.
pub fn spawn_sweeper(pool: PgPool) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(SWEEP_INTERVAL);
        loop {
            tick.tick().await;
            match sweep_once(&pool).await {
                Ok((delivered, dlq)) => {
                    metrics::counter!("heraldq_retention_deleted_total", "table" => "webhook_deliveries")
                        .increment(delivered);
                    metrics::counter!("heraldq_retention_deleted_total", "table" => "dead_letter_queue")
                        .increment(dlq);
                    if delivered > 0 || dlq > 0 {
                        info!(delivered, dlq, "retention sweep deleted expired rows");
                    }
                }
                Err(e) => warn!(error = ?e, "retention sweep failed, skipping cycle"),
            }
        }
    });
}
