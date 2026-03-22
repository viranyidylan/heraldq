use graphile_worker::{JobSpecBuilder, WorkerUtils, errors::GraphileWorkerError};
use ulid::Ulid;

use crate::{SendWebhook, WebhookRequest};

pub async fn enqueue(
    utils: &WorkerUtils, 
    request_body: WebhookRequest
) -> Result<(), GraphileWorkerError> {
    utils
        .add_job(
            SendWebhook {
                id: Ulid::new().to_string(),
                destination: request_body.destination.to_string(),
                payload: request_body.payload,
            },
            JobSpecBuilder::new().max_attempts(3).build(),
        )
        .await?;
    Ok(())
}