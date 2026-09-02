//! Tests for tier-based enforcement: monthly quota caps, retention wiring
//! on signup and tier change, and the tier-bucket rate limiter selection.
//!
//! Needs a live Postgres (`#[sqlx::test]`); run locally with:
//!
//! ```sh
//! export $(grep DATABASE_URL .env) && cargo test -- --ignored
//! ```

use chrono::Utc;
use hmac::Mac;
use serde_json::json;
use sliq::paddle::{events::PaddleEvent, signature::verify_signature};
use sqlx::PgPool;

const SECRET: &str = "pdl_ntfset_test";

fn signed_subscription_event(event_id: &str, customer_id: &str, tier: &str) -> (String, Vec<u8>) {
    let body = json!({
        "event_type": "subscription.created",
        "event_id": event_id,
        "occurred_at": "2026-08-27T12:00:00Z",
        "data": {
            "id": format!("sub_{event_id}"),
            "status": "active",
            "customer_id": customer_id,
            "scheduled_change": null,
            "current_billing_period": {
                "starts_at": "2026-08-27T00:00:00Z",
                "ends_at": "2026-09-27T00:00:00Z",
            },
            "canceled_at": null,
            "next_billed_at": "2026-09-27T00:00:00Z",
            "items": [{
                "status": "active",
                "quantity": 1,
                "price": {
                    "id": "pri_01h",
                    "product_id": "pro_01h",
                    "custom_data": { "tier": tier },
                }
            }]
        }
    })
    .to_string();

    let ts = Utc::now().timestamp();
    let mut mac = hmac::Hmac::<sha2::Sha256>::new_from_slice(SECRET.as_bytes()).unwrap();
    mac.update(format!("{ts}:").as_bytes());
    mac.update(body.as_bytes());
    let digest = hex::encode(mac.finalize().into_bytes());

    (format!("ts={ts};h1=v1:{digest}"), body.into_bytes())
}

async fn deliver(pool: &PgPool, header: &str, body: &[u8]) {
    verify_signature(header, body, SECRET, Utc::now()).expect("signed");
    let event: PaddleEvent = serde_json::from_slice(body).expect("parses");
    let mut tx = pool.begin().await.unwrap();
    let is_new = sliq::paddle::sync::record_event(
        &mut tx,
        &event.event_id,
        &event.event_type,
        event.occurred_at,
    )
    .await
    .unwrap();
    if is_new {
        sliq::paddle::process_event(&mut tx, &event, None)
            .await
            .unwrap();
    }
    tx.commit().await.unwrap();
}

async fn signup_tenant(pool: &PgPool, email: &str) -> uuid::Uuid {
    let mut tx = pool.begin().await.unwrap();
    let id = sliq::db::tenants::create_tenant(&mut tx, "tier-test", email, "not-a-real-secret")
        .await
        .unwrap();
    tx.commit().await.unwrap();
    id
}

