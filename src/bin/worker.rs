use std::time::Duration;

use graphile_worker::{self, LocalQueueConfig, RefetchDelayConfig};
use test_graphile::SendWebhook;

#[tokio::main] 
async fn main() -> Result<(), Box<dyn std::error::Error>> {

    println!("[worker] >>> Spinning up");

    let database_url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://postgres:xi23pe@localhost/sliq".to_string());
    // Create a PostgreSQL connection pool
    let pg_pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(10)
        .connect(&database_url)
        .await
        .unwrap();

    println!("[worker] >>> database connection established");

    // Initialize and run the worker
    let worker = graphile_worker::WorkerOptions::default()
        .local_queue(
            LocalQueueConfig::default()
                .with_size(100)                              // Cache up to 100 jobs
                .with_ttl(Duration::from_secs(300))          // Return unclaimed jobs after 5 minutes
                .with_refetch_delay(
                    RefetchDelayConfig::default()
                        .with_duration(Duration::from_millis(100))  // Delay between refetches
                        .with_threshold(10)                         // Refetch when queue drops below 10
                )
        )
        .concurrency(50)                 
        .schema("graphile_worker")      // Use this PostgreSQL schema
        .define_job::<SendWebhook>()    // Register the task handler
        .pg_pool(pg_pool)               // Provide the database connection
        .init()
        .await?;
    
    println!("[worker] >>> graphile configured");
    println!("[worker] >>> starting worker...");

    // start worker
    worker.run().await?;

    Ok(())
}