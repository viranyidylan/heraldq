//! Paddle → Postgres state sync.
//!
//! Every function here is idempotent: Paddle delivers webhooks at-least-once
//! with the same `event_id` on every retry, and `subscription.updated` can
//! arrive before its `subscription.created` if the first delivery is still
//! being retried. Upserts keyed on the Paddle resource id converge to the
//! latest state regardless of arrival order or duplication.

use chrono::{DateTime, Utc};
use tracing::{error, warn};

use crate::paddle::api::PaddleApi;
use crate::paddle::events::{CustomerPayload, SubscriptionPayload};

/// Values allowed in `tenants.tier` (the Postgres `account_tier` enum).
/// Anything else in a price's `custom_data.tier` is ignored rather than
/// written — an invalid enum value would make the statement error forever,
/// and Paddle would retry a permanently-failing event to exhaustion.
const VALID_TIERS: [&str; 4] = ["free", "basic", "professional", "enterprise"];

/// Record an event id in the dedup ledger.
///
/// Returns `Ok(true)` the first time this id is seen, `Ok(false)` when it was
/// already processed (a redelivery). This is the "have we done this?" gate:
/// callers ack immediately on `false` without touching anything else.
///
/// Runs inside the caller's transaction so that a processing failure rolls
/// the ledger entry back too — otherwise a redelivery would be deduped away
/// before its work ever succeeded.
pub async fn record_event(
    conn: &mut sqlx::PgConnection,
    event_id: &str,
    event_type: &str,
    occurred_at: Option<DateTime<Utc>>,
) -> Result<bool, sqlx::Error> {
    let inserted = sqlx::query!(
        r#"INSERT INTO paddle_processed_events (event_id, event_type, occurred_at)
            VALUES ($1, $2, $3)
            ON CONFLICT (event_id) DO NOTHING
            RETURNING event_id"#,
        event_id,
        event_type,
        occurred_at,
    )
    .fetch_optional(conn)
    .await?;

    Ok(inserted.is_some())
}

/// Mirror a Paddle customer.
///
/// The tenant link is resolved by email when it can be, but a previously
/// established link is never dropped: `COALESCE` keeps the stored
/// `tenant_id` when the new email matches no tenant (e.g. the customer
/// changed their email in Paddle before we saw `customer.updated`).
pub async fn upsert_customer(
    conn: &mut sqlx::PgConnection,
    customer: &CustomerPayload,
) -> Result<(), sqlx::Error> {
    // Stored lowercased: the email is the linkage key to tenants, and signup
    // lowercases its side too, so case differences (Paddle's collector vs a
    // phone keyboard) can never break the match.
    let email = customer.email.to_lowercase();
    sqlx::query!(
        r#"INSERT INTO paddle_customers (paddle_customer_id, email, tenant_id)
            VALUES ($1, $2,
                    (SELECT id FROM tenants WHERE email = $2))
            ON CONFLICT (paddle_customer_id) DO UPDATE
            SET email = EXCLUDED.email,
                tenant_id = COALESCE(
                    (SELECT id FROM tenants WHERE email = EXCLUDED.email),
                    paddle_customers.tenant_id),
                updated_at = now()"#,
        customer.id,
        email,
    )
    .execute(conn)
    .await?;
    Ok(())
}

/// Mirror a Paddle subscription, then propagate the effective tier to the
/// linked tenant (if any).
///
/// A stub customer row is inserted first when the `customer.created` event
/// hasn't landed yet — out-of-order delivery is normal under retries. If the
/// stub is still email-less (that event may *never* arrive: Paddle reuses
/// existing customers by email at checkout, so repeat buyers never generate
/// a new `customer.created`), the customer is fetched from the Paddle API so
/// the email link — and with it the tier upgrade — converges now.
pub async fn upsert_subscription(
    conn: &mut sqlx::PgConnection,
    sub: &SubscriptionPayload,
    api: Option<&PaddleApi>,
) -> Result<(), sqlx::Error> {
    let price = sub.primary_price();

    sqlx::query!(
        r#"INSERT INTO paddle_customers (paddle_customer_id, email)
            VALUES ($1, '')
            ON CONFLICT (paddle_customer_id) DO NOTHING"#,
        sub.customer_id,
    )
    .execute(&mut *conn)
    .await?;

    hydrate_stub_customer(&mut *conn, api, &sub.customer_id).await?;

    sqlx::query!(
        r#"INSERT INTO paddle_subscriptions (
                paddle_subscription_id, paddle_customer_id, status, tier,
                price_id, product_id, scheduled_change,
                current_period_start, current_period_end,
                canceled_at, next_billed_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
            ON CONFLICT (paddle_subscription_id) DO UPDATE
            SET status = EXCLUDED.status,
                tier = EXCLUDED.tier,
                price_id = EXCLUDED.price_id,
                product_id = EXCLUDED.product_id,
                scheduled_change = EXCLUDED.scheduled_change,
                current_period_start = EXCLUDED.current_period_start,
                current_period_end = EXCLUDED.current_period_end,
                canceled_at = EXCLUDED.canceled_at,
                next_billed_at = EXCLUDED.next_billed_at,
                updated_at = now()"#,
        sub.id,
        sub.customer_id,
        sub.status,
        sub.tier(),
        price.map(|p| p.id.as_str()),
        price.map(|p| p.product_id.as_str()),
        sub.scheduled_change,
        sub.current_billing_period
            .as_ref()
            .and_then(|p| p.starts_at),
        sub.current_billing_period.as_ref().and_then(|p| p.ends_at),
        sub.canceled_at,
        sub.next_billed_at,
    )
    .execute(&mut *conn)
    .await?;

    if let Some(effective_tier) = effective_tier(&sub.status, sub.tier()) {
        apply_tier_to_tenant(&mut *conn, &sub.customer_id, &effective_tier).await?;
    }

    Ok(())
}

