use sqlx::{PgPool, Postgres};

pub async fn count_jobs_queued_today(pool: &PgPool) -> Result<i64, sqlx::Error> {
    let count = sqlx::query_scalar::<Postgres, Option<i64>>(
        "SELECT COUNT(*) FROM graphile_worker.jobs WHERE created_at >= CURRENT_DATE",
    )
    .fetch_one(pool)
    .await?
    .unwrap_or(0);

    Ok(count)
}

pub async fn count_dlq_total(pool: &PgPool) -> Result<i64, sqlx::Error> {
    let count = sqlx::query_scalar::<Postgres, Option<i64>>(
        "SELECT COUNT(*) FROM dead_letter_queue",
    )
    .fetch_one(pool)
    .await?
    .unwrap_or(0);

    Ok(count)
}

pub struct ActivityCounts {
    pub queued: i64,
    pub delivered_today: i64,
    pub failed_today: i64,
    pub dlq_total: i64,
}

pub async fn webhook_activity_summary(pool: &PgPool) -> Result<ActivityCounts, sqlx::Error> {
    let row = sqlx::query_as::<Postgres, (Option<i64>, Option<i64>, Option<i64>, Option<i64>)>(
        "SELECT \
            (SELECT COUNT(*) FROM graphile_worker.jobs WHERE created_at >= CURRENT_DATE), \
            (SELECT COUNT(*) FROM webhook_deliveries WHERE delivered_at >= CURRENT_DATE), \
            (SELECT COUNT(*) FROM dead_letter_queue WHERE abandoned_at >= CURRENT_DATE), \
            (SELECT COUNT(*) FROM dead_letter_queue)"
    )
    .fetch_one(pool)
    .await?;

    Ok(ActivityCounts {
        queued: row.0.unwrap_or(0),
        delivered_today: row.1.unwrap_or(0),
        failed_today: row.2.unwrap_or(0),
        dlq_total: row.3.unwrap_or(0),
    })
}
