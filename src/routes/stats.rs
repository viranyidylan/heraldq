//! Read-only reporting: delivery history and today's activity counters.

use axum::{
    Json, Router,
    extract::{Query, State},
    response::Html,
    routing::get,
};
use serde::Serialize;

use crate::{
    AppState, auth,
    db::stats::WebhookActivitySummary,
    error::AppError,
    routes::{Pagination, clamp_pagination},
};

pub(super) fn routes() -> Router<AppState> {
    Router::new()
        .route("/deliveries", get(deliveries))
        .route("/summary", get(webhook_activity_summary))
        .route("/dashboard", get(dashboard))
}

pub(super) async fn webhook_activity_summary(
    State(state): State<AppState>,
    api_key: auth::ValidApiKey,
) -> Result<Json<WebhookActivitySummary>, AppError> {
    let summary = crate::db::stats::activity_summary(&state.pg_pool, api_key.tenant_id).await?;

    Ok(Json(summary))
}

pub(super) async fn dashboard(
    State(state): State<AppState>,
    api_key: auth::ValidApiKey,
) -> Result<Html<String>, AppError> {
    let counts = crate::db::stats::activity_summary(&state.pg_pool, api_key.tenant_id).await?;

    Ok(Html(crate::views::render_dashboard(&counts)))
}

#[derive(Serialize)]
pub(super) struct DeliveriesResponse {
    deliveries: Vec<crate::db::deliveries::DeliveryRecord>,
    total: i64,
    limit: i64,
    offset: i64,
}

pub(super) async fn deliveries(
    State(state): State<AppState>,
    api_key: auth::ValidApiKey,
    Query(pagination): Query<Pagination>,
) -> Result<Json<DeliveriesResponse>, AppError> {
    let tenant_id = api_key.tenant_id;

    let limit = clamp_pagination(pagination.limit, 25, 100);
    let offset = clamp_pagination(pagination.offset, 0, i64::MAX);

    let (deliveries, total) = tokio::try_join!(
        crate::db::deliveries::list_page(&state.pg_pool, tenant_id, limit, offset),
        crate::db::deliveries::count_total(&state.pg_pool, tenant_id),
    )?;

    Ok(Json(DeliveriesResponse {
        deliveries,
        total,
        limit,
        offset,
    }))
}
