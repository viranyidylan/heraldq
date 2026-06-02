use graphile_worker::{self, LocalQueueConfig, RefetchDelayConfig};
use reqwest::redirect;
use sliq::{SendWebhook, WorkerState, config::AppConfig, db};
use std::time::Duration;
use tracing::info;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenvy::dotenv().ok();
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    info!("Worker spinning up");

    let config = AppConfig::from_env()?;

    let pg_pool = db::create_pool(&config).await?;
    db::run_migrations(&pg_pool).await?;

    let worker_state = WorkerState {
        http_client: reqwest::Client::builder()
            .redirect(redirect::Policy::none())
            .connect_timeout(Duration::from_secs(8))
            .timeout(Duration::from_secs(30))
            .user_agent("HeraldQ/1.0")
            .build()?,
        pg_pool: pg_pool.clone(),
    };

    info!("Database connection established");

    // Initialize and run the worker
    let worker = graphile_worker::WorkerOptions::default()
        .add_extension(worker_state)
        // LocalQueue batch-fetches jobs from the database to reduce DB load, trading latency for throughput
        .local_queue(
            LocalQueueConfig::default()
                .with_size(100) // Cache up to 100 jobs
                .with_ttl(Duration::from_secs(300)) // Return unclaimed jobs after 5 minutes
                .with_refetch_delay(
                    RefetchDelayConfig::default()
                        .with_duration(Duration::from_millis(100)) // Delay between refetches
                        .with_threshold(10), // Refetch when queue drops below 10
                ),
        )
        // Job completions are collected in a batch and flushed together after a delay to reduce the number of SQL round trips
        .complete_job_batch_delay(Duration::from_millis(5))
        .concurrency(30)
        .schema(&config.schema.clone()) // Use this PostgreSQL schema
        .define_job::<SendWebhook>() // Register the task handler
        .pg_pool(pg_pool)
        .init()
        .await?;

    info!("Graphile configured, starting worker");

    // start worker
    worker.run().await?;

    Ok(())
}
