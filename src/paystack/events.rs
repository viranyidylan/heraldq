//! Serde models for Paystack webhook events.
//!
//! Every payload is `{"event": "<name>", "data": {...}}`. Only fields the
//! sync persists are typed; everything else in `data` is ignored by serde.
//! The envelope keeps `data` as raw JSON and each accessor tries to narrow
//! it, so an unexpected shape surfaces as a `None` for that accessor only
//! (unknown event types still ack cleanly).

use serde::Deserialize;
use serde_json::Value;

#[derive(Debug, Deserialize)]
pub struct PaystackEvent {
    /// e.g. "charge.success", "subscription.create".
    pub event: String,
    pub data: Value,
}

impl PaystackEvent {
    pub fn charge(&self) -> Option<ChargePayload> {
        serde_json::from_value(self.data.clone()).ok()
    }

    pub fn subscription(&self) -> Option<SubscriptionPayload> {
        serde_json::from_value(self.data.clone()).ok()
    }

    pub fn customer(&self) -> Option<CustomerPayload> {
        serde_json::from_value(self.data.clone()).ok()
    }

    /// Synthesize the dedup key. Paystack sends no event id, so identity is
    /// per-entity-per-type: charges by `reference` (unique per payment),
    /// everything else by `id` or the entity's code. `id` is numeric on
    /// some entities (invoices, disputes) and a string on others.
    /// `None` means the payload is too malformed to dedup safely.
    pub fn dedup_key(&self) -> Option<String> {
        let id = self
            .data
            .get("reference")
            .or_else(|| self.data.get("id"))
            .or_else(|| self.data.get("subscription_code"))
            .or_else(|| self.data.get("invoice_code"))
            .and_then(scalar_token)?;
        Some(format!("{}:{}", self.event, id))
    }
}

/// Extract an identity token from a JSON scalar: `data.id` is numeric on
/// some entities (invoices, disputes) and a string on others.
fn scalar_token(value: &Value) -> Option<String> {
    match value {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

/// `data` of `charge.success`.
#[derive(Debug, Deserialize)]
pub struct ChargePayload {
    pub reference: String,
    pub amount: i64,
    pub currency: Option<String>,
    pub customer: CustomerPayload,
    /// Present only when the charge belongs to a subscription; carries the
    /// plan that was charged.
    pub plan: Option<PlanPayload>,
}

/// `data` of `subscription.create` / `.enable` / `.not_renew` / `.disable`.
#[derive(Debug, Deserialize)]
pub struct SubscriptionPayload {
    pub subscription_code: String,
    /// Needed later to cancel programmatically — Paystack requires it.
    pub email_token: Option<String>,
    /// active | non-renewing | cancelled | ...
    pub status: Option<String>,
    pub amount: Option<i64>,
    pub currency: Option<String>,
    pub plan: PlanPayload,
    pub customer: CustomerPayload,
}

#[derive(Debug, Deserialize)]
pub struct PlanPayload {
    pub plan_code: String,
    pub name: Option<String>,
    pub amount: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct CustomerPayload {
    pub customer_code: String,
    pub email: String,
}
