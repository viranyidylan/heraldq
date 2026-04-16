use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::{prelude::FromRow, types::Uuid};

#[derive(Deserialize, Serialize, Clone, Debug, FromRow)]
pub struct WebhookRequest {
    pub destination: String,
    pub payload: Value,
}

#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct SendWebhook {
    pub id: String,
    pub tenant_id: Uuid,
    pub destination: String,
    pub payload: Value,
}

// struct SliqSignature(String);
// impl SliqSignature {
// }
