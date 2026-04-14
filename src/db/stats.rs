use sqlx::{PgPool, Postgres};
use uuid::Uuid;

pub async fn count_jobs_queued_today(pool: &PgPool, tenant_id: &Uuid) -> Result<i64, sqlx::Error> {
    let queue_name = format!("webhooks:{tenant_id}");
    let count = sqlx::query_scalar::<Postgres, Option<i64>>(
        "SELECT COUNT(*) FROM graphile_worker.jobs WHERE created_at >= CURRENT_DATE AND queue_name = $1",
    )
    .bind(queue_name)
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

// But line 40 in webhook_activity_summary still has the unfiltered queued count. That one also needs the queue_name filter.
// The tricky bit: query_as! uses $1 for tenant_id already. For the jobs subquery you need to bind the queue_name string. 
// You've got a couple approaches — how do you want to handle having two different bind types (Uuid for the tenant queries, String for the queue_name) in one query_as! macro call?
// Hint: you can use $2 with a second bind parameter, or you can do the string concat in SQL with 'webhooks:' || $1::text.

pub async fn webhook_activity_summary(pool: &PgPool, tenant_id: &Uuid) -> Result<ActivityCounts, sqlx::Error> {
    let activity_counts = sqlx::query_as!(
        ActivityCounts,
        "SELECT \
            COALESCE( (SELECT COUNT(*) FROM graphile_worker.jobs WHERE created_at >= CURRENT_DATE), 0) AS \"queued!: i64\" , \
            COALESCE( (SELECT COUNT(*) FROM webhook_deliveries WHERE delivered_at >= CURRENT_DATE AND tenant_id = $1), 0) AS \"delivered_today!: i64\", \
            COALESCE( (SELECT COUNT(*) FROM dead_letter_queue WHERE abandoned_at >= CURRENT_DATE AND tenant_id = $1), 0) AS \"failed_today!: i64\", \
            COALESCE( (SELECT COUNT(*) FROM dead_letter_queue WHERE tenant_id = $1), 0) AS \"dlq_total!: i64\"",
            tenant_id
    )
    .fetch_one(pool)
    .await?;

    Ok(activity_counts)
}
