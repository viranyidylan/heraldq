use std::sync::Arc;

use governor::{Quota, RateLimiter};
use graphile_worker::WorkerUtils;
use nonzero_ext::nonzero;
use sliq::{AppState, config::AppConfig, db, routes, webhook::ssrf};
use tracing::info;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenvy::dotenv().ok();
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let config = AppConfig::from_env()?;

    let pg_pool = db::create_pool(&config).await?;
    db::run_migrations(&pg_pool).await?;

    let utils = WorkerUtils::new(pg_pool.clone(), config.schema.clone());

    // Per-tenant burst limiter: one token bucket per tenant_id, 10 req/s each.
    let webhook_limiter = Arc::new(RateLimiter::keyed(Quota::per_second(nonzero!(10u32))));

    // Global throttle on signup to limit invite-code abuse: 100/min across all clients.
    let signup_limiter = Arc::new(RateLimiter::direct(Quota::per_minute(nonzero!(100u32))));

    let state = AppState {
        pg_pool,
        worker_utils: utils,
        schema: config.schema,
        webhook_limiter,
        signup_limiter,
        policy: ssrf::policy(),
    };

    let app = routes::build(state);

    let addr = format!("0.0.0.0:{}", config.port);
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    info!(addr = %addr, "API server listening");

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
