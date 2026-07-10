use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::types::Uuid;

#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct WebhookRequest {
    pub destination: String,
    pub payload: Value,
}

/// The job payload, serialized into the graphile_worker jobs table.
///
/// This is a PERSISTED WIRE FORMAT, not just a struct: rows written by one
/// deploy are deserialized by the next, so changes must be backwards
/// compatible with jobs already sitting in the queue.
///
/// - Adding a field: it MUST have `#[serde(default)]` (or be `Option`),
///   or every in-flight job fails to deserialize mid-deploy.
/// - Removing or renaming a field: don't. Deprecate and stop reading it.
/// - Unknown fields must stay ignored (serde's default) so a rollback can
///   read jobs written by the newer version — never add
///   `deny_unknown_fields`.
///
/// The `wire_format` tests below pin these rules; update the fixture only
/// when the change is deliberately, verifiably compatible.
#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct SendWebhook {
    pub id: String,
    pub tenant_id: Uuid,
    pub destination: String,
    pub payload: Value,
}

#[cfg(test)]
mod wire_format {
    use super::*;

    /// Byte-for-byte what a v1 job row's payload looks like in the DB.
    /// If this stops deserializing, a deploy will strand every queued job.
    const V1_FIXTURE: &str = r#"{
        "id": "01ARZ3NDEKTSV4RRFFQ69G5FAV",
        "tenant_id": "65403713-d01a-4bdd-ad03-8b5a6a9a3263",
        "destination": "https://example.com/hooks/orders",
        "payload": {"event": "order.created", "amount": 12999}
    }"#;

    #[test]
    fn v1_jobs_still_deserialize() {
        let job: SendWebhook = serde_json::from_str(V1_FIXTURE).expect(
            "stored v1 job payload no longer deserializes — this change \
             would strand every job already in the queue at deploy time",
        );
        assert_eq!(job.id, "01ARZ3NDEKTSV4RRFFQ69G5FAV");
        assert_eq!(job.destination, "https://example.com/hooks/orders");
        assert_eq!(job.payload["event"], "order.created");
    }

    // Rollback safety: an older binary must be able to read job rows written
    // by a newer one that added fields. Breaks if someone adds
    // `deny_unknown_fields`.
    #[test]
    fn unknown_fields_from_newer_versions_are_ignored() {
        let newer = V1_FIXTURE.replacen('{', r#"{"added_in_v2": true,"#, 1);
        let job: SendWebhook = serde_json::from_str(&newer)
            .expect("unknown fields must be ignored for rollback safety");
        assert_eq!(job.id, "01ARZ3NDEKTSV4RRFFQ69G5FAV");
    }
}
