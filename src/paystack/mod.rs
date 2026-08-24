//! Paystack billing integration: receive webhooks, verify signatures, and
//! mirror customer/subscription state into Postgres.
//!
//! Mirrors the Paddle flow (`crate::paddle`) with Paystack's differences:
//!
//! - Signature: HMAC-SHA512 of the raw body keyed by the secret key, sent in
//!   `x-paystack-signature`. No timestamp — no replay window to enforce, so
//!   idempotency carries all the weight.
//! - Dedup: no event ids from Paystack; keys are synthesized per event type
//!   (`PaystackEvent::dedup_key`).
//! - Semantics: `subscription.not_renew` = cancelled-but-paid-up (keep tier
//!   until period end), `subscription.disable` = terminal (tier → free).

pub mod events;
pub mod signature;
pub mod sync;

use sqlx::PgConnection;
use tracing::{error, info, warn};

use crate::paystack::events::PaystackEvent;

/// Dispatch a deduped event to its handler.
///
/// Errors propagate to the caller (→ 500 → Paystack retry); only conditions a
/// redelivery could never fix are handled inline. Unknown event types ack
/// silently — the dashboard's webhook list covers more events than we act on.
pub async fn process_event(
    conn: &mut PgConnection,
    event: &PaystackEvent,
) -> Result<(), sqlx::Error> {
    match event.event.as_str() {
        "charge.success" => match event.charge() {
            Some(charge) => sync::handle_charge(conn, &charge).await,
            None => {
                // Paystack redelivers byte-identical payloads, so a retry
                // can't succeed where the first attempt failed to parse.
                error!("unparseable charge.success payload");
                Ok(())
            }
        },
        "subscription.create"
        | "subscription.enable"
        | "subscription.not_renew"
        | "subscription.disable" => match event.subscription() {
            Some(sub) => sync::upsert_subscription(conn, &sub).await,
            None => {
                error!(event = %event.event, "unparseable subscription payload");
                Ok(())
            }
        },
        "invoice.create" | "invoice.update" => {
            info!(event = %event.event, "invoice event observed");
            Ok(())
        }
        "invoice.payment_failed" => {
            // Dunning: Paystack retries the card; a persistent failure ends
            // in subscription.disable, which reverts the tier.
            warn!(event = %event.event, "subscription renewal charge failed");
            Ok(())
        }
        other => {
            warn!(event = other, "webhook subscribed but unhandled");
            Ok(())
        }
    }
}
