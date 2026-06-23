use chrono::Utc;
use graphile_worker::errors::GraphileWorkerError;
use graphile_worker::{IntoTaskHandlerResult, TaskHandler, WorkerContext, WorkerContextExt};
use hmac::{Hmac, Mac};
use reqwest::Client;
use sha2::Sha256;
use tracing::{error, info, warn};

use crate::{WorkerState, db};

use super::SendWebhook;

type HmacSha256 = Hmac<Sha256>;

#[derive(Debug, PartialEq)]
pub enum FailureReason {
    HttpStatus(reqwest::StatusCode),
    Timeout,
    Connect,
    Network,
}

impl FailureReason {
    fn status_code(&self) -> Option<i16> {
        match self {
            FailureReason::HttpStatus(s) => Some(s.as_u16() as i16),
            _ => None,
        }
    }
}

impl std::fmt::Display for FailureReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FailureReason::HttpStatus(s) => write!(f, "http {}", s),
            FailureReason::Timeout => write!(f, "timeout"),
            FailureReason::Connect => write!(f, "connect"),
            FailureReason::Network => write!(f, "network"),
        }
    }
}

#[derive(Debug, PartialEq)]
enum StatusAction {
    Success,
    /// Transient failure — worker should retry
    Retry(FailureReason),
    /// Permanent failure — job should be dead-lettered, not retried
    PermanentFail(FailureReason),
    Unexpected(FailureReason),
}

fn classify_http_status(status: reqwest::StatusCode) -> StatusAction {
    if status.is_success() {
        StatusAction::Success
    } else if status.is_server_error() {
        StatusAction::Retry(FailureReason::HttpStatus(status))
    } else if status.is_client_error() {
        StatusAction::PermanentFail(FailureReason::HttpStatus(status))
    } else {
        StatusAction::Unexpected(FailureReason::HttpStatus(status))
    }
}

