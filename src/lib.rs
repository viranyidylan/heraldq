use std::time::Duration;

use graphile_worker::{IntoTaskHandlerResult, TaskHandler, Worker, WorkerContext};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use ulid::Ulid;
use reqwest::{
    Client
};

#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct WebhookRequest {
    pub destination: String,
    pub payload: Value,
}

// Re-export shared types
#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct SendWebhook {
    pub id: String,
    pub destination: String,
    pub payload: Value,
}

impl TaskHandler for SendWebhook {
    const IDENTIFIER: &'static str = "send_webhook";

    async fn run(self, _ctx: WorkerContext) -> impl IntoTaskHandlerResult {
        println!("[worker] {} Dispatching webhook {} to {} with subject '{}'", _ctx.worker_id(), self.id, self.destination, self.payload);
        try_dispatch_webhook(self, _ctx).await
    }
}

async fn try_dispatch_webhook(job: SendWebhook, _ctx: WorkerContext) -> Result<(), String> {
    let http = reqwest::Client::new();

    let result = http.post(&job.destination)
        .header("Content-Type", "application/json")
        .header("Idempotency-Key", &job.id)
        .json(&job.payload)        
        .timeout(Duration::from_secs(30))
        .send()
        .await;

    match result {
        Ok(response) => Ok(()),
        Err(e) => Err(e.to_string())
    }
}

async fn handle_result() {

}

pub const SCHEMA: &str = "graphile_worker";