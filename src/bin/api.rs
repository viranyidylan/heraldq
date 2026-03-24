use graphile_worker::WorkerUtils;
use sliq::{AppState, config::AppConfig, db, routes};
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

    let state = AppState {
        pg_pool,
        worker_utils: utils,
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
