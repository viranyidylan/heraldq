use graphile_worker::{WorkerUtils};
use serde_json::json;
use sqlx::{Pool, Postgres};
use test_graphile::{SendWebhook, WebhookRequest};
use tokio;
use axum::{
    Router, extract::{self, State}, http::request, routing::post
};
use ulid::Ulid;

#[derive(Clone)]
struct AppState {
    pg_pool: Pool<Postgres>,
    worker_utils: WorkerUtils,
    schema: String,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("[api] >>> starting api");
    let schema = "graphile_worker";
    
    // Create a PostgreSQL connection pool
    let database_url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://postgres:xi23pe@localhost/sliq".to_string());
    let pg_pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(5)
        .connect(&database_url)
        .await?;

    // Get a WorkerUtils instance to manage jobs
    let utils = WorkerUtils::new(pg_pool.clone(), schema.to_string());

    let state = AppState {
        pg_pool,
        worker_utils: utils,
        schema: schema.to_string(),
    };

    let app = Router::new()
        .route("/webhook", post(add_job))
        .with_state(state);
    
    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await.unwrap();
    
    println!("[api] >>> listening...");

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

async fn add_job(
    State(state): State<AppState>,
    extract::Json(request_body): extract::Json<WebhookRequest>
) {
    println!("[api] Received a post to add_job()");

    // Type-safe method (recommended):
    let _ = state.worker_utils.add_job(
        SendWebhook {
            id: Ulid::new().to_string(),
            destination: request_body.destination.to_string(),
            payload: request_body.payload
        },
        Default::default(), // Use default job options
    ).await;
}

async fn mock_job( 
    State(state): State<AppState>,
    extract::Json(payload): extract::Json<WebhookRequest>
) {
    println!("[api] Received a post to add_job()");

    // Type-safe method (recommended):
    let _ = state.worker_utils.add_job(
        SendWebhook {
            id: Ulid::new().to_string(),
            destination: "https://webhook.site/d6200f3e-42c1-4b5a-aed0-69dcd23f4942".to_string(),
            payload: json!({ "A": 65, "B": 66, "C": 67 })
        },
        Default::default(), // Use default job options
    ).await;
}