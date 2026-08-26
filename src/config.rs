use std::env::VarError;

#[derive(Debug)]
pub struct AppConfig {
    pub database_url: String,
    pub schema: String,
    pub port: u16,
    // Secret for verifying Paddle webhook signatures. Optional at parse time
    // so the API still boots without billing configured.
    pub paddle_webhook_secret: Option<String>,
    // Browser origins allowed to call /v1/signup (the marketing site's
    // signup form). Comma-separated; defaults to the production site.
    pub cors_allowed_origins: Vec<String>,
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
        Self::parse()
    }

    fn parse() -> Result<AppConfig, ConfigError> {
        let database_url = std::env::var("DATABASE_URL")
            .map_err(|_| ConfigError::MissingVar("DATABASE_URL".into()))?;

        let schema = std::env::var("GRAPHILE_SCHEMA")
            .map_err(|_| ConfigError::MissingVar("GRAPHILE_SCHEMA".into()))?;

        let port = std::env::var("PORT")
            .unwrap_or_else(|_| "3000".into())
            .parse::<u16>()
            .map_err(|_| ConfigError::MissingVar("PORT (invalid u16)".into()))?;

        let paddle_webhook_secret = std::env::var("PADDLE_WEBHOOK_SECRET")
            .ok()
            .filter(|s| !s.is_empty());

        let cors_allowed_origins = std::env::var("CORS_ALLOWED_ORIGINS")
            .unwrap_or_else(|_| "https://heraldq.com,https://www.heraldq.com".into())
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
            .collect();

        Ok(AppConfig {
            database_url,
            schema,
            port,
            paddle_webhook_secret,
            cors_allowed_origins,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    // Serialize env-mutating tests — env vars are process-global
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn setup_env(database_url: Option<&str>, schema: Option<&str>, port: Option<&str>) {
        unsafe {
            match database_url {
                Some(v) => std::env::set_var("DATABASE_URL", v),
                None => std::env::remove_var("DATABASE_URL"),
            }
            match schema {
                Some(v) => std::env::set_var("GRAPHILE_SCHEMA", v),
                None => std::env::remove_var("GRAPHILE_SCHEMA"),
            }
            match port {
                Some(v) => std::env::set_var("PORT", v),
                None => std::env::remove_var("PORT"),
            }
        }
    }

    #[test]
    fn from_env_succeeds_with_all_vars() {
        let _guard = ENV_LOCK.lock().unwrap();
        setup_env(
            Some("postgres://localhost/test"),
            Some("graphile"),
            Some("4000"),
        );
        let config = AppConfig::parse().expect("should succeed");
        assert_eq!(config.database_url, "postgres://localhost/test");
        assert_eq!(config.schema, "graphile");
        assert_eq!(config.port, 4000);
    }

    #[test]
    fn fails_missing_database_url() {
        let _guard = ENV_LOCK.lock().unwrap();
        setup_env(None, Some("graphile"), None);
        let err = AppConfig::parse().unwrap_err();
        assert!(err.to_string().contains("DATABASE_URL"), "error was: {err}");
    }

    #[test]
    fn fails_missing_schema() {
        let _guard = ENV_LOCK.lock().unwrap();
        setup_env(Some("postgres://localhost/test"), None, None);
        let err = AppConfig::parse().unwrap_err();
        assert!(
            err.to_string().contains("GRAPHILE_SCHEMA"),
            "error was: {err}"
        );
    }

    #[test]
    fn port_defaults_to_3000() {
        let _guard = ENV_LOCK.lock().unwrap();
        setup_env(Some("postgres://localhost/test"), Some("graphile"), None);
        let config = AppConfig::parse().expect("should succeed");
        assert_eq!(config.port, 3000);
    }

    #[test]
    fn port_rejects_non_numeric() {
        let _guard = ENV_LOCK.lock().unwrap();
        setup_env(
            Some("postgres://localhost/test"),
            Some("graphile"),
            Some("bad"),
        );
        let err = AppConfig::parse().unwrap_err();
        assert!(err.to_string().contains("PORT"), "error was: {err}");
    }
}
