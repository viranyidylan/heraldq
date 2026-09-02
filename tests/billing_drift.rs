//! Tests for the billing-drift detector: the queries behind the
//! `heraldq_paddle_*` gauges must count exactly the three failure modes
//! they exist to catch, and nothing healthy.
//!
//! ```sh
//! export $(grep DATABASE_URL .env) && cargo test -- --ignored
//! ```

use sqlx::PgPool;

use sliq::telemetry::{BillingDrift, billing_drift_snapshot};

async fn seed_tenant(pool: &PgPool, email: &str, tier: &str) -> uuid::Uuid {
    sqlx::query_scalar!(
        r#"INSERT INTO tenants (name, tier, email, hmac_secret)
            VALUES ('drift-test', $2::text::account_tier, $1, 'not-a-real-secret')
            RETURNING id"#,
        email,
        tier,
    )
    .fetch_one(pool)
    .await
    .unwrap()
}

async fn seed_customer(pool: &PgPool, id: &str, email: &str, tenant_id: Option<uuid::Uuid>) {
    sqlx::query!(
        "INSERT INTO paddle_customers (paddle_customer_id, email, tenant_id)
         VALUES ($1, $2, $3)",
        id,
        email,
        tenant_id,
    )
    .execute(pool)
    .await
    .unwrap();
}

async fn seed_subscription(pool: &PgPool, id: &str, customer_id: &str, status: &str, tier: &str) {
    sqlx::query!(
        "INSERT INTO paddle_subscriptions (
            paddle_subscription_id, paddle_customer_id, status, tier,
            price_id, product_id)
         VALUES ($1, $2, $3, $4, 'pri_test', 'pro_test')",
        id,
        customer_id,
        status,
        tier,
    )
    .execute(pool)
    .await
    .unwrap();
}

/// An empty mirror reports zero drift — the healthy baseline every alert
/// is built on.
#[sqlx::test]
#[ignore = "needs live Postgres; CI runs SQLX_OFFLINE with no database"]
async fn healthy_mirror_reports_zero_drift(pool: PgPool) {
    let tenant = seed_tenant(&pool, "healthy@test.invalid", "basic").await;
    seed_customer(&pool, "ctm_ok", "healthy@test.invalid", Some(tenant)).await;
    seed_subscription(&pool, "sub_ok", "ctm_ok", "active", "basic").await;

    let drift = billing_drift_snapshot(&pool).await.unwrap();
    assert_eq!(
        drift,
        BillingDrift {
            unhydrated_stubs: 0,
            tier_mismatches: 0,
            tenantless_subscriptions: 0
        },
    );
}

/// Each drift class counts exactly itself: a tier that disagrees with the
/// subscription, an old email-less stub (fresh ones are in-flight, not
/// drift), and an active subscription with no tenant behind it.
#[sqlx::test]
#[ignore = "needs live Postgres; CI runs SQLX_OFFLINE with no database"]
async fn each_drift_class_is_counted_independently(pool: PgPool) {
    // 1. Tier mismatch: paying for professional, holding basic.
    let under = seed_tenant(&pool, "under@test.invalid", "basic").await;
    seed_customer(&pool, "ctm_under", "under@test.invalid", Some(under)).await;
    seed_subscription(&pool, "sub_under", "ctm_under", "active", "professional").await;

    // Downgrade drift too: canceled subscription but tenant keeps the tier.
    let over = seed_tenant(&pool, "over@test.invalid", "basic").await;
    seed_customer(&pool, "ctm_over", "over@test.invalid", Some(over)).await;
    seed_subscription(&pool, "sub_over", "ctm_over", "canceled", "basic").await;

    // 2. Stale stub: email-less and untouched over an hour ago. Fresh stub
    // must NOT count (customer.created may simply be in flight).
    sqlx::query!(
        "INSERT INTO paddle_customers (paddle_customer_id, email, updated_at)
         VALUES ('ctm_stale', '', now() - interval '2 hours')",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query!(
        "INSERT INTO paddle_customers (paddle_customer_id, email)
         VALUES ('ctm_fresh', '')",
    )
    .execute(&pool)
    .await
    .unwrap();

    // 3. Tenantless payer: active subscription, customer linked to nobody.
    seed_customer(&pool, "ctm_orphan", "nobody@test.invalid", None).await;
    seed_subscription(&pool, "sub_orphan", "ctm_orphan", "active", "basic").await;

    let drift = billing_drift_snapshot(&pool).await.unwrap();
    assert_eq!(
        drift.unhydrated_stubs, 1,
        "stale stub only; fresh stub is not drift"
    );
    assert_eq!(drift.tier_mismatches, 2, "upgrade miss + downgrade miss");
    assert_eq!(drift.tenantless_subscriptions, 1);
}

/// The cutover trap: an active subscription whose price carries no tier
/// metadata (live catalog missing custom_data.tier) must count as drift —
/// otherwise it silently upgrades nobody forever.
#[sqlx::test]
#[ignore = "needs live Postgres; CI runs SQLX_OFFLINE with no database"]
async fn tierless_active_subscription_is_drift(pool: PgPool) {
    let tenant = seed_tenant(&pool, "tierless@test.invalid", "free").await;
    seed_customer(&pool, "ctm_tierless", "tierless@test.invalid", Some(tenant)).await;
    sqlx::query!(
        "INSERT INTO paddle_subscriptions (
            paddle_subscription_id, paddle_customer_id, status, tier,
            price_id, product_id)
         VALUES ('sub_tierless', 'ctm_tierless', 'active', NULL, 'pri_test', 'pro_test')",
    )
    .execute(&pool)
    .await
    .unwrap();

    let drift = billing_drift_snapshot(&pool).await.unwrap();
    assert_eq!(
        drift.tier_mismatches, 1,
        "NULL tier on an active sub is drift"
    );
}

/// Convergence must clear the gauges: after the fix that the alerts would
/// have demanded, drift returns to zero with the same rows present.
#[sqlx::test]
#[ignore = "needs live Postgres; CI runs SQLX_OFFLINE with no database"]
async fn repair_clears_drift(pool: PgPool) {
    let tenant = seed_tenant(&pool, "repair@test.invalid", "free").await;
    seed_customer(&pool, "ctm_repair", "repair@test.invalid", Some(tenant)).await;
    seed_subscription(&pool, "sub_repair", "ctm_repair", "active", "professional").await;

    let before = billing_drift_snapshot(&pool).await.unwrap();
    assert_eq!(before.tier_mismatches, 1);

    // The repair: propagation runs (exactly what apply_tier_to_tenant does).
    sqlx::query!(
        r#"UPDATE tenants SET tier = 'professional'::text::account_tier
            WHERE id = $1"#,
        tenant,
    )
    .execute(&pool)
    .await
    .unwrap();

    let after = billing_drift_snapshot(&pool).await.unwrap();
    assert_eq!(after.tier_mismatches, 0);
}
