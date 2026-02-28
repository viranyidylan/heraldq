use graphile_worker::{JobSpecBuilder, WorkerUtils};
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use serde_json::{json,Value};
use sqlx::{Pool, Postgres};
use test_graphile::{SCHEMA, SendWebhook, WebhookRequest};
use tokio;
use axum::{
    Json, Router, extract::{self, State}, http::request, routing::{get, post}
};
use rand::RngExt;
use ulid::Ulid;

#[derive(Clone)]
struct AppState {
    pg_pool: Pool<Postgres>,
    worker_utils: WorkerUtils,
    schema: String,
}

#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct HealthCheckResponse {
    pub status: i32,
    pub message: String,
} 

#[tokio::main]
async fn main() {
    println!("[api] >>> starting api");
    let schema = "graphile_worker";
    
    // Create a PostgreSQL connection pool
    let database_url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://postgres:xi23pe@localhost/sliq".to_string());
    let pg_pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(5)
        .connect(&database_url).await
        .expect("Failed to connect to database");

    // Get a WorkerUtils instance to manage jobs
    let utils = WorkerUtils::new(pg_pool.clone(), schema.to_string());

    let state = AppState {
        pg_pool,
        worker_utils: utils,
        schema: SCHEMA.to_string(),
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
    // axum::serve(listener, app).await.unwrap();
    
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .unwrap();
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
    Json(HealthCheckResponse { status: 200, message: "healthy".to_string() })
}

async fn add_job(
    State(state): State<AppState>,
    extract::Json(request_body): extract::Json<WebhookRequest>
) -> StatusCode {
    println!("[api] Received a post to add_job()");

    // Type-safe method (recommended):
    match state.worker_utils.add_job(
        SendWebhook {
            id: Ulid::new().to_string(),
            destination: request_body.destination.to_string(),
            payload: request_body.payload
        },
        JobSpecBuilder::new()
            .max_attempts(3)
            .build()
        // Default::default(), // Use default job options, default max_attempts is 25 
    ).await {
        Ok(job) => {
            println!("Job with id {} added", job.id());
            StatusCode::ACCEPTED
        }, 
        Err(e) => {
            eprintln!("Error: Could not add job to graphile: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        }
    }
}

async fn mock(    
    State(state): State<AppState>,
    extract::Json(payload): extract::Json<Value>
) -> StatusCode {
    let roll = rand::rng().random_range(0..100);

    println!("[/chaos] payload='{}' roll={}", payload, roll);

    if roll < 10 {
        println!("[api] /chaos 200 OK");
        StatusCode::OK               // 10%
    } else if roll < 15 {
        println!("[api] /chaos 418 IM A TEAPOT");
        StatusCode::IM_A_TEAPOT      // 5%
    } else {
        println!("[api] /chaos 500 INTERNAL SERVER ERROR");
        StatusCode::INTERNAL_SERVER_ERROR // 85%
    }}