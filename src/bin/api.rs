use std::sync::Arc;

use governor::{Quota, RateLimiter};
use graphile_worker::WorkerUtils;
use nonzero_ext::nonzero;
use sliq::{AppState, config::AppConfig, db, routes, telemetry, webhook::ssrf};
use tracing::info;

/// API queries are short (auth lookup, quota bump, enqueue, list pages);
/// a small pool suffices and leaves Postgres connection budget for the worker.
const API_POOL_SIZE: u32 = 10;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenvy::dotenv().ok();
    telemetry::init_tracing();
    telemetry::init_metrics()?;

    let config = AppConfig::from_env()?;

    let pg_pool = db::create_pool(&config, API_POOL_SIZE).await?;
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
        paddle_webhook_secret: config.paddle_webhook_secret.clone(),
        paddle_api: config.paddle_api_key.as_ref().map(|key| {
            sliq::paddle::api::PaddleApi::new(key.clone(), config.paddle_api_url.clone())
        }),
        cors_allowed_origins: config.cors_allowed_origins.clone(),
    };

    if state.paddle_api.is_none() {
        tracing::warn!(
            "PADDLE_API_KEY not set — stub Paddle customers won't hydrate until a customer event arrives"
        );
    }

    if state.paddle_webhook_secret.is_none() {
        tracing::warn!(
            "PADDLE_WEBHOOK_SECRET not set — /webhooks/paddle will refuse deliveries (503)"
        );
    }

    let app = routes::build(state);

    let addr = format!("0.0.0.0:{}", config.port);
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    info!(addr = %addr, "API server listening");

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    Ok(())
}

/// Resolves on SIGINT (Ctrl+C) or SIGTERM. Docker, Kubernetes, and systemd
/// all stop processes with SIGTERM — listening for Ctrl+C alone means every
/// production deploy kills in-flight requests instead of draining them.
async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install CTRL+C signal handler");
    };

    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM signal handler")
            .recv()
            .await;
    };

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }

    info!("shutdown signal received, draining in-flight requests");
}
