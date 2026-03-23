use axum::{
    Json, Router,
    extract::{self, FromRef, State},
    routing::{get, post},
};
use rand::RngExt;
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::PgPool;

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
        .route("/mock", post(mock))
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
    println!("[api] Received a post to add_job()");

    webhook::enqueue(&state.worker_utils, api_key.tenant_id, request_body).await?;

    Ok(StatusCode::ACCEPTED)
}

async fn mock(
    State(_state): State<AppState>,
    extract::Json(payload): extract::Json<Value>,
) -> StatusCode {
    let roll = rand::rng().random_range(0..100);

    println!("[/chaos] payload='{}' roll={}", payload, roll);

    if roll < 40 {
        println!("[api] /chaos 200 OK");
        StatusCode::OK
    } else if roll < 10 {
        println!("[api] /chaos 418 IM A TEAPOT");
        StatusCode::IM_A_TEAPOT
    } else {
        println!("[api] /chaos 500 INTERNAL SERVER ERROR");
        StatusCode::INTERNAL_SERVER_ERROR
    }
}
