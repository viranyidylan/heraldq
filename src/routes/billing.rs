//! Inbound Paddle notifications.
//!
//! Sits outside `/v1`: it authenticates by HMAC signature over the raw
//! body, not by tenant API key, and its caller is Paddle rather than a
//! customer.

use axum::{Router, body::Bytes, extract::State, http::HeaderMap, routing::post};
use chrono::Utc;
use reqwest::StatusCode;
use serde::Serialize;
use tracing::{error, info, warn};

use crate::{AppState, paddle};

pub(super) fn routes() -> Router<AppState> {
    Router::new().route("/webhooks/paddle", post(paddle_webhook))
}

#[derive(Serialize)]
pub(super) struct PaddleWebhookAck {
    received: bool,
    duplicate: bool,
}

/// Receive a Paddle webhook delivery.
///
/// Status codes follow Paddle's contract — only 2xx within 5s counts as
/// delivered, every non-2xx is retried on the same budget — so none of
/// these responses can lose an event that a later attempt could deliver:
///
/// - 503 — no webhook secret configured (deployment issue; loud + retried)
/// - 400 — structurally unusable request (missing signature/body, malformed
///   header, unparseable JSON). Redelivery is byte-identical, so retrying
///   can't help, but it keeps the dashboard's failure log honest.
/// - 401 — HMAC mismatch or stale timestamp. A rotated secret recovers on
///   the next retry after redeploy; a forged request is harmlessly retried.
/// - 500 — transient (DB); the dedup ledger + processing share one
///   transaction, so a rolled-back attempt is retried cleanly.
pub(super) async fn paddle_webhook(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<axum::Json<PaddleWebhookAck>, (StatusCode, &'static str)> {
    use paddle::signature::SignatureError;

    let Some(secret) = state.paddle_webhook_secret.as_deref() else {
        error!("PADDLE_WEBHOOK_SECRET not set; refusing delivery so Paddle retries");
        return Err((
            StatusCode::SERVICE_UNAVAILABLE,
            "webhook secret not configured",
        ));
    };

    let signature = headers
        .get("paddle-signature")
        .and_then(|value| value.to_str().ok())
        .unwrap_or("");
    if signature.is_empty() || body.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "missing signature or body"));
    }

    // Verified against the exact bytes received — never a re-serialization.
    match paddle::signature::verify_signature(signature, &body, secret, Utc::now()) {
        Ok(()) => {}
        Err(SignatureError::MalformedHeader | SignatureError::BadTimestamp) => {
            return Err((StatusCode::BAD_REQUEST, "malformed signature header"));
        }
        Err(e @ (SignatureError::StaleTimestamp | SignatureError::Mismatch)) => {
            warn!(error = %e, "paddle signature verification failed");
            return Err((StatusCode::UNAUTHORIZED, "invalid signature"));
        }
    }

    let event: paddle::events::PaddleEvent = match serde_json::from_slice(&body) {
        Ok(event) => event,
        Err(e) => {
            warn!(error = %e, "paddle webhook body is not a parseable event");
            return Err((StatusCode::BAD_REQUEST, "unparseable event"));
        }
    };

    let mut tx = state.pg_pool.begin().await.map_err(paddle_db_error)?;
    let is_new = paddle::sync::record_event(
        &mut tx,
        &event.event_id,
        &event.event_type,
        event.occurred_at,
    )
    .await
    .map_err(paddle_db_error)?;

    if is_new {
        paddle::process_event(&mut tx, &event, state.paddle_api.as_ref())
            .await
            .map_err(paddle_db_error)?;
    }
    tx.commit().await.map_err(paddle_db_error)?;

    info!(
        event_type = %event.event_type,
        event_id = %event.event_id,
        duplicate = !is_new,
        "paddle webhook accepted"
    );
    metrics::counter!(
        "heraldq_paddle_webhooks_total",
        "outcome" => if is_new { "accepted" } else { "duplicate" }
    )
    .increment(1);

    Ok(axum::Json(PaddleWebhookAck {
        received: true,
        duplicate: !is_new,
    }))
}

fn paddle_db_error(e: sqlx::Error) -> (StatusCode, &'static str) {
    error!(error = ?e, "paddle webhook database error");
    (StatusCode::INTERNAL_SERVER_ERROR, "internal error")
}
