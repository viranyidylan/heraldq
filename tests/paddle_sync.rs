//! Tests for the Paddle webhook pipeline: dedup, out-of-order upserts,
//! tier propagation to tenants, and the signature→ledger→sync transaction.
//!
//! These need a live Postgres (each `#[sqlx::test]` creates a throwaway
//! database and runs ./migrations), so they are ignored in CI where
//! SQLX_OFFLINE=true and no database exists. Run locally with:
//!
//! ```sh
//! export $(grep DATABASE_URL .env) && cargo test -- --ignored
//! ```

use chrono::Utc;
use hmac::Mac;
use serde_json::json;
use sliq::paddle::{events::WebhookEvent, signature::verify_signature};
use sqlx::PgPool;
use uuid::Uuid;

const SECRET: &str = "pdl_ntfset_test";

/// Build a full webhook event envelope with a correct Paddle-style signature
/// over the serialized body.
fn signed_event(event_type: &str, event_id: &str, data: serde_json::Value) -> (String, Vec<u8>) {
    let body = json!({
        "event_type": event_type,
        "event_id": event_id,
        "occurred_at": "2026-08-19T12:00:00Z",
        "data": data,
    })
    .to_string();

    let ts = Utc::now().timestamp();
    let mut mac = hmac::Hmac::<sha2::Sha256>::new_from_slice(SECRET.as_bytes()).unwrap();
    mac.update(format!("{ts}:").as_bytes());
    mac.update(body.as_bytes());
    let digest = hex::encode(mac.finalize().into_bytes());

    (format!("ts={ts};h1=v1:{digest}"), body.into_bytes())
}

fn customer_event(customer_id: &str, email: &str) -> serde_json::Value {
    json!({ "id": customer_id, "email": email })
}

fn subscription_event(customer_id: &str, status: &str, tier: Option<&str>) -> serde_json::Value {
    json!({
        "id": "sub_01h",
        "status": status,
        "customer_id": customer_id,
        "scheduled_change": null,
        "current_billing_period": {
            "starts_at": "2026-08-19T00:00:00Z",
            "ends_at": "2026-09-19T00:00:00Z",
        },
        "canceled_at": null,
        "next_billed_at": "2026-09-19T00:00:00Z",
        "items": [{
            "status": "active",
            "quantity": 1,
            "price": {
                "id": "pri_01h",
                "product_id": "pro_01h",
                "custom_data": tier.map(|t| json!({ "tier": t })),
            }
        }]
    })
}

/// The same work the route handler does: verify, then ledger + sync in one tx.
/// Returns whether the event was new (vs. deduped).
async fn deliver(pool: &PgPool, header: &str, body: &[u8]) -> bool {
    verify_signature(header, body, SECRET, Utc::now())
        .expect("test events are correctly signed");

    let event: WebhookEvent = serde_json::from_slice(body).expect("test events parse");

    let mut tx = pool.begin().await.unwrap();
    let is_new =
        sliq::paddle::sync::record_event(&mut tx, &event.event_id, &event.event_type, event.occurred_at)
            .await
            .unwrap();
    if is_new {
        sliq::paddle::process_event(&mut tx, &event).await.unwrap();
    }
    tx.commit().await.unwrap();
    is_new
}

