use chrono::Utc;
use graphile_worker::{IntoTaskHandlerResult, TaskHandler, WorkerContext};
use hmac::{Hmac, Mac};
use sha2::Sha256;
type HmacSha256 = Hmac<Sha256>;
use tracing::{error, info, warn};

use crate::{db, SendWebhook, WorkerState};

#[derive(Debug)]
enum Outcome {
    Success,
    Retry { reason: String, status: Option<u16> },
    PermaFail { reason: String, status: Option<u16> },
}

#[derive(Debug, thiserror::Error)]
pub enum DispatchError {
    #[error("worker state missing from context")]
    MissingState,

    #[error("retryable error")]
    Retryable{ reason: String, status: Option<u16> },

    #[error("http error")]
    Http(#[from] reqwest::Error),

    #[error("database error")]
    Database(#[from] sqlx::Error),
}

impl TaskHandler for SendWebhook {
    const IDENTIFIER: &'static str = "send_webhook";

    async fn run(self, ctx: WorkerContext) -> impl IntoTaskHandlerResult {
        let app_state = ctx
            .get_ext::<WorkerState>()
            .ok_or(DispatchError::MissingState)?;

        let job = ctx.job();
        let job_id = job.id();
        let attempts = job.attempts();

        info!(
            webhook_id = %self.id,
            job_id,
            destination = %self.destination,
            attempt = attempts,
            worker_id = %ctx.worker_id(),
            "Dispatching webhook"
        );

        let http_client = &app_state.http_client;
        let pg_pool = &app_state.pg_pool;

        let body_as_bytes = serde_json::to_vec(&self.payload)
            .expect("payload serialization should not fail");

        let timestamp_utc = Utc::now().timestamp().to_string();
        let secret = db::tenants::get_hmac_secret(pg_pool, self.tenant_id).await?;
        let hex_sig = sign_bytes(secret.as_bytes(), &timestamp_utc, &body_as_bytes);
        let signature_header = format!("t={},v1={}", timestamp_utc, hex_sig);
        
        let res = http_client
            .post(&self.destination)
            .header("Content-Type", "application/json")
            .header("Idempotency-Key", &self.id)
            .header("HeraldQ-Signature", signature_header)
            .body(body_as_bytes)
            .send()
            .await;

        let outcome = match res {
            Ok(r) => {
                let status = r.status().as_u16();
                match status {
                    408 | 425 | 429 => Outcome::Retry { reason: r.status().to_string(), status: Some(status) }, // retry
                    500..=599       => Outcome::Retry { reason: r.status().to_string(), status: Some(status) }, // retry
                    200..=299       => Outcome::Success, // success
                    _               => Outcome::PermaFail { reason: r.status().to_string(), status: Some(status) } // perma-fail
                }
            },
            Err(e) if e.is_builder() => Outcome::PermaFail { reason: "reqwest builder error (check for bad URL, bad config).".to_string(), status: None },
            Err(e) if e.is_connect() => Outcome::Retry { reason: "reqwest connection error".to_string(), status: None },
            Err(e) if e.is_timeout() => Outcome::Retry { reason: "reqwest timeout error".to_string(), status: None }, // retry
            Err(e)                   => Outcome::Retry { reason: e.to_string(), status: None } // retry
        };

        match outcome {
            Outcome::Success => {
                info!(webhook_id = %self.id, job_id = %job_id, "Webhook delivered successfully");
                db::deliveries::insert(pg_pool, self, job).await?;
                Ok(())
            }
            Outcome::Retry { reason, status } => {
                warn!(webhook_id = %self.id, job_id = %job_id, reason = %reason, "Retryable failure, will retry");
                if attempts >= job.max_attempts() {
                    info!("[TRACER BULLET] Max retries exhausted, adding to dlq");
                    if let Err(db_err) = db::dlq::insert(pg_pool, &self, job, Some("Retries exhausted".to_string()), status.map(|s| s as i16)).await {
                        error!(webhook_id = %self.id,
                            job_id = %job_id, 
                            error = ?db_err,
                            "[CRITICAL ERROR] Failed to write exhausted job to DLQ"
                        );
                        return Err(DispatchError::Database(db_err));
                    };
                }
                Err(DispatchError::Retryable { reason, status })
            }
            Outcome::PermaFail { reason, status } => {
                warn!(webhook_id = %self.id, job_id = %job_id, reason = %reason, "Permanent failure, dead-lettering");
                if let Err(db_err) = db::dlq::insert(pg_pool, &self, job, Some(reason), status.map(|s| s as i16)).await {
                    error!(webhook_id = %self.id,
                        job_id = %job_id, 
                        error = ?db_err,
                        "[CRITICAL ERROR] Failed to write perma-fail job to DLQ"
                    );
                    return Err(DispatchError::Database(db_err));
                }
                Ok(())
            }
        }
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
