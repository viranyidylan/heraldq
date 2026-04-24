pub mod auth;
pub mod config;
pub mod db;
pub mod error;
pub mod queue;
pub mod routes;
pub mod views;
pub mod webhook;

use std::sync::Arc;

use graphile_worker::WorkerUtils;
use reqwest::Client;
use sqlx::PgPool;
// use governor::DefaultKeyedRateLimiter<K>;
use governor::DefaultDirectRateLimiter;
pub use webhook::{SendWebhook, WebhookRequest};

#[derive(Clone)]
pub struct AppState {
    pub pg_pool: PgPool,
    pub worker_utils: WorkerUtils,
    pub schema: String,
    pub webhook_limiter: Arc<DefaultDirectRateLimiter>,
}

#[derive(Clone, Debug)]
pub struct WorkerState {
    pub http_client: Client,
    pub pg_pool: PgPool,
}
