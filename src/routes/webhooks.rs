//! The product: enqueue a webhook, and ask what happened to it.

use axum::{
    Json, Router,
    extract::{self, Path, State},
    routing::{get, post},
};
use chrono::Utc;
use rand::RngExt;
use reqwest::StatusCode;
use serde::Serialize;
use tracing::{debug, info};

use crate::{
    AppState, WebhookRequest, auth,
    error::AppError,
    webhook::{self, ssrf::validate_destination},
};

pub(super) fn routes() -> Router<AppState> {
    Router::new()
        .route("/webhook", post(enqueue_webhook))
        .route("/webhook/{id}", get(webhook_status))
        .route("/mock", post(mock))
}

#[derive(Serialize)]
pub(super) struct EnqueueResponse {
    id: String,
}

pub(super) async fn enqueue_webhook(
    State(state): State<AppState>,
    api_key: auth::ValidApiKey,
    extract::Json(request_body): extract::Json<WebhookRequest>,
) -> Result<(StatusCode, Json<EnqueueResponse>), AppError> {
    info!(tenant_id = %api_key.tenant_id, tier = %api_key.tier, "Webhook enqueue requested");

    // Intake funnel: every exit from this handler counts its outcome. The
    // 4xx status codes are already in heraldq_http_requests_total, but as
    // raw slices — outcome labels make "why are we rejecting" answerable
    // in one query (and quotable in the dashboard) without re-deriving it.
    if state
        .tier_rate_limits
        .for_tier(&api_key.tier)
        .check_key(&api_key.tenant_id)
        .is_err()
    {
        metrics::counter!("heraldq_webhooks_enqueued_total", "outcome" => "rate_limited")
            .increment(1);
        return Err(AppError::TooManyRequests);
    };

    if let Err(e) =
        validate_destination(&state.policy, &request_body.destination).map_err(AppError::from)
    {
        metrics::counter!("heraldq_webhooks_enqueued_total", "outcome" => "invalid_destination")
            .increment(1);
        return Err(e);
    }

    // Hard monthly cap, atomically counted in the DB, sized by the tenant's
    // tier. Only valid requests that will actually enqueue should burn a
    // quota slot.
    if !crate::db::quotas::increment_monthly_quota(
        &state.pg_pool,
        api_key.tenant_id,
        crate::tiers::effective_monthly_limit(
            &api_key.tier,
            api_key.monthly_webhook_limit_override,
        ),
    )
    .await?
    {
        metrics::counter!("heraldq_webhooks_enqueued_total", "outcome" => "quota_exceeded")
            .increment(1);
        return Err(AppError::MonthlyQuotaExceeded);
    }

    let webhook_id = webhook::enqueue(&state.worker_utils, api_key.tenant_id, request_body).await?;

    metrics::counter!("heraldq_webhooks_enqueued_total", "outcome" => "enqueued").increment(1);

    Ok((
        StatusCode::ACCEPTED,
        Json(EnqueueResponse { id: webhook_id }),
    ))
}

#[derive(Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub(super) enum WebhookStatusResponse {
    Delivered {
        id: String,
        destination: String,
        status_code: Option<i16>,
        attempts: Option<i16>,
        enqueued_at: chrono::DateTime<Utc>,
        delivered_at: chrono::DateTime<Utc>,
    },
    Failed {
        id: String,
        destination: String,
        failure_reason: Option<String>,
        status_code: Option<i16>,
        attempts: Option<i16>,
        enqueued_at: chrono::DateTime<Utc>,
        abandoned_at: chrono::DateTime<Utc>,
    },
    Pending {
        id: String,
        destination: String,
        attempts: i16,
        max_attempts: i16,
        last_error: Option<String>,
        enqueued_at: chrono::DateTime<Utc>,
        next_attempt_at: chrono::DateTime<Utc>,
    },
}