async fn seed_tenant(pool: &PgPool, email: &str) -> Uuid {
    sqlx::query_scalar!(
        r#"INSERT INTO tenants (name, tier, email, hmac_secret)
        VALUES ('paddle-test', 'free', $1, 'not-a-real-secret')
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
async fn redelivery_is_deduped(pool: PgPool) {
    let (header, body) = signed_event(
        "customer.created",
        "evt_dup",
        customer_event("ctm_01h", "dup@test.invalid"),
    );

    assert!(deliver(&pool, &header, &body).await, "first delivery is new");
    assert!(
        !deliver(&pool, &header, &body).await,
        "redelivery is deduped"
    );

    // Exactly one customer row, despite two deliveries.
    let count: i64 = sqlx::query_scalar!("SELECT count(*) FROM paddle_customers")
        .fetch_one(&pool)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(count, 1);
}

#[sqlx::test]
#[ignore = "needs live Postgres; CI runs SQLX_OFFLINE with no database"]
async fn subscription_before_customer_creates_stub(pool: PgPool) {
    // Out-of-order delivery: the subscription event references a customer
    // whose customer.created hasn't landed (e.g. it's still being retried).
    let (header, body) = signed_event(
        "subscription.created",
        "evt_ooo",
        subscription_event("ctm_01h", "active", Some("basic")),
    );

    assert!(deliver(&pool, &header, &body).await);

    let sub = sqlx::query!(
        r#"SELECT status, tier::text AS tier, price_id FROM paddle_subscriptions"#
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(sub.status, "active");
    assert_eq!(sub.tier.as_deref(), Some("basic"));
    assert_eq!(sub.price_id, "pri_01h");

    let stub_email: String = sqlx::query_scalar!(
        "SELECT email FROM paddle_customers WHERE paddle_customer_id = 'ctm_01h'"
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(stub_email, "", "stub customer has no email yet");
}

#[sqlx::test]
#[ignore = "needs live Postgres; CI runs SQLX_OFFLINE with no database"]
async fn active_subscription_upgrades_tenant_by_email(pool: PgPool) {
    let email = "paid@test.invalid";
    let tenant_id = seed_tenant(&pool, email).await;

    // Customer first, so the email link exists...
    let (h, b) = signed_event("customer.created", "evt_c1", customer_event("ctm_01h", email));
    assert!(deliver(&pool, &h, &b).await);

    // ...then the subscription on the professional price.
    let (h, b) = signed_event(
        "subscription.created",
        "evt_s1",
        subscription_event("ctm_01h", "active", Some("professional")),
    );
    assert!(deliver(&pool, &h, &b).await);

    assert_eq!(tenant_tier(&pool, tenant_id).await, "professional");

    // Customer row is linked to the tenant it matched.
    let linked: Option<Uuid> = sqlx::query_scalar!(
        "SELECT tenant_id FROM paddle_customers WHERE paddle_customer_id = 'ctm_01h'"
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(linked, Some(tenant_id));
}

#[sqlx::test]
#[ignore = "needs live Postgres; CI runs SQLX_OFFLINE with no database"]
async fn canceled_subscription_downgrades_tenant_to_free(pool: PgPool) {
    let email = "churn@test.invalid";
    let tenant_id = seed_tenant(&pool, email).await;

    let (h, b) = signed_event("customer.created", "evt_c2", customer_event("ctm_02h", email));
    deliver(&pool, &h, &b).await;

    let (h, b) = signed_event(
        "subscription.created",
        "evt_s2",
        subscription_event("ctm_02h", "active", Some("basic")),
    );
    deliver(&pool, &h, &b).await;
    assert_eq!(tenant_tier(&pool, tenant_id).await, "basic");

    // Terminal cancel: the paid period has ended, tier drops to free.
    let mut canceled = subscription_event("ctm_02h", "canceled", Some("basic"));
    canceled["canceled_at"] = json!("2026-09-19T00:00:00Z");
    let (h, b) = signed_event("subscription.canceled", "evt_s3", canceled);
    deliver(&pool, &h, &b).await;

    assert_eq!(tenant_tier(&pool, tenant_id).await, "free");
    let status: String =
        sqlx::query_scalar!("SELECT status FROM paddle_subscriptions WHERE paddle_subscription_id = 'sub_01h'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(status, "canceled");
}

#[sqlx::test]
#[ignore = "needs live Postgres; CI runs SQLX_OFFLINE with no database"]
async fn unknown_event_types_ack_without_side_effects(pool: PgPool) {
    let (header, body) = signed_event("payout.paid", "evt_p1", json!({ "id": "po_01h" }));

    assert!(deliver(&pool, &header, &body).await);

    let events: i64 = sqlx::query_scalar!("SELECT count(*) FROM paddle_processed_events")
        .fetch_one(&pool)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(events, 1, "unknown events are still ledgered");
    let customers: i64 = sqlx::query_scalar!("SELECT count(*) FROM paddle_customers")
        .fetch_one(&pool)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(customers, 0, "but produce no rows");
}

#[sqlx::test]
#[ignore = "needs live Postgres; CI runs SQLX_OFFLINE with no database"]
async fn invalid_tier_metadata_never_reaches_the_database(pool: PgPool) {
    let email = "meta@test.invalid";
    let tenant_id = seed_tenant(&pool, email).await;

    let (h, b) = signed_event("customer.created", "evt_c3", customer_event("ctm_03h", email));
    deliver(&pool, &h, &b).await;

    // A bogus custom_data.tier must not error the handler (which would make
    // Paddle retry forever) nor downgrade the tenant.
    let (h, b) = signed_event(
        "subscription.created",
        "evt_s4",
        subscription_event("ctm_03h", "active", Some("platinum")),
    );
    deliver(&pool, &h, &b).await;

    assert_eq!(
        tenant_tier(&pool, tenant_id).await,
        "free",
        "invalid tier leaves the tenant untouched"
    );
    let tier: Option<String> = sqlx::query_scalar!(
        "SELECT tier::text FROM paddle_subscriptions"
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(tier.as_deref(), Some("platinum"), "raw metadata is mirrored verbatim");
}
