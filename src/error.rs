use axum::response::IntoResponse;
use graphile_worker::errors::GraphileWorkerError;
use reqwest::StatusCode;

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("unauthorized error")]
    Unauthorized,
    // db failure
    #[error("database error")]
    Database(#[from] sqlx::Error),
    // graphile job add failure
    #[error("job enqueue error")]
    JobEnqueue(#[from] GraphileWorkerError),
    // classic not found error
    // NotFound()
}

impl IntoResponse for AppError {
    fn into_response(self) -> axum::response::Response {
        let (status, message) = match self {
            Self::Unauthorized => (StatusCode::UNAUTHORIZED, "unauthorized"),
            Self::Database(e) => {
                eprintln!("database error: {e}");
                (StatusCode::INTERNAL_SERVER_ERROR, "database error")
            }
            Self::JobEnqueue(e) => {
                eprintln!("job enqueue error: {e}");
                (StatusCode::INTERNAL_SERVER_ERROR, "job enqueue error")
            }
        };
        (status, message.to_owned()).into_response()
    }
}
