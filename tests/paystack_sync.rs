//! Tests for the Paystack webhook pipeline: dedup via synthesized keys,
//! tier projection on subscription lifecycle, and the charge path.
//!
//! These need a live Postgres (each `#[sqlx::test]` creates a throwaway
//! database and runs ./migrations), so they are ignored in CI where
//! SQLX_OFFLINE=true and no database exists. Run locally with:
//!
//! ```sh
//! export $(grep DATABASE_URL .env) && cargo test -- --ignored
//! ```

use serde_json::json;
use sliq::paystack::{events::PaystackEvent, signature::verify_signature};
use sqlx::PgPool;
use uuid::Uuid;

const SECRET: &str = "sk_test_example";
const STARTER_PLAN: &str = "PLN_sticj73pyngn646";
const PRO_PLAN: &str = "PLN_g1g3cjpmclgpoqf";

/// Build a webhook payload with a correct Paystack-style signature.
fn signed_event(event: &str, data: serde_json::Value) -> (String, Vec<u8>) {
    let body = json!({ "event": event, "data": data }).to_string();

    use hmac::Mac;
    let mut mac = hmac::Hmac::<sha2::Sha512>::new_from_slice(SECRET.as_bytes()).unwrap();
    mac.update(body.as_bytes());
    let digest = hex::encode(mac.finalize().into_bytes());

    (digest, body.into_bytes())
}

fn customer(code: &str, email: &str) -> serde_json::Value {
    json!({ "customer_code": code, "email": email, "first_name": "", "last_name": "" })
}

fn subscription(customer_code: &str, email: &str, plan: &str, status: &str) -> serde_json::Value {
    json!({
        "subscription_code": "SUB_01",
        "email_token": "tok_abc",
        "status": status,
        "amount": 54900,
        "currency": "ZAR",
        "plan": { "plan_code": plan, "name": "HeraldQ Starter", "amount": 54900 },
        "customer": customer(customer_code, email),
    })
}

fn charge(
    reference: &str,
    customer_code: &str,
    email: &str,
    plan: Option<&str>,
) -> serde_json::Value {
    json!({
        "reference": reference,
        "amount": 54900,
        "currency": "ZAR",
        "customer": customer(customer_code, email),
        "plan": plan.map(|p| json!({ "plan_code": p, "name": "HeraldQ Starter", "amount": 54900 })),
    })
}

/// The same work the route handler does: verify, then ledger + process in
/// one tx. Returns whether the event was new (vs. deduped).
async fn deliver(pool: &PgPool, header: &str, body: &[u8]) -> bool {
    verify_signature(header, body, SECRET).expect("test events are correctly signed");

    let event: PaystackEvent = serde_json::from_slice(body).expect("test events parse");

    let mut tx = pool.begin().await.unwrap();
    let is_new = sliq::paystack::sync::record_event(
        &mut tx,
        &event.dedup_key().expect("test events carry identity"),
        &event.event,
    )
    .await
    .unwrap();
    if is_new {
        sliq::paystack::process_event(&mut tx, &event)
            .await
            .unwrap();
    }
    tx.commit().await.unwrap();
    is_new
}

async fn seed_tenant(pool: &PgPool, email: &str) -> Uuid {
    sqlx::query_scalar!(
        r#"INSERT INTO tenants (name, tier, email, hmac_secret)
        VALUES ('paystack-test', 'free', $1, 'not-a-real-secret')
        RETURNING id"#,
        email,
    )
    .fetch_one(pool)
    .await
    .expect("tenant seed should insert")
}

