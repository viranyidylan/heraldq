use sqlx::PgPool;

use crate::config::AppConfig;

pub mod deliveries;
pub mod dlq;
pub mod jobs;
pub mod quotas;
pub mod stats;
pub mod tenants;

pub async fn create_pool(config: &AppConfig, max_connections: u32) -> Result<PgPool, sqlx::Error> {
    let pg_pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(max_connections)
        .connect(config.database_url.as_ref())
        .await?;

    Ok(pg_pool)
}

pub async fn run_migrations(pool: &PgPool) -> Result<(), sqlx::migrate::MigrateError> {
    sqlx::migrate!("./migrations").run(pool).await?;

    Ok(())
}
