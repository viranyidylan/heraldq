//! Paystack → Postgres state sync.
//!
//! Every function here is idempotent: Paystack redelivers webhooks, provides
//! no event ids, and signs with an untimestamped HMAC — so identity is
//! synthesized (`PaystackEvent::dedup_key`) and upserts keyed on Paystack
//! entity codes converge to the latest state regardless of arrival order.

use sqlx::PgConnection;

use crate::paystack::events::{ChargePayload, CustomerPayload, SubscriptionPayload};

/// Plan code → heraldQ tier. Paystack plans carry no metadata, so the mapping
/// lives here. These are the TEST plans; swap for live plan codes at go-live.
const PLAN_TIERS: &[(&str, &str)] = &[
    ("PLN_sticj73pyngn646", "basic"),        // HeraldQ Starter
    ("PLN_g1g3cjpmclgpoqf", "professional"), // HeraldQ Pro
];

/// Values allowed in `tenants.tier` (the Postgres `account_tier` enum).
/// Anything else is ignored rather than written — an invalid enum value
/// would make the statement error forever, and Paystack would retry a
/// permanently-failing event to exhaustion.
const VALID_TIERS: [&str; 4] = ["free", "basic", "professional", "enterprise"];

fn tier_for_plan(plan_code: &str) -> Option<&str> {
    PLAN_TIERS
        .iter()
        .find(|(code, _)| *code == plan_code)
        .map(|(_, tier)| *tier)
        .filter(|tier| VALID_TIERS.contains(tier))
}

/// Record a dedup key in the ledger.
///
/// Returns `Ok(true)` the first time this key is seen, `Ok(false)` when it
/// was already processed (a redelivery). Runs inside the caller's
/// transaction so a processing failure rolls the ledger entry back too.
pub async fn record_event(
    conn: &mut PgConnection,
    dedup_key: &str,
    event_type: &str,
) -> Result<bool, sqlx::Error> {
    let inserted = sqlx::query!(
        r#"INSERT INTO paystack_processed_events (dedup_key, event_type)
            VALUES ($1, $2)
            ON CONFLICT (dedup_key) DO NOTHING
            RETURNING dedup_key"#,
        dedup_key,
        event_type,
    )
    .fetch_optional(&mut *conn)
    .await?;

    Ok(inserted.is_some())
}

/// Mirror a Paystack customer. The tenant link is resolved by email here —
/// not at signup — because the Paystack customer may exist before any
/// heraldQ tenant does.
pub async fn upsert_customer(
    conn: &mut PgConnection,
    customer: &CustomerPayload,
) -> Result<(), sqlx::Error> {
    sqlx::query!(
        r#"INSERT INTO paystack_customers (paystack_customer_code, email, tenant_id)
            VALUES ($1, $2,
                    (SELECT id FROM tenants WHERE email = $2))
            ON CONFLICT (paystack_customer_code) DO UPDATE
            SET email = EXCLUDED.email,
                tenant_id = (SELECT id FROM tenants WHERE email = EXCLUDED.email),
                updated_at = now()"#,
        customer.customer_code,
        customer.email,
    )
    .execute(&mut *conn)
    .await?;
    Ok(())
}

/// Mirror a subscription event (`subscription.create/.enable/.not_renew/.disable`)
/// and propagate the effective tier to the linked tenant (if any).
///
/// A stub customer row is inserted first if the customer hasn't been seen —
/// the subscription payload carries full customer details, so it's never
/// actually a stub, but the INSERT keeps the FK satisfied on redeliveries
/// with partially-different payloads.
pub async fn upsert_subscription(
    conn: &mut PgConnection,
    sub: &SubscriptionPayload,
) -> Result<(), sqlx::Error> {
    upsert_customer(conn, &sub.customer).await?;

    let status = sub.status.as_deref().unwrap_or("active");
    // Tier follows the plan while active/non-renewing (non-renewing keeps
    // access to the end of the paid period); disable is terminal → free.
    let effective_tier: Option<String> = match status {
        "disabled" | "cancelled" => Some("free".to_owned()),
        _ => tier_for_plan(&sub.plan.plan_code).map(str::to_owned),
    };

    sqlx::query!(
        r#"INSERT INTO paystack_subscriptions (
                paystack_subscription_code, paystack_customer_code, status,
                plan_code, tier, email_token, amount, currency)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
            ON CONFLICT (paystack_subscription_code) DO UPDATE
            SET status = EXCLUDED.status,
                plan_code = EXCLUDED.plan_code,
                tier = EXCLUDED.tier,
                email_token = EXCLUDED.email_token,
                amount = EXCLUDED.amount,
                currency = EXCLUDED.currency,
                updated_at = now()"#,
        sub.subscription_code,
        sub.customer.customer_code,
        status,
        sub.plan.plan_code,
        effective_tier,
        sub.email_token,
        sub.amount,
        sub.currency,
    )
    .execute(&mut *conn)
    .await?;

    if let Some(tier) = effective_tier {
        apply_tier_to_tenant(conn, &sub.customer.customer_code, &tier).await?;
    }

    Ok(())
}

/// `charge.success`: mirror the customer; when the charge belongs to one of
/// our plans, project the tier immediately. The subscription row itself is
/// owned by the `subscription.*` events (charges carry no subscription_code),
/// but tier projection here is convergent, so ordering doesn't matter.
pub async fn handle_charge(
    conn: &mut PgConnection,
    charge: &ChargePayload,
) -> Result<(), sqlx::Error> {
    upsert_customer(conn, &charge.customer).await?;

    if let Some(plan) = &charge.plan
        && let Some(tier) = tier_for_plan(&plan.plan_code)
    {
        apply_tier_to_tenant(conn, &charge.customer.customer_code, tier).await?;
    }

    Ok(())
}

async fn apply_tier_to_tenant(
    conn: &mut PgConnection,
    paystack_customer_code: &str,
    tier: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query!(
        r#"UPDATE tenants t
            SET tier = $2::text::account_tier
            FROM paystack_customers c
            WHERE c.paystack_customer_code = $1
              AND t.email = c.email"#,
        paystack_customer_code,
        tier,
    )
    .execute(&mut *conn)
    .await?;
    Ok(())
}
