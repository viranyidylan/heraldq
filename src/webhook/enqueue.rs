use graphile_worker::{JobSpecBuilder, WorkerUtils, errors::GraphileWorkerError};
use ulid::Ulid;
use uuid::Uuid;

use crate::{SendWebhook, WebhookRequest, queue::TenantQueue};

pub async fn enqueue(
    utils: &WorkerUtils,
    tenant_id: Uuid,
    request_body: WebhookRequest,
) -> Result<String, GraphileWorkerError> {
    let tenant_queue = TenantQueue::for_tenant(tenant_id);
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
                .queue_name(tenant_queue.as_str())
                // 1s, 2s, 4s, 8s, 16s, 32s, 64s, 128s(~2mins)
                // 256s(~4mins), 512s(~8.5mins), 1024s(~17mins)
                // 2048s(~34mins), 4096s(~68mins)
                // 8_192s(~2.25hrs), 16_384s(~4.5hrs)
                .max_attempts(15)
                .build(),
        )
        .await?;
    Ok(webhook_id)
}
