pub mod auth;
pub mod config;
pub mod db;
pub mod error;
pub mod routes;
pub mod webhook;

use graphile_worker::WorkerUtils;
use reqwest::Client;
use sqlx::PgPool;
pub use webhook::{SendWebhook, WebhookRequest};

#[derive(Clone)]
pub struct AppState {
    pub pg_pool: PgPool,
    pub worker_utils: WorkerUtils,
    pub schema: String,
}

#[derive(Clone, Debug)]
pub struct WorkerState {
    pub http_client: Client,
    pub pg_pool: PgPool,
}
