//! Serde models for Paddle webhook events.
//!
//! Only the fields the sync actually persists are typed; everything else in
//! `data` is ignored by serde. The envelope keeps `data` as raw JSON and each
//! handler tries to narrow it, so an unexpected shape surfaces as a parse
//! error for that handler only (unknown event types still ack cleanly).

use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::Value;

#[derive(Debug, Deserialize)]
pub struct WebhookEvent {
    /// e.g. "subscription.created" — routes the event to a handler.
    #[serde(rename = "event_type")]
    pub event_type: String,
    /// e.g. "evt_01h..." — identical across redeliveries; the dedup key.
    #[serde(rename = "event_id")]
    pub event_id: String,
    #[serde(rename = "occurred_at")]
    pub occurred_at: Option<DateTime<Utc>>,
    pub data: Value,
}

impl WebhookEvent {
    pub fn customer(&self) -> Option<CustomerPayload> {
        serde_json::from_value(self.data.clone()).ok()
    }

    pub fn subscription(&self) -> Option<SubscriptionPayload> {
        serde_json::from_value(self.data.clone()).ok()
    }
}

/// `data` of `customer.created` / `customer.updated`.
#[derive(Debug, Deserialize)]
pub struct CustomerPayload {
    pub id: String,
    pub email: String,
}

/// `data` of `subscription.created` / `.updated` / `.canceled`.
#[derive(Debug, Deserialize)]
pub struct SubscriptionPayload {
    pub id: String,
    pub status: String,
    #[serde(rename = "customer_id")]
    pub customer_id: String,
    #[serde(rename = "scheduled_change")]
    pub scheduled_change: Option<Value>,
    #[serde(rename = "current_billing_period")]
    pub current_billing_period: Option<BillingPeriod>,
    #[serde(rename = "canceled_at")]
    pub canceled_at: Option<DateTime<Utc>>,
    #[serde(rename = "next_billed_at")]
    pub next_billed_at: Option<DateTime<Utc>>,
    pub items: Vec<SubscriptionItem>,
}

#[derive(Debug, Deserialize)]
pub struct BillingPeriod {
    #[serde(rename = "starts_at")]
    pub starts_at: Option<DateTime<Utc>>,
    #[serde(rename = "ends_at")]
    pub ends_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Deserialize)]
pub struct SubscriptionItem {
    pub price: SubscriptionItemPrice,
}

#[derive(Debug, Deserialize)]
pub struct SubscriptionItemPrice {
    pub id: String,
    #[serde(rename = "product_id")]
    pub product_id: String,
    /// Our own plan metadata: `{ "tier": "basic" | "professional" | ... }`,
    /// set per price in the Paddle catalog.
    #[serde(rename = "custom_data")]
    pub custom_data: Option<Value>,
}

impl SubscriptionPayload {
    /// First item's price — heraldQ subscriptions are single-item.
    pub fn primary_price(&self) -> Option<&SubscriptionItemPrice> {
        self.items.first().map(|item| &item.price)
    }

    /// Tier from the price's `custom_data.tier`, if present and a valid
    /// `account_tier` value.
    pub fn tier(&self) -> Option<String> {
        self.primary_price()?
            .custom_data
            .as_ref()?
            .get("tier")?
            .as_str()
            .map(str::to_owned)
    }
}
