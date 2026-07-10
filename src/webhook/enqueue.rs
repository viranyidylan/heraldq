use graphile_worker::{JobSpecBuilder, WorkerUtils, errors::GraphileWorkerError};
use ulid::Ulid;
use uuid::Uuid;

use crate::{SendWebhook, WebhookRequest};

pub async fn enqueue(
    utils: &WorkerUtils,
    tenant_id: Uuid,
    request_body: WebhookRequest,
) -> Result<String, GraphileWorkerError> {
    let webhook_id = Ulid::new().to_string();
    enqueue_with_id(utils, webhook_id.clone(), tenant_id, request_body).await?;
    Ok(webhook_id)
}

/// Enqueue under a caller-supplied webhook id. Used by DLQ replay, which
/// mints the id up front so it can atomically claim the DLQ row with it
/// before the job exists.
pub async fn enqueue_with_id(
    utils: &WorkerUtils,
    webhook_id: String,
    tenant_id: Uuid,
    request_body: WebhookRequest,
) -> Result<(), GraphileWorkerError> {
    utils
        .add_job(
            SendWebhook {
                id: webhook_id.to_owned(),
                tenant_id,
                destination: request_body.destination.to_string(),
                payload: request_body.payload,
            },
            JobSpecBuilder::new()
                // The webhook ULID doubles as graphile's job key, which is
                // backed by a unique index — status lookups find pending jobs
                // by key instead of scanning every job's payload. ULIDs are
                // unique, so the key's replace-on-collision semantics never
                // trigger.
                .job_key(webhook_id.clone())
                // graphile backoff is exp(least(attempts, 10)) seconds, i.e.
                // e^n capped at e^10 ≈ 6.1h: ~2.7s, 7.4s, 20s, 55s, 2.5m,
                // 6.7m, 18m, 50m, 2.25h, then 6.1h between each remaining
                // attempt. 15 attempts span ~34h total before dead-lettering.
                // Documented for customers in docs/delivery-behavior.md —
                // change these together.
                .max_attempts(15)
                .build(),
        )
        .await?;
    Ok(())
}
