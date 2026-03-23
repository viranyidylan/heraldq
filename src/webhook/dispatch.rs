use graphile_worker::errors::GraphileWorkerError;
use graphile_worker::{IntoTaskHandlerResult, TaskHandler, WorkerContext, WorkerContextExt};
use reqwest::Client;
use sqlx::PgPool;
use std::time::Duration;

use crate::{WorkerState, db};

use super::SendWebhook;

#[derive(Debug, thiserror::Error)]
pub enum DispatchError {
    #[error("http request failed: {0}")]
    Network(#[from] reqwest::Error),
    #[error("server error: {0}")]
    ServerError(reqwest::StatusCode),
    #[error("client error: {0}")]
    ClientError(reqwest::StatusCode),
    #[error("unexpected status: {0}")]
    UnexpectedStatus(reqwest::StatusCode),
    #[error("failed to permanently fail job: {0}")]
    WorkerError(#[from] GraphileWorkerError),
    #[error("missing worker extension: {0}")]
    MissingExtension(&'static str),
}

impl TaskHandler for SendWebhook {
    const IDENTIFIER: &'static str = "send_webhook";

    async fn run(self, ctx: WorkerContext) -> impl IntoTaskHandlerResult {
        let app_state = ctx
            .get_ext::<WorkerState>()
            .ok_or(DispatchError::MissingExtension("Worker State"))
            .unwrap();

        let http_client = &app_state.http_client;
        let pg_pool = &app_state.pg_pool;

        let job = ctx.job();
        let job_id = job.id();
        let attempts = job.attempts();
        let max_attempts = job.max_attempts();
        let run_at = job.run_at();

        println!(
            "[worker] {} Dispatching webhook {} to {} | attempt {}/{} | run_at: {}",
            ctx.worker_id(),
            self.id,
            self.destination,
            attempts,
            max_attempts,
            run_at
        );

        let res = try_dispatch_webhook(self.clone(), ctx.clone(), http_client, pg_pool).await;

        if let Err(ref e) = res {
            if attempts >= max_attempts {

                // TODO: write to DLQ table in postgres
                db::dlq::insert(pg_pool, self.clone(), job.clone()).await;

                eprintln!(
                    "[worker] [DLQ] FINAL ATTEMPT EXHAUSTED - job {} to {} will NOT be retried. Webhook inserted into DLQ. attempts={}/{} error={}",
                    job_id, self.destination, attempts, max_attempts, e
                );
            }
        }
        res
    }
}

async fn try_dispatch_webhook(
    webhook: SendWebhook,
    ctx: WorkerContext,
    http: &Client,
    pg_pool: &PgPool
) -> Result<(), DispatchError> {
    let job_id = ctx.job().id();

    let response = http
        .post(&webhook.destination)
        .header("Content-Type", "application/json")
        .header("Idempotency-Key", &webhook.id)
        .json(&webhook.payload)
        .timeout(Duration::from_secs(30))
        .send()
        .await?;

    let status = response.status();

    if status.is_success() {
        if let Err(e) = 
            db::deliveries::insert(pg_pool, webhook.clone(), ctx.job().clone()).await 
        {
            eprintln!(">>> [dispatch] FAILURE TO INSERT INTO DELIVERIES TABLE. Error: {}", e);
        };

        println!("[worker] {} Success for job {}", ctx.worker_id(), job_id);
        return Ok(());
    }

    if status.is_server_error() {
        println!(
            "[worker] {} Server error, will retry job: {}",
            ctx.worker_id(),
            job_id
        );
        return Err(DispatchError::ServerError(status));
    }

    if status.is_client_error() {
        println!(
            "[worker] {} Client error, will not retry job: {}",
            ctx.worker_id(),
            job_id
        );

        // TODO: write to DLQ table in postgres
        db::dlq::insert(pg_pool, webhook.clone(), ctx.job().clone()).await;

        ctx.utils()
            .permanently_fail_jobs(&[*job_id], &format!("client error: {}", status))
            .await?;

        return Ok(());
    }

    Err(DispatchError::UnexpectedStatus(status))
}
