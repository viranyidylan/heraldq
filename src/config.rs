use std::env::VarError;

pub struct AppConfig {
    pub database_url: String,
    pub schema: String,
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("missing required environment variable: {0}")]
    MissingVar(String),
}

impl From<VarError> for ConfigError {
    fn from(error: VarError) -> Self {
        ConfigError::MissingVar(error.to_string())
    }
}

impl AppConfig {
    pub fn from_env() -> Result<AppConfig, ConfigError> {
        dotenvy::dotenv().ok();

        let database_url = std::env::var("DATABASE_URL")
            .map_err(|_| ConfigError::MissingVar("DATABASE_URL".into()))?;

        let schema = std::env::var("GRAPHILE_SCHEMA")
            .map_err(|_| ConfigError::MissingVar("GRAPHILE_SCHEMA".into()))?;

        Ok(AppConfig {
            database_url,
            schema,
        })
    }
}