impl From<crate::db::deliveries::DeliveryRecord> for WebhookStatusResponse {
    fn from(d: crate::db::deliveries::DeliveryRecord) -> Self {
        Self::Delivered {
            id: d.id,
            destination: d.destination,
            status_code: d.status_code,
            attempts: d.attempts,
            enqueued_at: d.enqueued_at,
            delivered_at: d.delivered_at,
        }
    }
}

impl From<crate::db::dlq::DlqEntry> for WebhookStatusResponse {
    fn from(f: crate::db::dlq::DlqEntry) -> Self {
        Self::Failed {
            id: f.id,
            destination: f.destination,
            failure_reason: f.failure_reason,
            status_code: f.status_code,
            attempts: f.attempts,
            enqueued_at: f.enqueued_at,
            abandoned_at: f.abandoned_at,
        }
    }
}

pub(super) async fn webhook_status(
    State(state): State<AppState>,
    api_key: auth::ValidApiKey,
    Path(webhook_id): Path<String>,
) -> Result<Json<WebhookStatusResponse>, AppError> {
    let pool = &state.pg_pool;
    let tenant_id = api_key.tenant_id;

    // Terminal states first — once written they never change.
    if let Some(d) = crate::db::deliveries::find_by_id(pool, tenant_id, &webhook_id).await? {
        return Ok(Json(d.into()));
    }
    if let Some(f) = crate::db::dlq::find_by_id(pool, tenant_id, &webhook_id).await? {
        return Ok(Json(f.into()));
    }
    if let Some(p) = crate::db::jobs::find_by_webhook_id(pool, tenant_id, &webhook_id).await? {
        return Ok(Json(WebhookStatusResponse::Pending {
            id: webhook_id,
            destination: p.destination,
            attempts: p.attempts,
            max_attempts: p.max_attempts,
            last_error: p.last_error,
            enqueued_at: p.created_at,
            next_attempt_at: p.run_at,
        }));
    }

    // The job may have reached a terminal state between the checks above and
    // the queue lookup (the terminal row is committed before the job row is
    // deleted) — re-check the terminal stores once before declaring 404.
    if let Some(d) = crate::db::deliveries::find_by_id(pool, tenant_id, &webhook_id).await? {
        return Ok(Json(d.into()));
    }
    if let Some(f) = crate::db::dlq::find_by_id(pool, tenant_id, &webhook_id).await? {
        return Ok(Json(f.into()));
    }

    Err(AppError::NotFound)
}

pub(super) async fn mock(State(_state): State<AppState>) -> StatusCode {
    let roll = rand::rng().random_range(0..100);

    let status = if roll < 5 {
        StatusCode::TOO_MANY_REQUESTS
    } else if roll < 10 {
        StatusCode::IM_A_TEAPOT
    } else if roll < 20 {
        StatusCode::REQUEST_TIMEOUT
    } else if roll < 30 {
        StatusCode::TOO_EARLY
    } else if roll < 40 {
        StatusCode::PERMANENT_REDIRECT
    } else if roll < 85 {
        StatusCode::INTERNAL_SERVER_ERROR
    } else {
        StatusCode::OK
    };

    debug!(roll, status = status.as_u16(), "Mock endpoint hit");
    status
}

#[cfg(test)]
mod tests {
    use super::*;

    // The `status` tag and snake_case variant names are the public API
    // contract for GET /webhook/{id} — a rename here breaks customers.
    #[test]
    fn webhook_status_serializes_with_status_tag() {
        let now = Utc::now();
        let pending = WebhookStatusResponse::Pending {
            id: "01TEST".into(),
            destination: "https://example.com".into(),
            attempts: 3,
            max_attempts: 15,
            last_error: Some("500 Internal Server Error".into()),
            enqueued_at: now,
            next_attempt_at: now,
        };
        let json = serde_json::to_value(&pending).unwrap();
        assert_eq!(json["status"], "pending");
        assert_eq!(json["attempts"], 3);
        assert_eq!(json["last_error"], "500 Internal Server Error");
    }
}
