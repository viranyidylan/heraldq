//! Regression tests for at-least-once job re-runs.
//!
//! Graphile can re-run a job whose terminal row was already written (worker
//! crash between the insert and the job-completion write). The terminal
//! inserts must converge on the first-written row instead of erroring on the
//! primary key — a PK error here would fail the re-run, which retries and
//! fails again: a poison loop.
//!
//! These tests need a live Postgres (each `#[sqlx::test]` creates a throwaway
//! database and runs ./migrations), so they are ignored in CI where
//! SQLX_OFFLINE=true and no database exists. Run locally with:
//!
//! ```sh
//! export $(grep DATABASE_URL .env) && cargo test -- --ignored
//! ```

use graphile_worker::Job;
use sliq::{SendWebhook, db};
use sqlx::PgPool;
use ulid::Ulid;
use uuid::Uuid;

async fn seed_tenant(pool: &PgPool) -> Uuid {
    sqlx::query_scalar!(
        r#"INSERT INTO tenants (name, tier, email, hmac_secret)
        VALUES ('idem-test', 'free', 'idem@test.invalid', 'not-a-real-secret')
        RETURNING id"#,
    )
    .fetch_one(pool)
    .await
    .expect("tenant seed should insert")
}

fn webhook(tenant_id: Uuid) -> SendWebhook {
    SendWebhook {
        id: Ulid::new().to_string(),
        tenant_id,
        destination: "https://example.com/hook".into(),
        payload: serde_json::json!({"event": "test"}),
    }
}

#[sqlx::test]
#[ignore = "needs live Postgres; CI runs SQLX_OFFLINE with no database"]
async fn delivery_rerun_converges_on_first_write(pool: PgPool) {
    let tenant_id = seed_tenant(&pool).await;
    let wh = webhook(tenant_id);
    let job = Job::builder().attempts(1).max_attempts(15).build();

    db::deliveries::insert(&pool, wh.clone(), &job, Some(200))
        .await
        .expect("first write succeeds");
    db::deliveries::insert(&pool, wh, &job, Some(200))
        .await
        .expect("re-run after a crash window must not error");

    let total = db::deliveries::count_total(&pool, tenant_id).await.unwrap();
    assert_eq!(total, 1, "exactly one delivery row despite the re-run");
}

#[sqlx::test]
#[ignore = "needs live Postgres; CI runs SQLX_OFFLINE with no database"]
async fn dlq_rerun_converges_on_first_write(pool: PgPool) {
    let tenant_id = seed_tenant(&pool).await;
    let wh = webhook(tenant_id);
    let job = Job::builder().attempts(15).max_attempts(15).build();

    db::dlq::insert(&pool, &wh, &job, Some("410 Gone".into()), Some(410))
        .await
        .expect("first write succeeds");
    db::dlq::insert(&pool, &wh, &job, Some("410 Gone".into()), Some(410))
        .await
        .expect("re-run after a crash window must not error");

    let total = db::dlq::count_dlq_total(&pool, tenant_id).await.unwrap();
    assert_eq!(total, 1, "exactly one DLQ row despite the re-run");
}
