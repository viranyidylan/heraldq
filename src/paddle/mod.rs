//! Paddle billing integration: receive webhooks, verify signatures, and
//! mirror customer/subscription state into Postgres.
//!
//! The flow, per delivery:
//!
//! 1. `routes::paddle_webhook` verifies the `Paddle-Signature` HMAC against
//!    the raw body (see `signature`).
//! 2. One transaction runs `record_event` (the dedup ledger) and, if the
//!    event is new, `process_event`. Committing both together means a failed
//!    handler rolls the ledger entry back with it — a Paddle retry then gets
//!    a clean "not yet processed" answer instead of being deduped away.
//! 3. `process_event` dispatches on `event_type`; `sync` upserts the
//!    mirrored row(s) and propagates the effective tier to the tenant.
//!
//! Only `2xx` within 5s counts as delivered to Paddle; anything else is
//! retried (3 attempts over ~15 min in sandbox, 60 over ~3 days live). So
//! permanent problems (bad signature, unparseable body) still return
//! non-2xx — redelivery can't fix them, but it costs nothing and keeps the
//! dashboard's notification log honest — while transient problems (DB down)
//! return 500 and recover automatically.

pub mod api;
pub mod events;
pub mod signature;
pub mod sync;

use sqlx::PgConnection;
use tracing::{error, info, warn};

use crate::paddle::api::PaddleApi;
use crate::paddle::events::PaddleEvent;

/// Dispatch a deduped event to its handler.
///
/// Errors propagate to the caller (→ 500 → Paddle retry); only conditions a
/// redelivery could never fix are handled inline. Unknown event types ack
/// silently — a destination can subscribe to more events than the code
/// handles.
///
/// `api` is used to converge stub customers (see `api` module); `None`
/// disables that fetch and the mirror still converges on customer events.
pub async fn process_event(
    conn: &mut PgConnection,
    event: &PaddleEvent,
    api: Option<&PaddleApi>,
) -> Result<(), sqlx::Error> {
    match event.event_type.as_str() {
        "customer.created" | "customer.updated" => match event.customer() {
            Some(customer) => sync::upsert_customer(conn, &customer).await,
            None => {
                // Paddle redelivers byte-identical payloads, so a retry can't
                // succeed where the first attempt failed to deserialize.
                error!(event_type = %event.event_type, "unparseable customer payload");
                Ok(())
            }
        },
        "subscription.created" | "subscription.updated" | "subscription.canceled" => {
            match event.subscription() {
                Some(sub) => sync::upsert_subscription(conn, &sub, api).await,
                None => {
                    error!(event_type = %event.event_type, "unparseable subscription payload");
                    Ok(())
                }
            }
        }
        "transaction.completed" => {
            // Payment observed. The subscription events already mirror state;
            // revenue/provisioning hooks will hang off this arm later.
            info!(event_id = %event.event_id, "transaction completed");
            Ok(())
        }
        other => {
            warn!(event_type = other, "subscribed but unhandled event type");
            Ok(())
        }
    }
}
