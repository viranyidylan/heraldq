use graphile_worker::{
    IntoTaskHandlerResult, JobSpecBuilder, TaskHandler, WorkerContext, WorkerContextExt,
};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{f32::consts::E, result, time::Duration};
use ulid::Ulid;

pub mod auth;
pub mod config;
pub mod error;

pub const SCHEMA: &str = "graphile_worker";

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
        println!(
            "[worker] {} Dispatching webhook {} to {} with subject '{}'",
            _ctx.worker_id(),
            self.id,
            self.destination,
            self.payload
        );

        let run_at = _ctx.job().run_at();
        let retry_count = _ctx.job().attempts();
        println!(
            "[worker] {} run at: {}, retry_count '{}'",
            _ctx.worker_id(),
            run_at,
            retry_count
        );

        let attempts = _ctx.job().attempts();
        let max_attempts = _ctx.job().max_attempts();
        let job_id = _ctx.job().id();

        let res = try_dispatch_webhook(self.clone(), _ctx.clone()).await;

        if let Err(ref e) = res {
            if attempts >= max_attempts {
                // TODO: write to DLQ table in postgres
                eprintln!(
                    "[worker] [DLQ] FINAL ATTEMPT EXHAUSTED - job {} to {} will NOT be retried. attempts={}/{} error={}",
                    job_id, self.destination, attempts, max_attempts, e
                );
            }
        }
        res
    }
}

async fn try_dispatch_webhook(job: SendWebhook, _ctx: WorkerContext) -> Result<(), String> {
    let http: Client = reqwest::Client::new();
    let job_id = _ctx.job().id();

    let http_result = http
        .post(&job.destination)
        .header("Content-Type", "application/json")
        .header("Idempotency-Key", &job.id)
        .json(&job.payload)
        .timeout(Duration::from_secs(30))
        .send()
        .await;

    let response = http_result.map_err(|e| e.to_string())?;

    let status = response.status();

    if status.is_success() {
        println!("[worker] {} Success for job {}'", _ctx.worker_id(), job_id);
        return Ok(());
    }

    if status.is_server_error() {
        println!(
            "[worker] {} Server error, will retry job: {}",
            _ctx.worker_id(),
            job_id
        );
        return Err(format!("server error: {}", status));
    }

    if status.is_client_error() {
        println!(
            "[worker] {} Client error, will not retry job: '{}'",
            _ctx.worker_id(),
            job_id
        );

        // TODO: write to DLQ table in postgres

        _ctx.utils()
            .permanently_fail_jobs(&[*job_id], &format!("client error: {}", status))
            .await
            .map_err(|e| e.to_string())?;

        return Ok(());
    }

    // default: treat unknown as retryable
    Err(format!(
        "[worker] {} unexpected status: {} for job: {}",
        _ctx.worker_id(),
        status,
        job_id
    ))
}
