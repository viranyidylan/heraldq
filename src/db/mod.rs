use sqlx::PgPool;

use crate::{config::AppConfig, error::AppError};

pub async fn create_pool(config: &AppConfig) -> Result<PgPool, AppError> {
    let pg_pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(5)
        .connect(config.database_url.as_ref())
        .await
        .expect("Failed to connect to database");

    Ok(pg_pool)
}

pub async fn run_migrations(pool: &PgPool) {
    sqlx::migrate!("./migrations")
        .run(pool)
        .await
        .expect("[api] Failed to run database migrations");
}