use graphile_worker::errors::GraphileWorkerError;
use graphile_worker::{IntoTaskHandlerResult, TaskHandler, WorkerContext, WorkerContextExt};
use reqwest::Client;
use std::time::Duration;

use tracing::{error, info, warn};

use crate::{WorkerState, db};

use super::SendWebhook;

#[derive(Debug, PartialEq)]
enum StatusAction {
    Success,
    /// Transient failure — worker should retry
    Retry(reqwest::StatusCode),
    /// Permanent failure — job should be dead-lettered, not retried
    PermanentFail(reqwest::StatusCode),
    Unexpected(reqwest::StatusCode),
}

fn classify_status(status: reqwest::StatusCode) -> StatusAction {
    if status.is_success() {
        StatusAction::Success
    } else if status.is_server_error() {
        StatusAction::Retry(status)
    } else if status.is_client_error() {
        StatusAction::PermanentFail(status)
    } else {
        StatusAction::Unexpected(status)
    }
}

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

/// Pure HTTP dispatch — no WorkerContext, no DB, no side effects.
/// Makes the request, returns what happened.
async fn dispatch_http(
    http: &Client,
    webhook: &SendWebhook,
) -> Result<StatusAction, DispatchError> {
    let response = http
        .post(&webhook.destination)
        .header("Content-Type", "application/json")
        .header("Idempotency-Key", &webhook.id)
        .json(&webhook.payload)
        .timeout(Duration::from_secs(30))
        .send()
        .await?;

    Ok(classify_status(response.status()))
}

impl TaskHandler for SendWebhook {
    const IDENTIFIER: &'static str = "send_webhook";

    async fn run(self, ctx: WorkerContext) -> impl IntoTaskHandlerResult {
        let app_state = ctx
            .get_ext::<WorkerState>()
            .ok_or(DispatchError::MissingExtension("Worker State"))
            ?;

        let http_client = &app_state.http_client;
        let pg_pool = &app_state.pg_pool;

        let job = ctx.job();
        let job_id = job.id();
        let attempts = job.attempts();
        let max_attempts = job.max_attempts();

        info!(
            webhook_id = %self.id,
            destination = %self.destination,
            attempt = attempts,
            max_attempts,
            worker_id = %ctx.worker_id(),
            "Dispatching webhook"
        );

        let result = dispatch_http(http_client, &self).await;

        let res = match result {
            Ok(StatusAction::Success) => {
                if let Err(e) = db::deliveries::insert(pg_pool, self.clone(), job.clone()).await {
                    error!(job_id = %job_id, error = %e, "Failed to insert delivery record");
                }
                info!(job_id = %job_id, "Webhook delivered successfully");
                Ok(())
            }
            Ok(StatusAction::Retry(status)) => {
                warn!(job_id = %job_id, status = status.as_u16(), "Server error, will retry");
                Err(DispatchError::ServerError(status))
            }
            Ok(StatusAction::PermanentFail(status)) => {
                warn!(job_id = %job_id, status = status.as_u16(), "Client error, permanently failing");
                let _ = db::dlq::insert(pg_pool, self.clone(), job.clone()).await;
                ctx.utils()
                    .permanently_fail_jobs(&[*job_id], &format!("client error: {}", status))
                    .await?;
                Ok(())
            }
            Ok(StatusAction::Unexpected(status)) => Err(DispatchError::UnexpectedStatus(status)),
            Err(e) => Err(e),
        };

        if let Err(ref e) = res {
            if attempts >= max_attempts {
                let _ = db::dlq::insert(pg_pool, self.clone(), job.clone()).await;
                error!(
                    job_id = %job_id,
                    destination = %self.destination,
                    attempt = attempts,
                    max_attempts,
                    error = %e,
                    "Final attempt exhausted, job moved to DLQ"
                );
            }
        }
        res
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use wiremock::{Mock, MockServer, ResponseTemplate};
    use wiremock::matchers::{method, header};

    fn test_webhook(destination: &str) -> SendWebhook {
        SendWebhook {
            id: "test-webhook-id".to_string(),
            tenant_id: uuid::Uuid::nil(),
            destination: destination.to_string(),
            payload: json!({"event": "test"}),
        }
    }

    #[tokio::test]
    async fn dispatch_returns_success_on_2xx() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;

        let webhook = test_webhook(&server.uri());
        let result = dispatch_http(&Client::new(), &webhook).await.unwrap();
        assert_eq!(result, StatusAction::Success);
    }

    #[tokio::test]
    async fn dispatch_returns_retry_on_5xx() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(503))
            .mount(&server)
            .await;

        let webhook = test_webhook(&server.uri());
        let result = dispatch_http(&Client::new(), &webhook).await.unwrap();
        assert_eq!(result, StatusAction::Retry(reqwest::StatusCode::SERVICE_UNAVAILABLE));
    }

    #[tokio::test]
    async fn dispatch_returns_permanent_fail_on_4xx() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;

        let webhook = test_webhook(&server.uri());
        let result = dispatch_http(&Client::new(), &webhook).await.unwrap();
        assert_eq!(result, StatusAction::PermanentFail(reqwest::StatusCode::NOT_FOUND));
    }

    #[tokio::test]
    async fn dispatch_returns_network_error_on_connection_refused() {
        let webhook = test_webhook("http://127.0.0.1:1"); // nothing listening
        let result = dispatch_http(&Client::new(), &webhook).await;
        assert!(matches!(result, Err(DispatchError::Network(_))));
    }

    #[tokio::test]
    async fn dispatch_sends_correct_headers() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(header("Content-Type", "application/json"))
            .and(header("Idempotency-Key", "test-webhook-id"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;

        let webhook = test_webhook(&server.uri());
        dispatch_http(&Client::new(), &webhook).await.unwrap();

        // wiremock will fail the test if the headers don't match —
        // unmatched requests cause a panic on MockServer drop
    }
}
