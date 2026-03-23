use sqlx::PgPool;

use crate::{config::AppConfig};

pub mod dlq;
pub mod deliveries;

pub async fn create_pool(config: &AppConfig) -> Result<PgPool, sqlx::Error> {
    let pg_pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(5)
        .connect(config.database_url.as_ref())
        .await?;

    Ok(pg_pool)
}

pub async fn run_migrations(pool: &PgPool) -> Result<(), sqlx::migrate::MigrateError> {
    sqlx::migrate!("./migrations")
        .run(pool)
        .await?;

    Ok(())
}