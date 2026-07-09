use uuid::Uuid;

const QUEUE_PREFIX: &str = "webhooks:";

pub struct TenantQueue(String);
impl TenantQueue {
    pub fn for_tenant(tenant_id: Uuid) -> Self {
        TenantQueue(format!("{QUEUE_PREFIX}{tenant_id}"))
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
