// use chrono::Utc;
// use graphile_worker::{IntoTaskHandlerResult, TaskHandler, WorkerContext};
// use hmac::{Hmac, Mac};
// use reqwest::{Client, Response};
// use sha2::Sha256;
// use tracing::{error, info, warn};

// use crate::{WorkerState, db};

// use super::SendWebhook;

// type HmacSha256 = Hmac<Sha256>;

// #[derive(Debug, thiserror::Error)]
// pub enum DispatchError {
//     #[error("dispatch error: {0}")]
//     Default(&'static str),
//     #[error("missing worker extension: {0}")]
//     MissingExtension(&'static str),
// }

// #[derive(Debug)]
// pub enum Outcome {
//     Success,
//     PermanentFailure { reason: FailureReason, status_code: u16 },
//     Retry { reason: RetryReason },
// }

// #[derive(Debug)]
// pub enum FailureReason {
//     Redirect,
//     ClientError,
//     Other(String)
// }

// #[derive(Debug)]
// pub enum RetryReason {
//     HttpStatus(u16),
//     RateLimit,
//     Timeout,
//     Connect,
//     Other(String)
// }

// async fn dispatch_http(
//     http: &Client,
//     webhook: &SendWebhook,
//     body: Vec<u8>,
//     signature_header: String,
// ) -> Outcome {
//     match http
//         .post(&webhook.destination)
//         .header("Content-Type", "application/json")
//         .header("Idempotency-Key", &webhook.id)
//         .header("HeraldQ-Signature", signature_header)
//         .body(body)
//         .send()
//     .await {
//         Ok(r) => {
//             let status = r.status().as_u16();
//             match status {
//                 408 | 425 | 429 => Outcome::Retry {
//                     reason: RetryReason::HttpStatus(status),
//                 },
//                 500..=599 => Outcome::Retry {
//                     reason: RetryReason::HttpStatus(status),
//                 },
//                 200..=299 => Outcome::Success,
//                 _ => Outcome::PermanentFailure { reason: FailureReason::Other(r.status().to_string()), status_code: status },
//             }
//         },
//         Err(e) if e.is_timeout() => Outcome::Retry { reason: RetryReason::Timeout },
//         Err(e) if e.is_connect() => Outcome::Retry { reason: RetryReason::Connect },
//         Err(e) => Outcome::Retry { reason: RetryReason::Other(e.to_string()) },
//     }
// }

// impl TaskHandler for SendWebhook {
//     const IDENTIFIER: &'static str = "send_webhook";

//     async fn run(self, ctx: WorkerContext) -> impl IntoTaskHandlerResult {
//     let app_state = ctx
//             .get_ext::<WorkerState>()
//             .ok_or(DispatchError::MissingExtension("Worker State"))?;

//         let http_client = &app_state.http_client;   
//         let pg_pool = &app_state.pg_pool;

//         let job = ctx.job();
//         let job_id = job.id();
//         let attempts = job.attempts();
//         let max_attempts = job.max_attempts();

//         info!(
//             webhook_id = %self.id,
//             destination = %self.destination,
//             attempt = attempts,
//             max_attempts,
//             worker_id = %ctx.worker_id(),
//             "Dispatching webhook"
//         );

//         let timestamp_utc = Utc::now().timestamp().to_string();
//         let body_as_bytes = serde_json::to_vec(&self.payload)
//             .expect("payload serialization should not fail");

//         let secret = db::tenants::get_hmac_secret(&app_state.pg_pool, self.tenant_id).await?;
//         let hex_sig = sign_bytes(secret.as_bytes(), &timestamp_utc, &body_as_bytes);
//         let signature_header = format!("t={},v1={}", timestamp_utc, hex_sig);

//         let outcome = dispatch_http(
//             http_client, &self, body_as_bytes, signature_header).await;

//         match outcome {
//             Outcome::Success => {
//                 // save delivery
//                 info!("Webhook delivered successfully");
//                 Ok(())
//             }
//             Outcome::Retry {reason} => {
//                 // if attempts have been exhausted
//                 if attempts >= max_attempts {
//                     // DLQ insert + mark permanently failed
//                     let reason = e.failure_reason();
//                     let failure_reason = reason.map(|r| r.to_string());
//                     let status_code = reason.and_then(|r| r.status_code());

//                     let _ = db::dlq::insert(
//                         pg_pool, self.clone(), job.clone(), 
//                         failure_reason, status_code
//                     ).await;
//                     Ok(())
//                 } else {
//                     // graphile-worker will reschedule
//                 }
//                 // retry
//                 warn!("Retryable failure, scheduling retry");
//                 Err("DispatchError")
//             }
//             Outcome::PermanentFailure {reason,status_code} => {
//                 warn!("Permanent failure, dead-lettering");
//                 let _ = db::dlq::insert(
//                     pg_pool, self.clone(), job.clone(), 
//                     Some(reason.to_string()), status_code
//                 ).await;

//                 ctx.utils()
//                     .permanently_fail_jobs(&[*job_id], &format!("permanent: {}", reason))
//                     .await?;
//                 Ok(())
//             }
//         };

//         // If all attempts have been exhausted
//         if outcome == Outcome::Retry {reason: any()}
//             && attempts >= max_attempts
//         {
//             let reason = e.failure_reason();
//             let failure_reason = reason.map(|r| r.to_string());
//             let status_code = reason.and_then(|r| r.status_code());

//             let _ = db::dlq::insert(
//                 pg_pool, self.clone(), job.clone(), 
//                 failure_reason, status_code
//             ).await;

//             error!(
//                 job_id = %job_id,
//                 destination = %self.destination,
//                 attempt = attempts,
//                 max_attempts,
//                 error = %e,
//                 "Final attempt exhausted, job moved to DLQ"
//             );
//         }
//         res
//     }
// }

// fn sign_bytes(secret: &[u8], timestamp_utc: &str, body_as_bytes: &[u8]) -> String {
//     let mut mac = HmacSha256::new_from_slice(secret)
//         .expect("HMAC can take key of any size");
//     mac.update(timestamp_utc.as_bytes());
//     mac.update(b".");
//     mac.update(body_as_bytes);
//     hex::encode(mac.finalize().into_bytes())
// }