/// Fill in an email-less stub customer from the Paddle API.
///
/// No-op when the customer is already hydrated or no API client is
/// configured. Transient Paddle API failures propagate (→ 500 → Paddle
/// retries the webhook); permanent ones (bad key, unknown customer) are
/// logged and skipped — the stub converges on any later customer event.
async fn hydrate_stub_customer(
    conn: &mut sqlx::PgConnection,
    api: Option<&PaddleApi>,
    customer_id: &str,
) -> Result<(), sqlx::Error> {
    let row = sqlx::query!(
        r#"SELECT email FROM paddle_customers WHERE paddle_customer_id = $1"#,
        customer_id,
    )
    .fetch_optional(&mut *conn)
    .await?;

    if !row.is_some_and(|r| r.email.is_empty()) {
        return Ok(());
    }

    let Some(api) = api else {
        warn!(
            paddle_customer_id = customer_id,
            "stub customer and no PADDLE_API_KEY — email link deferred to a customer event"
        );
        return Ok(());
    };

    match api.get_customer(customer_id).await {
        Ok(customer) => upsert_customer(&mut *conn, &customer).await,
        Err(crate::paddle::api::ApiError::Transient(detail)) => {
            error!(paddle_customer_id = customer_id, %detail,
                "transient Paddle API failure — failing the webhook so Paddle retries");
            Err(sqlx::Error::Io(std::io::Error::other(detail)))
        }
        Err(crate::paddle::api::ApiError::Permanent(detail)) => {
            error!(paddle_customer_id = customer_id, %detail,
                "cannot hydrate stub customer — skipping");
            Ok(())
        }
    }
}

/// The tier a tenant should hold given a subscription's Paddle status.
///
/// `active`/`trialing` map to the price's tier; `past_due` keeps it for the
/// dunning grace period (the tenant sees payment failures elsewhere);
/// `paused` and `canceled` drop the tenant back to `free`. `canceled` only
/// arrives once the paid period has actually ended, so this honors "your key
/// keeps working to the end of the paid period".
fn effective_tier(status: &str, price_tier: Option<String>) -> Option<String> {
    let tier = match status {
        "active" | "trialing" | "past_due" => price_tier?,
        "paused" | "canceled" => "free".to_owned(),
        _ => return None, // unknown status: touch nothing
    };

    if VALID_TIERS.contains(&tier.as_str()) {
        Some(tier)
    } else {
        None
    }
}

async fn apply_tier_to_tenant(
    conn: &mut sqlx::PgConnection,
    paddle_customer_id: &str,
    tier: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query!(
        r#"UPDATE tenants t
            SET tier = $2::text::account_tier
            FROM paddle_customers c
            WHERE c.paddle_customer_id = $1
              AND (t.id = c.tenant_id
                   OR (c.tenant_id IS NULL
                       AND c.email <> ''
                       AND t.email = c.email))"#,
        paddle_customer_id,
        tier,
    )
    .execute(conn)
    .await?;
    Ok(())
}

/// Claim the Paddle customer(s) matching a just-signed-up tenant's email,
/// then apply whatever tier their mirrored subscription implies.
///
/// Called from `signup` inside its transaction: signup is the front door,
/// so this is the moment a deterministic `tenant_id` link replaces email
/// matching. It closes the gap where a customer who paid before creating
/// their account (legacy checkouts, support-created subscriptions, or
/// webhooks that arrived pre-signup) would otherwise stay on `free` until
/// Paddle's next billing event — up to a month later.
pub async fn link_tenant_and_apply_tier(
    conn: &mut sqlx::PgConnection,
    tenant_id: uuid::Uuid,
    email: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query!(
        r#"UPDATE paddle_customers
            SET tenant_id = $1,
                updated_at = now()
            WHERE email = $2"#,
        tenant_id,
        email,
    )
    .execute(&mut *conn)
    .await?;

    // Latest mirrored subscription for this tenant, if any. Out-of-order or
    // redelivered events converge, so `updated_at DESC` is the newest state
    // Paddle has told us about.
    let sub = sqlx::query!(
        r#"SELECT s.status, s.tier
            FROM paddle_subscriptions s
            JOIN paddle_customers c
              ON c.paddle_customer_id = s.paddle_customer_id
            WHERE c.tenant_id = $1
            ORDER BY s.updated_at DESC
            LIMIT 1"#,
        tenant_id,
    )
    .fetch_optional(&mut *conn)
    .await?;

    if let Some(row) = sub
        && let Some(tier) = effective_tier(&row.status, row.tier)
    {
        sqlx::query!(
            r#"UPDATE tenants SET tier = $2::text::account_tier WHERE id = $1"#,
            tenant_id,
            tier,
        )
        .execute(&mut *conn)
        .await?;
    }

    Ok(())
}
