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
    utils
        .add_job(
            SendWebhook {
                id: webhook_id.to_owned(),
                tenant_id,
                destination: request_body.destination.to_string(),
                payload: request_body.payload,
            },
            JobSpecBuilder::new()
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
    Ok(webhook_id)
}
