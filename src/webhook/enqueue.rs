use graphile_worker::{JobSpecBuilder, WorkerUtils, errors::GraphileWorkerError};
use ulid::Ulid;
use uuid::Uuid;

use crate::{SendWebhook, WebhookRequest, queue::TenantQueue};

pub async fn enqueue(
    utils: &WorkerUtils,
    tenant_id: Uuid,
    request_body: WebhookRequest
) -> Result<(), GraphileWorkerError> {
    let tenant_queue = TenantQueue::for_tenant(tenant_id);
    utils
        .add_job(
            SendWebhook {
                id: Ulid::new().to_string(),
                tenant_id,
                destination: request_body.destination.to_string(),
                payload: request_body.payload,
            },
            JobSpecBuilder::new()
                .queue_name(tenant_queue.as_str())
                .max_attempts(5)
                .build(),
        )
        .await?;
    Ok(())
}