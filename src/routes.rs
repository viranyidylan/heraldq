use axum::{
    Json, Router,
    extract::{self, FromRef, State},
    routing::{get, post},
};
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

#[derive(Deserialize, Serialize)]
struct StatusResponse {
    received: i32,
    delivered: i32,
    abandoned: i32,
}

pub fn build(state: AppState) -> Router {
    Router::new()
        .route("/ping", get(ping))
        .route("/health", get(health))
        .route("/webhook", post(add_job))
        .route("/mock", post(mock))
        .route("/dlq", get(dlq))
        .route("/status", get(status))
        .with_state(state)
}

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

async fn status(
    State(state): State<AppState>,
    api_key: auth::ValidApiKey,
) -> Result<Json<StatusResponse>, AppError> {
    Ok(Json(StatusResponse {
        abandoned: 5,
        delivered: 3,
        received: 1,
    }))
}

async fn dlq() {
    todo!()
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
