use axum::{
    Json, Router,
    extract::{self, FromRef, Query, State},
    response::Html,
    routing::{get, post},
};
use chrono::{Duration, Utc};
use rand::RngExt;
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;

use tracing::{debug, info};

use crate::{
    AppState, WebhookRequest, auth,
    db::stats::WebhookActivitySummary,
    error::AppError,
    webhook::{self, ssrf::validate_destination},
};

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
    let v1 = Router::new()
        .route("/webhook", post(enqueue_webhook))
        .route("/signup", post(signup))
        .route("/mock", post(mock))
        .route("/dlq", get(dlq))
        .route("/summary", get(webhook_activity_summary))
        .route("/dashboard", get(dashboard));

    Router::new()
        .route("/ping", get(ping))
        .route("/health", get(health))
        .nest("/v1", v1)
        .with_state(state)
}

const TRIAL_DURATION_DAYS: i64 = 14;

/// Per-tenant cap on webhooks enqueued per calendar month.
const MONTHLY_WEBHOOK_LIMIT: i64 = 50_000;

async fn ping(State(_state): State<AppState>) -> Result<&'static str, StatusCode> {
    Ok("pong")
}

async fn health() -> Json<HealthCheckResponse> {
    Json(HealthCheckResponse {
        status: 200,
        message: "healthy".into(),
    })
}

#[derive(Serialize)]
struct EnqueueResponse {
    id: String,
}

async fn enqueue_webhook(
    State(state): State<AppState>,
    api_key: auth::ValidApiKey,
    extract::Json(request_body): extract::Json<WebhookRequest>,
) -> Result<(StatusCode, Json<EnqueueResponse>), AppError> {
    info!(tenant_id = %api_key.tenant_id, "Webhook enqueue requested");

    if state.webhook_limiter.check_key(&api_key.tenant_id).is_err() {
        return Err(AppError::TooManyRequests);
    };

    validate_destination(&state.policy, &request_body.destination)?;

    // Hard monthly cap, atomically counted in the DB. Only valid requests that
    // will actually enqueue should burn a quota slot.
    if !crate::db::quotas::increment_monthly_quota(
        &state.pg_pool,
        api_key.tenant_id,
        MONTHLY_WEBHOOK_LIMIT,
    )
    .await?
    {
        return Err(AppError::MonthlyQuotaExceeded);
    }

    let webhook_id = webhook::enqueue(&state.worker_utils, api_key.tenant_id, request_body).await?;

    Ok((
        StatusCode::ACCEPTED,
        Json(EnqueueResponse { id: webhook_id }),
    ))
}

#[derive(Deserialize)]
struct SignupRequest {
    invite_code: String,
    name: String,
    email: String,
}

#[derive(Serialize)]
struct SignupResponse {
    tenant_id: uuid::Uuid,
    api_key: String,
    hmac_secret: String,
    expires_at: chrono::DateTime<Utc>,
}

async fn signup(
    State(state): State<AppState>,
    extract::Json(body): extract::Json<SignupRequest>,
) -> Result<(StatusCode, Json<SignupResponse>), AppError> {
    if state.signup_limiter.check().is_err() {
        return Err(AppError::TooManyRequests);
    };

    let hmac_secret = auth::generate_hmac_secret();
    let email_parsed = body.email;

    let mut tx = state.pg_pool.begin().await?;

    let tenant_id =
        crate::db::tenants::create_tenant(&mut tx, &body.name, &email_parsed, &hmac_secret).await?;

    // redeem invite code for this tenant
    crate::db::tenants::redeem_invite_code(&mut tx, &body.invite_code, tenant_id).await?;

    // if that worked, create and serve their api key
    let raw_key = auth::generate_api_key();
    let key_hash = auth::hash_api_key(&raw_key);
    let expires_at = Utc::now() + Duration::days(TRIAL_DURATION_DAYS);

    crate::db::tenants::create_api_key(&mut tx, tenant_id, &key_hash, "trial", expires_at).await?;

    tx.commit().await?;

    info!(tenant_id = %tenant_id, "New trial tenant signed up");

    Ok((
        StatusCode::CREATED,
        Json(SignupResponse {
            tenant_id,
            api_key: raw_key,
            hmac_secret,
            expires_at,
        }),
    ))
}

async fn webhook_activity_summary(
    State(state): State<AppState>,
    api_key: auth::ValidApiKey,
) -> Result<Json<WebhookActivitySummary>, AppError> {
    let summary = crate::db::stats::activity_summary(&state.pg_pool, api_key.tenant_id).await?;

    Ok(Json(summary))
}

async fn dashboard(
    State(state): State<AppState>,
    api_key: auth::ValidApiKey,
) -> Result<Html<String>, AppError> {
    let counts = crate::db::stats::activity_summary(&state.pg_pool, api_key.tenant_id).await?;

    Ok(Html(crate::views::render_dashboard(&counts)))
}

#[derive(Serialize)]
struct DeadLetterResponse {
    messages: Vec<crate::db::dlq::DlqEntry>,
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

async fn mock(State(_state): State<AppState>) -> StatusCode {
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
