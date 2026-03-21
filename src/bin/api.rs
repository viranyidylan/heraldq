use axum::{
    Json, Router,
    extract::{self, FromRef, State},
    routing::{get, post},
};
use graphile_worker::{JobSpecBuilder, WorkerUtils};
use rand::RngExt;
use reqwest::{Client, StatusCode};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sliq::{SendWebhook, WebhookRequest, config::AppConfig, db, error::AppError};
use sqlx::PgPool;
use ulid::Ulid;

#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct HealthCheckResponse {
    pub status: i32,
    pub message: String,
}

struct ValidApiKey {
    api_key_id: String,
    tenant_id: String,
}

#[derive(Clone)]
pub struct AppState {
    pg_pool: PgPool,
    worker_utils: WorkerUtils,
    http_client: Client,
    schema: String,
}

impl FromRef<AppState> for PgPool {
    fn from_ref(app_state: &AppState) -> PgPool {
        app_state.pg_pool.clone()
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {

    let config = AppConfig::from_env()?;
    
    let pg_pool = db::create_pool(&config).await?;
    db::run_migrations(&pg_pool).await;

    // Get a WorkerUtils instance to manage jobs
    let utils = WorkerUtils::new(pg_pool.clone(), config.schema.clone());

    let http = reqwest::Client::new();

    let state = AppState {
        pg_pool,
        worker_utils: utils,
        http_client: http,
        schema: config.schema,
    };

    let app = Router::new()
        .route("/ping", get(ping))
        .route("/health", get(health))
        .route("/webhook", post(add_job))
        .route("/mock", post(mock))
        .with_state(state);

    let port = std::env::var("PORT").unwrap_or_else(|_| "3000".to_string());
    let addr = format!("0.0.0.0:{}", port);
    let listener = tokio::net::TcpListener::bind(&addr).await.unwrap();
    println!("App listening at address: {}", &addr);

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    Ok(())
}

async fn shutdown_signal() {
    tokio::signal::ctrl_c()
        .await
        .expect("failed to install CTRL+C signal handler");
}

async fn ping() -> String {
    "pong".to_string()
}

async fn health() -> axum::Json<HealthCheckResponse> {
    Json(HealthCheckResponse {
        status: 200,
        message: "healthy".to_string(),
    })
}

async fn add_job(
    State(state): State<AppState>,
    _api_key: sliq::auth::ValidApiKey,
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
            JobSpecBuilder::new().max_attempts(3).build(), // Default::default(), // Use default job options, default max_attempts is 25
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
        StatusCode::OK // 10%
    } else if roll < 15 {
        println!("[api] /chaos 418 IM A TEAPOT");
        StatusCode::IM_A_TEAPOT // 5%
    } else {
        println!("[api] /chaos 500 INTERNAL SERVER ERROR");
        StatusCode::INTERNAL_SERVER_ERROR // 85%
    }
}
