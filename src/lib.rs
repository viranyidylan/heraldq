pub mod auth;
pub mod config;
pub mod db;
pub mod error;
pub mod paddle;
pub mod routes;
pub mod telemetry;
pub mod tiers;
pub mod views;
pub mod webhook;

use std::sync::Arc;

use governor::{DefaultDirectRateLimiter, DefaultKeyedRateLimiter, Quota, RateLimiter};
use graphile_worker::WorkerUtils;
use http_acl::HttpAcl;
use reqwest::Client;
use sqlx::PgPool;
use uuid::Uuid;
pub use webhook::{SendWebhook, WebhookRequest};

/// Per-tier keyed rate limiters for the enqueue hot path. One bucket per
/// tier because governor keys share a quota per limiter; tenants are keyed
/// within their tier's limiter, so a `free` tenant can never consume a
/// `professional` tenant's budget (or vice versa).
#[derive(Clone)]
pub struct TierRateLimits {
    free: Arc<DefaultKeyedRateLimiter<Uuid>>,
    basic: Arc<DefaultKeyedRateLimiter<Uuid>>,
    professional: Arc<DefaultKeyedRateLimiter<Uuid>>,
    enterprise: Arc<DefaultKeyedRateLimiter<Uuid>>,
}

impl TierRateLimits {
    /// Build the four buckets from the `tiers` map — the single source of
    /// truth for what each tier's rate limit is.
    pub fn new() -> Self {
        let build = |tier: &str| {
            let per_second = std::num::NonZeroU32::new(crate::tiers::rate_limit_per_second(tier))
                .expect("tiers map returns nonzero rates");
            Arc::new(RateLimiter::keyed(Quota::per_second(per_second)))
        };
        Self {
            free: build("free"),
            basic: build("basic"),
            professional: build("professional"),
            enterprise: build("enterprise"),
        }
    }

    /// The limiter bucket a tenant of `tier` belongs to. Unknown tiers land
    /// in the free bucket (lowest limit) — consistent with `tiers`.
    pub fn for_tier(&self, tier: &str) -> &Arc<DefaultKeyedRateLimiter<Uuid>> {
        match tier {
            "basic" => &self.basic,
            "professional" => &self.professional,
            "enterprise" => &self.enterprise,
            _ => &self.free,
        }
    }
}

impl Default for TierRateLimits {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone)]
pub struct AppState {
    pub pg_pool: PgPool,
    pub worker_utils: WorkerUtils,
    pub schema: String,
    pub tier_rate_limits: TierRateLimits,
    // Global throttle on the unauthenticated signup endpoint — caps total
    // invite-code attempts to limit brute-force / mass-signup abuse.
    pub signup_limiter: Arc<DefaultDirectRateLimiter>,
    pub policy: HttpAcl,
    // Secret of the Paddle notification destination, used to verify
    // Paddle-Signature headers. None = endpoint refuses deliveries (503)
    // until configured, so events fail loudly instead of silently.
    pub paddle_webhook_secret: Option<String>,
    // Server-side Paddle API client, for hydrating stub customers during
    // webhook processing. None = hydration deferred to customer events.
    pub paddle_api: Option<crate::paddle::api::PaddleApi>,
    // Browser origins allowed to POST /v1/signup from the marketing site.
    // Scoped to that one route — every other endpoint is API-key or
    // HMAC-authenticated and has no browser callers.
    pub cors_allowed_origins: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct WorkerState {
    pub http_client: Client,
    pub pg_pool: PgPool,
}
