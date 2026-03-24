use axum::{
    Json, Router,
    extract::{self, FromRef, State},
    routing::{get, post},
};
use chrono::{Duration, Utc};
use rand::RngExt;
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
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
    let tenant = sqlx::query!(
        "INSERT INTO tenants (name, tier) VALUES ($1, 'free') RETURNING id",
        body.name,
    )
    .fetch_one(&state.pg_pool)
    .await?;

    let raw_key = auth::generate_api_key();
    let key_hash = hex::encode(Sha256::digest(raw_key.as_bytes()));
    let expires_at = Utc::now() + Duration::days(TRIAL_DURATION_DAYS);

    sqlx::query!(
        "INSERT INTO api_keys (tenant_id, key_hash, name, expires_at) VALUES ($1, $2, $3, $4)",
        tenant.id,
        key_hash,
        "trial",
        expires_at,
    )
    .execute(&state.pg_pool)
    .await?;

    info!(tenant_id = %tenant.id, "New trial tenant signed up");

    Ok((
        StatusCode::CREATED,
        Json(SignupResponse {
            tenant_id: tenant.id,
            api_key: raw_key,
            expires_at,
        }),
    ))
}

async fn mock(
    State(_state): State<AppState>,
) -> StatusCode {
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
