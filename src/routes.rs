use axum::{
    Json, Router, extract::{self, FromRef, Query, State}, response::Html, routing::{get, post}
};
use chrono::{Duration, Utc};
use rand::RngExt;
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;

use tracing::{debug, info};

use crate::{AppState, WebhookRequest, auth, error::AppError, webhook};

impl FromRef<AppState> for PgPool {
    fn from_ref(app_state: &AppState) -> PgPool {
        app_state.pg_pool.clone()
    }
}

#[derive(Deserialize, Serialize)]
struct HealthCheckResponse {
    status: i32,
    message: String,
}

pub fn build(state: AppState) -> Router {
    Router::new()
        .route("/ping", get(ping))
        .route("/health", get(health))
        .route("/webhook", post(add_job))
        .route("/signup", post(signup))
        .route("/mock", post(mock))
        .route("/dlq", get(dlq))
        .route("/queued", get(jobs_queued_today))
        .route("/summary", get(webhook_activity_summary))
        .route("/dashboard", get(dashboard))
        .with_state(state)
}

const TRIAL_DURATION_DAYS: i64 = 7;

async fn ping() -> &'static str {
    "pong"
}

async fn health() -> Json<HealthCheckResponse> {
    Json(HealthCheckResponse {
        status: 200,
        message: "healthy".into(),
    })
}

async fn add_job(
    State(state): State<AppState>,
    api_key: auth::ValidApiKey,
    extract::Json(request_body): extract::Json<WebhookRequest>,
) -> Result<StatusCode, AppError> {
    info!(tenant_id = %api_key.tenant_id, "Webhook enqueue requested");

    webhook::enqueue(&state.worker_utils, api_key.tenant_id, request_body).await?;

    Ok(StatusCode::ACCEPTED)
}

#[derive(Deserialize)]
struct SignupRequest {
    name: String,
}

#[derive(Serialize)]
struct SignupResponse {
    tenant_id: uuid::Uuid,
    api_key: String,
    expires_at: chrono::DateTime<Utc>,
}

async fn signup(
    State(state): State<AppState>,
    extract::Json(body): extract::Json<SignupRequest>,
) -> Result<(StatusCode, Json<SignupResponse>), AppError> {
    let tenant_id = crate::db::tenants::create_tenant(&state.pg_pool, &body.name).await?;

    let raw_key = auth::generate_api_key();
    let key_hash = auth::hash_api_key(&raw_key);
    let expires_at = Utc::now() + Duration::days(TRIAL_DURATION_DAYS);

    crate::db::tenants::create_api_key(&state.pg_pool, tenant_id, &key_hash, "trial", expires_at)
        .await?;

    info!(tenant_id = %tenant_id, "New trial tenant signed up");

    Ok((
        StatusCode::CREATED,
        Json(SignupResponse {
            tenant_id,
            api_key: raw_key,
            expires_at,
        }),
    ))
}

async fn jobs_queued_today(
    State(state): State<AppState>,
    api_key: auth::ValidApiKey,
) -> Result<Json<i64>, AppError> {
    let tenant_id = api_key.tenant_id;
    let count = crate::db::stats::count_jobs_queued_today(&state.pg_pool, &tenant_id).await?;
    Ok(Json(count))
}

#[derive(Deserialize, Serialize)]
struct WebhookActivitySummary {
    received_today: i64,
    queued: i64,
    delivered_today: i64,
    failed_today: i64,
    dlq_total: i64,
    downtime: Option<i64>,
}

// TODO: MASSIVE BUG: returns activity for ALL accounts, not just tenant
async fn webhook_activity_summary(
    State(state): State<AppState>,
    api_key: auth::ValidApiKey,
) -> Result<Json<WebhookActivitySummary>, AppError> {
    let counts = crate::db::stats::webhook_activity_summary(&state.pg_pool, &api_key.tenant_id).await?;

    Ok(Json(WebhookActivitySummary {
        received_today: counts.delivered_today + counts.failed_today + counts.queued,
        queued: counts.queued,
        delivered_today: counts.delivered_today,
        failed_today: counts.failed_today,
        dlq_total: counts.dlq_total,
        downtime: None,
    }))
}

async fn dashboard(
    State(state): State<AppState>,
    api_key: auth::ValidApiKey,
) -> Result<Html<String>, AppError> {
    let counts = crate::db::stats::webhook_activity_summary(&state.pg_pool, &api_key.tenant_id).await?;

    Ok(Html(crate::views::render_dashboard(&counts)))
}

#[derive(Deserialize, Serialize)]
struct DeadLetterResponse {
    messages: Vec<WebhookRequest>,
    dlq_total: i64,
    limit: i64,
    offset: i64,
}

#[derive(Deserialize)]
struct DlqPagination {
    limit: Option<i64>,
    offset: Option<i64>,
}

fn clamp_pagination(value: Option<i64>, default: i64, max: i64) -> i64 {
    value.unwrap_or(default).clamp(0, max)
}

async fn dlq(
    State(state): State<AppState>,
    api_key: auth::ValidApiKey,
    Query(pagination): Query<DlqPagination>,
) -> Result<Json<DeadLetterResponse>, AppError> {
    let tenant_id = api_key.tenant_id;

    let limit = clamp_pagination(pagination.limit, 25, 100);
    let offset = clamp_pagination(pagination.offset, 0, i64::MAX);

    let messages = crate::db::dlq::list_page(&state.pg_pool, &api_key.tenant_id, limit, offset).await?;
    let dlq_total = crate::db::dlq::count_dlq_total(&state.pg_pool, &tenant_id).await?;

    Ok(Json(DeadLetterResponse {
        messages,
        dlq_total,
        limit,
        offset,
    }))
}

async fn mock(State(_state): State<AppState>) -> StatusCode {
    let roll = rand::rng().random_range(0..100);

    let status = if roll < 5 {
        StatusCode::IM_A_TEAPOT
    } else if roll < 85 {
        StatusCode::INTERNAL_SERVER_ERROR
    } else {
        StatusCode::OK
    };

    debug!(roll, status = status.as_u16(), "Mock endpoint hit");
    status
}
