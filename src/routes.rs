use axum::{
    Json, Router,
    extract::{self, FromRef, State},
    routing::{get, post},
};
use graphile_worker::{JobSpecBuilder, WorkerUtils};
use rand::RngExt;
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::PgPool;
use ulid::Ulid;

use crate::{SendWebhook, WebhookRequest, auth, error::AppError};

#[derive(Clone)]
pub struct AppState {
    pub pg_pool: PgPool,
    pub worker_utils: WorkerUtils,
}

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
    _api_key: auth::ValidApiKey,
    extract::Json(request_body): extract::Json<WebhookRequest>,
) -> Result<StatusCode, AppError> {
    println!("[api] Received a post to add_job()");

    state
        .worker_utils
        .add_job(
            SendWebhook {
                id: Ulid::new().to_string(),
                destination: request_body.destination.to_string(),
                payload: request_body.payload,
            },
            JobSpecBuilder::new().max_attempts(3).build(),
        )
        .await?;

    Ok(StatusCode::ACCEPTED)
}

async fn mock(
    State(_state): State<AppState>,
    extract::Json(payload): extract::Json<Value>,
) -> StatusCode {
    let roll = rand::rng().random_range(0..100);

    println!("[/chaos] payload='{}' roll={}", payload, roll);

    if roll < 10 {
        println!("[api] /chaos 200 OK");
        StatusCode::OK
    } else if roll < 15 {
        println!("[api] /chaos 418 IM A TEAPOT");
        StatusCode::IM_A_TEAPOT
    } else {
        println!("[api] /chaos 500 INTERNAL SERVER ERROR");
        StatusCode::INTERNAL_SERVER_ERROR
    }
}
