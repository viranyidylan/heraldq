//! Minimal Paddle REST API client.
//!
//! Used for exactly one convergence job: when a subscription event arrives
//! for a customer whose mirror row is still an email-less stub (their
//! `customer.created` predates our webhook destination, or was missed),
//! no webhook payload carries the email — so we ask Paddle directly.
//!
//! Failures split by whether a Paddle redelivery could fix them:
//! - `Transient` (network, 5xx) propagates → webhook returns 500 → Paddle retries.
//! - `Permanent` (bad key, unknown customer, unparseable) is logged and
//!   skipped — retrying the same event can't help, and the stub keeps
//!   converging on any later customer/subscription event.

use serde::Deserialize;

use crate::paddle::events::CustomerPayload;

#[derive(Debug)]
pub enum ApiError {
    Transient(String),
    Permanent(String),
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Transient(m) | Self::Permanent(m) => write!(f, "paddle api: {m}"),
        }
    }
}

#[derive(Clone)]
pub struct PaddleApi {
    http: reqwest::Client,
    base: String,
    key: String,
}

#[derive(Deserialize)]
struct CustomerEnvelope {
    data: CustomerResponse,
}

#[derive(Deserialize)]
struct CustomerResponse {
    id: String,
    email: String,
}

impl PaddleApi {
    pub fn new(key: String, base: String) -> Self {
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(3))
            .build()
            .expect("reqwest client builder has valid defaults");
        Self { http, base, key }
    }

    /// `GET /customers/{id}` — returns just the fields the mirror needs.
    pub async fn get_customer(&self, customer_id: &str) -> Result<CustomerPayload, ApiError> {
        let url = format!("{}/customers/{}", self.base, customer_id);
        let resp = self
            .http
            .get(&url)
            .bearer_auth(&self.key)
            .send()
            .await
            .map_err(|e| ApiError::Transient(e.to_string()))?;

        match resp.status() {
            ok if ok.is_success() => resp
                .json::<CustomerEnvelope>()
                .await
                .map(|env| CustomerPayload {
                    id: env.data.id,
                    email: env.data.email,
                })
                .map_err(|e| ApiError::Permanent(format!("unparseable customer: {e}"))),
            status if status.as_u16() == 401 || status.as_u16() == 403 => {
                Err(ApiError::Permanent(format!("api key rejected ({status})")))
            }
            status if status.as_u16() == 404 => Err(ApiError::Permanent(format!(
                "unknown customer {customer_id}"
            ))),
            status => Err(ApiError::Transient(format!("paddle returned {status}"))),
        }
    }
}
