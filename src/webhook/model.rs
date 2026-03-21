use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct WebhookRequest {
    pub destination: String,
    pub payload: Value,
}

#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct SendWebhook {
    pub id: String,
    pub destination: String,
    pub payload: Value,
}
