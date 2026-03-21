pub mod auth;
pub mod config;
pub mod db;
pub mod error;
pub mod webhook;

use reqwest::Client;
pub use webhook::{SendWebhook, WebhookRequest};

#[derive(Clone, Debug)]
pub struct WorkerState {
    pub http_client: Client,
}

