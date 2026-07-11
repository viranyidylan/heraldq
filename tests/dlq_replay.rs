//! Tests for the DLQ replay claim: first caller wins, double replay is
//! refused, and a failed enqueue releases the claim for a retry.
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
        VALUES ('replay-test', 'free', 'replay@test.invalid', 'not-a-real-secret')
        RETURNING id"#,
    )
    .fetch_one(pool)
    .await
    .expect("tenant seed should insert")
}

async fn seed_dlq(pool: &PgPool, tenant_id: Uuid) -> String {
    let id = Ulid::new().to_string();
    sqlx::query!(
        r#"INSERT INTO dead_letter_queue
            (id, tenant_id, destination, payload, enqueued_at, failure_reason)
        VALUES ($1, $2, 'https://example.com/hook', '{"event":"test"}'::jsonb,
            now(), 'Retries exhausted')"#,
        id,
        tenant_id,
    )
    .execute(pool)
    .await
    .expect("dlq seed should insert");
    id
}

#[sqlx::test]
#[ignore = "needs live Postgres; CI runs SQLX_OFFLINE with no database"]
async fn claim_returns_source_and_stamps_row(pool: PgPool) {
    let tenant_id = seed_tenant(&pool).await;
    let dlq_id = seed_dlq(&pool, tenant_id).await;

    let source = db::dlq::claim_for_replay(&pool, tenant_id, &dlq_id, "new-ulid")
        .await
        .unwrap()
        .expect("unreplayed row should be claimable");

    assert_eq!(source.destination, "https://example.com/hook");
    assert_eq!(source.payload["event"], "test");

    let entry = db::dlq::find_by_id(&pool, tenant_id, &dlq_id)
        .await
        .unwrap()
        .expect("row is kept after replay");
    assert_eq!(entry.replayed_as.as_deref(), Some("new-ulid"));
    assert!(entry.replayed_at.is_some());
}

#[sqlx::test]
#[ignore = "needs live Postgres; CI runs SQLX_OFFLINE with no database"]
async fn second_claim_loses(pool: PgPool) {
    let tenant_id = seed_tenant(&pool).await;
    let dlq_id = seed_dlq(&pool, tenant_id).await;

    let first = db::dlq::claim_for_replay(&pool, tenant_id, &dlq_id, "first")
        .await
        .unwrap();
    let second = db::dlq::claim_for_replay(&pool, tenant_id, &dlq_id, "second")
        .await
        .unwrap();

    assert!(first.is_some());
    assert!(
        second.is_none(),
        "a claimed row must not be claimable again"
    );

    let entry = db::dlq::find_by_id(&pool, tenant_id, &dlq_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        entry.replayed_as.as_deref(),
        Some("first"),
        "the losing claim must not overwrite the winner"
    );
}

#[sqlx::test]
#[ignore = "needs live Postgres; CI runs SQLX_OFFLINE with no database"]
async fn other_tenants_rows_are_not_claimable(pool: PgPool) {
    let owner = seed_tenant(&pool).await;
    let dlq_id = seed_dlq(&pool, owner).await;

    let attacker = sqlx::query_scalar!(
        r#"INSERT INTO tenants (name, tier, email, hmac_secret)
        VALUES ('other', 'free', 'other@test.invalid', 'not-a-real-secret')
        RETURNING id"#,
    )
    .fetch_one(&pool)
    .await
    .unwrap();

    let claim = db::dlq::claim_for_replay(&pool, attacker, &dlq_id, "stolen")
        .await
        .unwrap();
    assert!(claim.is_none(), "tenant isolation on replay");
}

#[sqlx::test]
#[ignore = "needs live Postgres; CI runs SQLX_OFFLINE with no database"]
async fn unclaim_releases_the_row(pool: PgPool) {
    let tenant_id = seed_tenant(&pool).await;
    let dlq_id = seed_dlq(&pool, tenant_id).await;

    db::dlq::claim_for_replay(&pool, tenant_id, &dlq_id, "failed-attempt")
        .await
        .unwrap()
        .expect("claim");
    db::dlq::unclaim_replay(&pool, tenant_id, &dlq_id, "failed-attempt")
        .await
        .unwrap();

    let reclaim = db::dlq::claim_for_replay(&pool, tenant_id, &dlq_id, "retry")
        .await
        .unwrap();
    assert!(
        reclaim.is_some(),
        "an unclaimed row must be claimable again after a failed enqueue"
    );
}
