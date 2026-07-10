//! Tests for the retention sweeper: expired terminal rows are deleted,
//! in-window rows survive, and per-tenant overrides beat the defaults.
//!
//! These need a live Postgres (each `#[sqlx::test]` creates a throwaway
//! database and runs ./migrations), so they are ignored in CI where
//! SQLX_OFFLINE=true and no database exists. Run locally with:
//!
//! ```sh
//! export $(grep DATABASE_URL .env) && cargo test -- --ignored
//! ```

use sliq::db;
use sqlx::PgPool;
use ulid::Ulid;
use uuid::Uuid;

async fn seed_tenant(pool: &PgPool) -> Uuid {
    sqlx::query_scalar!(
        r#"INSERT INTO tenants (name, tier, email, hmac_secret)
        VALUES ('retention-test', 'free', 'retention@test.invalid', 'not-a-real-secret')
        RETURNING id"#,
    )
    .fetch_one(pool)
    .await
    .expect("tenant seed should insert")
}

/// Insert a delivery row with delivered_at backdated by `age_days`.
async fn seed_delivery(pool: &PgPool, tenant_id: Uuid, age_days: i32) -> String {
    let id = Ulid::new().to_string();
    sqlx::query!(
        r#"INSERT INTO webhook_deliveries
            (id, tenant_id, destination, payload, enqueued_at, delivered_at)
        VALUES ($1, $2, 'https://example.com/hook', '{}'::jsonb,
            now() - make_interval(days => $3),
            now() - make_interval(days => $3))"#,
        id,
        tenant_id,
        age_days,
    )
    .execute(pool)
    .await
    .expect("delivery seed should insert");
    id
}

/// Insert a DLQ row with abandoned_at backdated by `age_days`.
async fn seed_dlq(pool: &PgPool, tenant_id: Uuid, age_days: i32) -> String {
    let id = Ulid::new().to_string();
    sqlx::query!(
        r#"INSERT INTO dead_letter_queue
            (id, tenant_id, destination, payload, enqueued_at, abandoned_at)
        VALUES ($1, $2, 'https://example.com/hook', '{}'::jsonb,
            now() - make_interval(days => $3),
            now() - make_interval(days => $3))"#,
        id,
        tenant_id,
        age_days,
    )
    .execute(pool)
    .await
    .expect("dlq seed should insert");
    id
}

async fn delivery_ids(pool: &PgPool, tenant_id: Uuid) -> Vec<String> {
    sqlx::query_scalar!(
        "SELECT id FROM webhook_deliveries WHERE tenant_id = $1",
        tenant_id
    )
    .fetch_all(pool)
    .await
    .unwrap()
}

async fn dlq_ids(pool: &PgPool, tenant_id: Uuid) -> Vec<String> {
    sqlx::query_scalar!(
        "SELECT id FROM dead_letter_queue WHERE tenant_id = $1",
        tenant_id
    )
    .fetch_all(pool)
    .await
    .unwrap()
}

#[sqlx::test]
#[ignore = "needs live Postgres; CI runs SQLX_OFFLINE with no database"]
async fn sweep_deletes_expired_and_keeps_fresh(pool: PgPool) {
    let tenant_id = seed_tenant(&pool).await;

    // Defaults: delivered kept 30d, DLQ kept 90d.
    let old_delivery = seed_delivery(&pool, tenant_id, 31).await;
    let fresh_delivery = seed_delivery(&pool, tenant_id, 29).await;
    let old_dlq = seed_dlq(&pool, tenant_id, 91).await;
    let fresh_dlq = seed_dlq(&pool, tenant_id, 89).await;

    let (deleted_deliveries, deleted_dlq) = db::retention::sweep_once(&pool)
        .await
        .expect("sweep should run");

    assert_eq!(deleted_deliveries, 1, "exactly the expired delivery");
    assert_eq!(deleted_dlq, 1, "exactly the expired DLQ row");

    let remaining = delivery_ids(&pool, tenant_id).await;
    assert!(remaining.contains(&fresh_delivery));
    assert!(!remaining.contains(&old_delivery));

    let remaining = dlq_ids(&pool, tenant_id).await;
    assert!(remaining.contains(&fresh_dlq));
    assert!(!remaining.contains(&old_dlq));
}

#[sqlx::test]
#[ignore = "needs live Postgres; CI runs SQLX_OFFLINE with no database"]
async fn per_tenant_override_beats_default(pool: PgPool) {
    let tenant_id = seed_tenant(&pool).await;
    sqlx::query!(
        "UPDATE tenants SET delivered_retention_days = 7 WHERE id = $1",
        tenant_id
    )
    .execute(&pool)
    .await
    .unwrap();

    // 10 days old: inside the 30d default, outside this tenant's 7d window.
    seed_delivery(&pool, tenant_id, 10).await;

    let (deleted, _) = db::retention::sweep_once(&pool)
        .await
        .expect("sweep should run");

    assert_eq!(deleted, 1, "tenant override should expire the 10-day row");
    assert!(delivery_ids(&pool, tenant_id).await.is_empty());
}

#[sqlx::test]
#[ignore = "needs live Postgres; CI runs SQLX_OFFLINE with no database"]
async fn sweep_on_clean_tables_deletes_nothing(pool: PgPool) {
    let tenant_id = seed_tenant(&pool).await;
    seed_delivery(&pool, tenant_id, 1).await;
    seed_dlq(&pool, tenant_id, 1).await;

    let (deliveries, dlq) = db::retention::sweep_once(&pool)
        .await
        .expect("sweep should run");

    assert_eq!((deliveries, dlq), (0, 0));
}