// Consider expanding the granularity of this reqwest error retry strategy
fn classify_reqwest_error(err: &reqwest::Error) -> StatusAction {
    if err.is_timeout() {
        StatusAction::Retry(FailureReason::Timeout)
    } else if err.is_connect() {
        StatusAction::PermanentFail(FailureReason::Connect)
    } else {
        StatusAction::Retry(FailureReason::Network)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum DispatchError {
    #[error("retryable failure: {0}")]
    Retryable(FailureReason),
    #[error("unexpected: {0}")]
    Unexpected(FailureReason),
    #[error("failed to permanently fail job: {0}")]
    WorkerError(#[from] GraphileWorkerError),
    #[error("missing worker extension: {0}")]
    MissingExtension(&'static str),
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),
}

impl DispatchError {
    fn failure_reason(&self) -> Option<&FailureReason> {
        match self {
            DispatchError::Retryable(r) | DispatchError::Unexpected(r) => Some(r),
            DispatchError::WorkerError(_)
            | DispatchError::MissingExtension(_)
            | DispatchError::Database(_) => None,
        }
    }
}

async fn dispatch_http(
    http: &Client,
    webhook: &SendWebhook,
    body: Vec<u8>,
    signature_header: String,
) -> StatusAction {
    match http
        .post(&webhook.destination)
        .header("Content-Type", "application/json")
        .header("Idempotency-Key", &webhook.id)
        .header("Sliq-Signature", signature_header)
        .body(body)
        .send()
        .await
    {
        Ok(response) => classify_http_status(response.status()),
        Err(e) => classify_reqwest_error(&e),
    }
}

impl TaskHandler for SendWebhook {
    const IDENTIFIER: &'static str = "send_webhook";

    async fn run(self, ctx: WorkerContext) -> impl IntoTaskHandlerResult {
        let app_state = ctx
            .get_ext::<WorkerState>()
            .ok_or(DispatchError::MissingExtension("Worker State"))?;

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

        let timestamp_utc = Utc::now().timestamp().to_string();
        let body_as_bytes = serde_json::to_vec(&self.payload)
            .expect("payload serialization should not fail");

    // TODO Support HMAC rotation: dual-secret rotation pattern: 
    //      add hmac_secret_previous column, 
    //      convert get_hmac_secret → get_hmac_secrets returning Vec<String>, 
    //      loop in dispatch to emit v1=<new>,v1=<old>.
        let result: Result<StatusAction, DispatchError> =
            match db::tenants::get_hmac_secret(pg_pool, self.tenant_id).await {
                Ok(secret) => {
                    let hex_sig = sign_bytes(secret.as_bytes(), &timestamp_utc, &body_as_bytes);
                    let signature_header = format!("t={},v1={}", timestamp_utc, hex_sig);
                    Ok(dispatch_http(http_client, &self, body_as_bytes, signature_header).await)
                }
                Err(e) => Err(DispatchError::Database(e)),
            };

        let res = match result {
            Ok(StatusAction::Success) => {
                if let Err(e) = db::deliveries::insert(pg_pool, self.clone(), job.clone()).await {
                    error!(job_id = %job_id, error = %e, "Failed to insert delivery record");
                }
                info!(job_id = %job_id, "Webhook delivered successfully");
                Ok(())
            }
            Ok(StatusAction::Retry(reason)) => {
                warn!(job_id = %job_id, reason = %reason, "Retryable failure, will retry");
                Err(DispatchError::Retryable(reason))
            }
            Ok(StatusAction::PermanentFail(reason)) => {
                warn!(job_id = %job_id, reason = %reason, "Permanent failure, dead-lettering");
                let _ = db::dlq::insert(
                    pg_pool, self.clone(), job.clone(), 
                    Some(reason.to_string()), reason.status_code()
                ).await;
                
                ctx.utils()
                    .permanently_fail_jobs(&[*job_id], &format!("permanent: {}", reason))
                    .await?;
                Ok(())
            }
            Ok(StatusAction::Unexpected(reason)) => Err(DispatchError::Unexpected(reason)),
            Err(e) => Err(e),
        };

        // If all attempts have been exhausted
        if let Err(ref e) = res
            && attempts >= max_attempts
        {
            let reason = e.failure_reason();
            let failure_reason = reason.map(|r| r.to_string());
            let status_code = reason.and_then(|r| r.status_code());

            let _ = db::dlq::insert(
                pg_pool, self.clone(), job.clone(), 
                failure_reason, status_code
            ).await;

            error!(
                job_id = %job_id,
                destination = %self.destination,
                attempt = attempts,
                max_attempts,
                error = %e,
                "Final attempt exhausted, job moved to DLQ"
            );
        }
        res
    }
}

pub fn sign_bytes(secret: &[u8], timestamp_utc: &str, body_as_bytes: &[u8]) -> String {
    let mut mac = HmacSha256::new_from_slice(secret)
        .expect("HMAC can take key of any size");
    mac.update(timestamp_utc.as_bytes());
    mac.update(b".");
    mac.update(body_as_bytes);
    hex::encode(mac.finalize().into_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::time::Duration;
    use wiremock::matchers::{header, method};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    const KEY: &[u8] = b"SECRET_KEY";

    fn test_webhook(destination: &str) -> SendWebhook {
        SendWebhook {
            id: "test-webhook-id".to_string(),
            tenant_id: uuid::Uuid::nil(),
            destination: destination.to_string(),
            payload: json!({"event": "test"}),
        }
    }

    fn sign_for_test(webhook: &SendWebhook) -> (Vec<u8>, String) {
        let ts = Utc::now().timestamp().to_string();
        let body = serde_json::to_vec(&webhook.payload).unwrap();
        let hex_sig = sign_bytes(KEY, &ts, &body);
        (body, format!("t={},v1={}", ts, hex_sig))
    }

    #[tokio::test]
    async fn dispatch_returns_success_on_2xx() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;

        let webhook = test_webhook(&server.uri());
        let (body, sig) = sign_for_test(&webhook);
        let result = dispatch_http(&Client::new(), &webhook, body, sig).await;
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
        let (body, sig) = sign_for_test(&webhook);
        let result = dispatch_http(&Client::new(), &webhook, body, sig).await;
        assert_eq!(
            result,
            StatusAction::Retry(FailureReason::HttpStatus(
                reqwest::StatusCode::SERVICE_UNAVAILABLE
            ))
        );
    }

    #[tokio::test]
    async fn dispatch_returns_permanent_fail_on_4xx() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;

        let webhook = test_webhook(&server.uri());
        let (body, sig) = sign_for_test(&webhook);
        let result = dispatch_http(&Client::new(), &webhook, body, sig).await;
        assert_eq!(
            result,
            StatusAction::PermanentFail(FailureReason::HttpStatus(
                reqwest::StatusCode::NOT_FOUND
            ))
        );
    }

    #[tokio::test]
    async fn dispatch_classifies_connection_refused_as_permanent_fail() {
        let webhook = test_webhook("http://127.0.0.1:1"); // nothing listening

        let (body, sig) = sign_for_test(&webhook);
        let result = dispatch_http(&Client::new(), &webhook, body, sig).await;
        assert_eq!(result, StatusAction::PermanentFail(FailureReason::Connect));
    }

    #[tokio::test]
    async fn dispatch_classifies_timeout_as_retry() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_delay(Duration::from_secs(5)))
            .mount(&server)
            .await;

        let client = Client::builder()
            .timeout(Duration::from_millis(50))
            .build()
            .unwrap();
        let webhook = test_webhook(&server.uri());
        let (body, sig) = sign_for_test(&webhook);
        let result = dispatch_http(&client, &webhook, body, sig).await;
        assert_eq!(result, StatusAction::Retry(FailureReason::Timeout));
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
        let (body, sig) = sign_for_test(&webhook);
        dispatch_http(&Client::new(), &webhook, body, sig).await;

        // wiremock will fail the test if the headers don't match —
        // unmatched requests cause a panic on MockServer drop
    }
}