async fn tenant_tier(pool: &PgPool, tenant_id: Uuid) -> String {
    sqlx::query_scalar!(r#"SELECT tier::text FROM tenants WHERE id = $1"#, tenant_id)
        .fetch_one(pool)
        .await
        .unwrap()
        .expect("tier has a default")
}

#[sqlx::test]
#[ignore = "needs live Postgres; CI runs SQLX_OFFLINE with no database"]
async fn subscription_create_upgrades_tenant_by_email(pool: PgPool) {
    let email = "paid@test.invalid";
    let tenant_id = seed_tenant(&pool, email).await;

    let (h, b) = signed_event(
        "subscription.create",
        subscription("CUS_1", email, PRO_PLAN, "active"),
    );
    assert!(deliver(&pool, &h, &b).await);

    assert_eq!(tenant_tier(&pool, tenant_id).await, "professional");

    let sub =
        sqlx::query!("SELECT status, tier, email_token, plan_code FROM paystack_subscriptions")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(sub.status, "active");
    assert_eq!(sub.tier.as_deref(), Some("professional"));
    assert_eq!(sub.email_token.as_deref(), Some("tok_abc"));
    assert_eq!(sub.plan_code, PRO_PLAN);
}

#[sqlx::test]
#[ignore = "needs live Postgres; CI runs SQLX_OFFLINE with no database"]
async fn redelivery_is_deduped(pool: PgPool) {
    let (h, b) = signed_event(
        "subscription.create",
        subscription("CUS_1", "dup@test.invalid", STARTER_PLAN, "active"),
    );

    assert!(deliver(&pool, &h, &b).await, "first delivery is new");
    assert!(!deliver(&pool, &h, &b).await, "redelivery is deduped");

    let count: i64 = sqlx::query_scalar!("SELECT count(*) FROM paystack_subscriptions")
        .fetch_one(&pool)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(count, 1);
}

#[sqlx::test]
#[ignore = "needs live Postgres; CI runs SQLX_OFFLINE with no database"]
async fn not_renew_keeps_tier_until_disable(pool: PgPool) {
    let email = "churn@test.invalid";
    let tenant_id = seed_tenant(&pool, email).await;

    let (h, b) = signed_event(
        "subscription.create",
        subscription("CUS_2", email, STARTER_PLAN, "active"),
    );
    deliver(&pool, &h, &b).await;
    assert_eq!(tenant_tier(&pool, tenant_id).await, "basic");

    // Cancelled but paid-up: access (and tier) hold until period end.
    let (h, b) = signed_event(
        "subscription.not_renew",
        subscription("CUS_2", email, STARTER_PLAN, "non-renewing"),
    );
    deliver(&pool, &h, &b).await;
    assert_eq!(
        tenant_tier(&pool, tenant_id).await,
        "basic",
        "non-renewing keeps the tier"
    );

    // Terminal disable → free.
    let (h, b) = signed_event(
        "subscription.disable",
        subscription("CUS_2", email, STARTER_PLAN, "disabled"),
    );
    deliver(&pool, &h, &b).await;
    assert_eq!(tenant_tier(&pool, tenant_id).await, "free");
    let status: String = sqlx::query_scalar!("SELECT status FROM paystack_subscriptions")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(status, "disabled");
}

#[sqlx::test]
#[ignore = "needs live Postgres; CI runs SQLX_OFFLINE with no database"]
async fn charge_with_plan_projects_tier_without_subscription_row(pool: PgPool) {
    let email = "charge@test.invalid";
    let tenant_id = seed_tenant(&pool, email).await;

    // charge.success can arrive before subscription.create; tier projection
    // must work from the charge alone.
    let (h, b) = signed_event(
        "charge.success",
        charge("ref_1", "CUS_3", email, Some(STARTER_PLAN)),
    );
    assert!(deliver(&pool, &h, &b).await);

    assert_eq!(tenant_tier(&pool, tenant_id).await, "basic");
    let subs: i64 = sqlx::query_scalar!("SELECT count(*) FROM paystack_subscriptions")
        .fetch_one(&pool)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(subs, 0, "charges don't fabricate subscription rows");

    // A plan-less charge (one-off) must not touch the tier.
    let (h, b) = signed_event("charge.success", charge("ref_2", "CUS_3", email, None));
    deliver(&pool, &h, &b).await;
    assert_eq!(tenant_tier(&pool, tenant_id).await, "basic");
}

#[sqlx::test]
#[ignore = "needs live Postgres; CI runs SQLX_OFFLINE with no database"]
async fn unknown_plan_leaves_tenant_untouched(pool: PgPool) {
    let email = "stranger@test.invalid";
    let tenant_id = seed_tenant(&pool, email).await;

    let (h, b) = signed_event(
        "subscription.create",
        subscription("CUS_4", email, "PLN_somebody_elses_plan", "active"),
    );
    deliver(&pool, &h, &b).await;

    assert_eq!(
        tenant_tier(&pool, tenant_id).await,
        "free",
        "unmapped plan never projects a tier"
    );
    let tier: Option<String> = sqlx::query_scalar!("SELECT tier FROM paystack_subscriptions")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(tier, None, "mirror stores no tier for unknown plans");
}

#[sqlx::test]
#[ignore = "needs live Postgres; CI runs SQLX_OFFLINE with no database"]
async fn unhandled_events_are_ledgered_without_side_effects(pool: PgPool) {
    let (h, b) = signed_event(
        "invoice.payment_failed",
        json!({ "id": 42, "status": "failed" }),
    );
    assert!(deliver(&pool, &h, &b).await);

    let events: i64 = sqlx::query_scalar!("SELECT count(*) FROM paystack_processed_events")
        .fetch_one(&pool)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(events, 1, "observed events are still ledgered");
    let customers: i64 = sqlx::query_scalar!("SELECT count(*) FROM paystack_customers")
        .fetch_one(&pool)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(customers, 0, "but produce no rows");
}

/// Unit tests for the synthesized dedup key (no DB).
#[test]
fn dedup_key_prefers_reference_then_id_then_code() {
    let mk = |data: serde_json::Value| PaystackEvent {
        event: "charge.success".into(),
        data,
    };
    assert_eq!(
        mk(json!({"reference": "ref_1", "id": 9}))
            .dedup_key()
            .as_deref(),
        Some("charge.success:ref_1")
    );
    assert_eq!(
        mk(json!({"id": 9})).dedup_key().as_deref(),
        Some("charge.success:9")
    );
    assert_eq!(
        mk(json!({"subscription_code": "SUB_x"}))
            .dedup_key()
            .as_deref(),
        Some("charge.success:SUB_x")
    );
    assert_eq!(mk(json!({})).dedup_key(), None);
}
