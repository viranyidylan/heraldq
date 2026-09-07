//! Dead-letter queue: read what failed, and replay it.

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    routing::{get, post},
};
use reqwest::StatusCode;
use serde::Serialize;
use tracing::info;

use crate::{
    AppState, WebhookRequest, auth,
    error::AppError,
    routes::{Pagination, clamp_pagination},
    webhook::{self, ssrf::validate_destination},
};

pub(super) fn routes() -> Router<AppState> {
    Router::new()
        .route("/dlq", get(list_entries))
        .route("/dlq/{id}/replay", post(replay))
}

#[derive(Serialize)]
pub(super) struct DeadLetterResponse {
    messages: Vec<crate::db::dlq::DlqEntry>,
    dlq_total: i64,
    limit: i64,
    offset: i64,
}

pub(super) async fn list_entries(
    State(state): State<AppState>,
    api_key: auth::ValidApiKey,
    Query(pagination): Query<Pagination>,
) -> Result<Json<DeadLetterResponse>, AppError> {
    let tenant_id = api_key.tenant_id;

    let limit = clamp_pagination(pagination.limit, 25, 100);
    let offset = clamp_pagination(pagination.offset, 0, i64::MAX);

    let messages =
        crate::db::dlq::list_page(&state.pg_pool, &api_key.tenant_id, limit, offset).await?;
    let dlq_total = crate::db::dlq::count_dlq_total(&state.pg_pool, tenant_id).await?;

    Ok(Json(DeadLetterResponse {
        messages,
        dlq_total,
        limit,
        offset,
    }))
}

#[derive(Serialize)]
pub(super) struct ReplayResponse {
    /// The new delivery attempt's webhook id — poll /webhook/{id} with it.
    id: String,
    replayed_from: String,
}

/// Re-enqueue a dead-lettered webhook as a fresh delivery attempt.
///
/// The DLQ row is claimed atomically before the job is created, so a
/// double-click can't produce two deliveries: the second request gets a 409
/// carrying the id the first one minted. The replay gets a NEW webhook id —
/// reusing the original would collide with graphile's job-key row for the
/// exhausted job, and the DLQ row (kept, stamped replayed_at/-as) still
/// records the original.
pub(super) async fn replay(
    State(state): State<AppState>,
    api_key: auth::ValidApiKey,
    Path(webhook_id): Path<String>,
) -> Result<(StatusCode, Json<ReplayResponse>), AppError> {
    let tenant_id = api_key.tenant_id;

    // Same per-tier throttle as fresh enqueues: a replay is an outbound
    // webhook like any other.
    if state
        .tier_rate_limits
        .for_tier(&api_key.tier)
        .check_key(&tenant_id)
        .is_err()
    {
        return Err(AppError::TooManyRequests);
    };

    let new_id = ulid::Ulid::new().to_string();
    let Some(source) =
        crate::db::dlq::claim_for_replay(&state.pg_pool, tenant_id, &webhook_id, &new_id).await?
    else {
        // Claimed by nobody = not this tenant's row; claimed already = 409
        // with the existing replay id so the caller can follow it.
        return match crate::db::dlq::find_by_id(&state.pg_pool, tenant_id, &webhook_id).await? {
            Some(entry) => Err(AppError::AlreadyReplayed {
                replayed_as: entry.replayed_as.unwrap_or_default(),
            }),
            None => Err(AppError::NotFound),
        };
    };

    // The destination passed SSRF policy at original enqueue time, but
    // policy can change between then and a replay months later — re-check,
    // and release the claim on any failure so the tenant can retry.
    let replay = async {
        validate_destination(&state.policy, &source.destination)?;
        webhook::enqueue_with_id(
            &state.worker_utils,
            new_id.clone(),
            tenant_id,
            WebhookRequest {
                destination: source.destination,
                payload: source.payload,
            },
        )
        .await?;
        Ok::<_, AppError>(())
    }
    .await;

    if let Err(e) = replay {
        crate::db::dlq::unclaim_replay(&state.pg_pool, tenant_id, &webhook_id, &new_id).await?;
        return Err(e);
    }

    info!(tenant_id = %tenant_id, replayed_from = %webhook_id, id = %new_id, "DLQ entry replayed");

    Ok((
        StatusCode::ACCEPTED,
        Json(ReplayResponse {
            id: new_id,
            replayed_from: webhook_id,
        }),
    ))
}