/// The boundary the pricing page promises: a free tenant's 5,000th webhook
/// of the month is accepted; the 5,001st is refused. Paid tiers get 10x.
#[sqlx::test]
#[ignore = "needs live Postgres; CI runs SQLX_OFFLINE with no database"]
async fn monthly_quota_enforces_tier_limits(pool: PgPool) {
    let tenant_id = signup_tenant(&pool, "quota@test.invalid").await;

    // Seed the free tenant at 4,999 used this month.
    sqlx::query!(
        "INSERT INTO tenant_monthly_quotas (tenant_id, month_start, webhook_count)
         VALUES ($1, DATE_TRUNC('month', NOW())::date, 4999)",
        tenant_id,
    )
    .execute(&pool)
    .await
    .unwrap();

    let free_cap = sliq::tiers::monthly_webhook_limit("free");
    assert_eq!(free_cap, 5_000);

    // 5,000th: allowed (count goes 4999 -> 5000).
    assert!(
        sliq::db::quotas::increment_monthly_quota(&pool, tenant_id, free_cap)
            .await
            .unwrap(),
        "request at exactly the cap is the last allowed one"
    );
    // 5,001st: refused, atomically — no row update, no decrement.
    assert!(
        !sliq::db::quotas::increment_monthly_quota(&pool, tenant_id, free_cap)
            .await
            .unwrap(),
        "request past the cap is refused"
    );

    let count: i64 = sqlx::query_scalar!(
        "SELECT webhook_count FROM tenant_monthly_quotas WHERE tenant_id = $1",
        tenant_id
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(count, 5_000, "refused requests must not burn quota");
}

/// A negotiated override is the quota, regardless of tier. This is the
/// enterprise mechanism: Paddle sells tiers, a human writes the deal into
/// monthly_webhook_limit_override.
#[sqlx::test]
#[ignore = "needs live Postgres; CI runs SQLX_OFFLINE with no database"]
async fn negotiated_override_is_the_quota_not_the_tier_default(pool: PgPool) {
    let tenant_id = signup_tenant(&pool, "deal@test.invalid").await;

    // The deal: 7 webhooks/month, on a free-tier tenant (default 5,000).
    sqlx::query!(
        "UPDATE tenants SET monthly_webhook_limit_override = 7 WHERE id = $1",
        tenant_id,
    )
    .execute(&pool)
    .await
    .unwrap();

    let limit = sliq::tiers::effective_monthly_limit(
        "free",
        sqlx::query_scalar!(
            "SELECT monthly_webhook_limit_override FROM tenants WHERE id = $1",
            tenant_id,
        )
        .fetch_one(&pool)
        .await
        .unwrap(),
    );
    assert_eq!(limit, 7, "the deal is the quota");

    // Seed at 6: the 7th is the last allowed, the 8th refused.
    sqlx::query!(
        "INSERT INTO tenant_monthly_quotas (tenant_id, month_start, webhook_count)
         VALUES ($1, DATE_TRUNC('month', NOW())::date, 6)",
        tenant_id,
    )
    .execute(&pool)
    .await
    .unwrap();

    assert!(
        sliq::db::quotas::increment_monthly_quota(&pool, tenant_id, limit)
            .await
            .unwrap()
    );
    assert!(
        !sliq::db::quotas::increment_monthly_quota(&pool, tenant_id, limit)
            .await
            .unwrap()
    );
}

/// Signup writes the free tier's retention (1 day history / 7 day DLQ),
/// not the column defaults from migration 0009 (30/90).
#[sqlx::test]
#[ignore = "needs live Postgres; CI runs SQLX_OFFLINE with no database"]
async fn signup_sets_free_tier_retention_not_column_defaults(pool: PgPool) {
    let tenant_id = signup_tenant(&pool, "retention@test.invalid").await;

    let row = sqlx::query!(
        "SELECT delivered_retention_days, dlq_retention_days FROM tenants WHERE id = $1",
        tenant_id,
    )
    .fetch_one(&pool)
    .await
    .unwrap();

    assert_eq!(row.delivered_retention_days, 1);
    assert_eq!(row.dlq_retention_days, 7);
}

/// Upgrading via a Paddle subscription event rewrites retention alongside
/// the tier — and a later cancel drops both back to free values.
#[sqlx::test]
#[ignore = "needs live Postgres; CI runs SQLX_OFFLINE with no database"]
async fn tier_change_rewrites_retention_in_both_directions(pool: PgPool) {
    let email = "upgrade-retention@test.invalid";
    let tenant_id = signup_tenant(&pool, email).await;

    // customer.created links by email...
    let customer_body = json!({
        "event_type": "customer.created",
        "event_id": "evt_tier_c1",
        "occurred_at": "2026-08-27T12:00:00Z",
        "data": { "id": "ctm_tier01", "email": email },
    })
    .to_string();
    let ts = Utc::now().timestamp();
    let mut mac = hmac::Hmac::<sha2::Sha256>::new_from_slice(SECRET.as_bytes()).unwrap();
    mac.update(format!("{ts}:").as_bytes());
    mac.update(customer_body.as_bytes());
    let digest = hex::encode(mac.finalize().into_bytes());
    deliver(
        &pool,
        &format!("ts={ts};h1=v1:{digest}"),
        customer_body.as_bytes(),
    )
    .await;

    // ...then a professional subscription upgrades tier AND retention.
    let (h, b) = signed_subscription_event("evt_tier_s1", "ctm_tier01", "professional");
    deliver(&pool, &h, &b).await;

    let upgraded = sqlx::query!(
        "SELECT tier::text AS tier, delivered_retention_days, dlq_retention_days
         FROM tenants WHERE id = $1",
        tenant_id,
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(upgraded.tier.as_deref(), Some("professional"));
    assert_eq!(upgraded.delivered_retention_days, 30);
    assert_eq!(upgraded.dlq_retention_days, 90);

    // Terminal cancel drops tier and retention back to free.
    let mut canceled: serde_json::Value = serde_json::from_slice(
        &signed_subscription_event("evt_tier_s2", "ctm_tier01", "professional").1,
    )
    .unwrap();
    canceled["event_type"] = json!("subscription.canceled");
    canceled["event_id"] = json!("evt_tier_s2");
    canceled["data"]["status"] = json!("canceled");
    canceled["data"]["canceled_at"] = json!("2026-09-27T00:00:00Z");
    let body = canceled.to_string();
    let ts = Utc::now().timestamp();
    let mut mac = hmac::Hmac::<sha2::Sha256>::new_from_slice(SECRET.as_bytes()).unwrap();
    mac.update(format!("{ts}:").as_bytes());
    mac.update(body.as_bytes());
    let digest = hex::encode(mac.finalize().into_bytes());
    deliver(&pool, &format!("ts={ts};h1=v1:{digest}"), body.as_bytes()).await;

    let downgraded = sqlx::query!(
        "SELECT tier::text AS tier, delivered_retention_days, dlq_retention_days
         FROM tenants WHERE id = $1",
        tenant_id,
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(downgraded.tier.as_deref(), Some("free"));
    assert_eq!(downgraded.delivered_retention_days, 1);
    assert_eq!(downgraded.dlq_retention_days, 7);
}

/// Rate buckets: each tier gets its own limiter, unknown tiers land in the
/// free bucket, and the buckets actually enforce their rates.
#[tokio::test]
async fn rate_limit_buckets_are_per_tier_and_enforced() {
    use std::time::Duration;

    let limits = sliq::TierRateLimits::new();
    let free_key = uuid::Uuid::new_v4();

    // Free tier: 4/s burst — a rapid 5th call is throttled.
    for _ in 0..4 {
        assert!(limits.for_tier("free").check_key(&free_key).is_ok());
    }
    assert!(
        limits.for_tier("free").check_key(&free_key).is_err(),
        "free bucket must throttle past its burst"
    );

    // The same tenant id in a higher tier's bucket is a different budget —
    // proving buckets are independent.
    assert!(limits.for_tier("professional").check_key(&free_key).is_ok());

    // Unknown tier → free bucket (already exhausted above for this key).
    assert!(limits.for_tier("platinum").check_key(&free_key).is_err());

    // Tokens return over time.
    tokio::time::sleep(Duration::from_millis(350)).await;
    assert!(limits.for_tier("free").check_key(&free_key).is_ok());
}
