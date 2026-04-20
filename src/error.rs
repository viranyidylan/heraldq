use axum::response::IntoResponse;
use graphile_worker::errors::GraphileWorkerError;
use reqwest::StatusCode;
use tracing::error;

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
    #[error("invalid invite code")]
    InvalidInviteCode
}

impl IntoResponse for AppError {
    fn into_response(self) -> axum::response::Response {
        let (status, message) = match self {
            Self::Unauthorized => (StatusCode::UNAUTHORIZED, "unauthorized"),
            Self::Database(ref e) => {
                error!(error = %e, "Database error");
                (StatusCode::INTERNAL_SERVER_ERROR, "database error")
            }
            Self::JobEnqueue(ref e) => {
                error!(error = %e, "Job enqueue error");
                (StatusCode::INTERNAL_SERVER_ERROR, "job enqueue error")
            }
            Self::InvalidInviteCode => (StatusCode::BAD_REQUEST, "invalid invite code")
        };
        (status, message.to_owned()).into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::response::IntoResponse;
    use http_body_util::BodyExt;

    async fn response_body(response: axum::response::Response) -> String {
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        String::from_utf8(bytes.to_vec()).unwrap()
    }

    #[tokio::test]
    async fn unauthorized_is_401() {
        let response = AppError::Unauthorized.into_response();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn database_error_does_not_leak_internals() {
        let response = AppError::Database(sqlx::Error::RowNotFound).into_response();
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        let body = response_body(response).await;
        assert_eq!(body, "database error", "response must not contain internal error details");
    }

    #[tokio::test]
    async fn job_enqueue_error_does_not_leak_internals() {
        let inner = GraphileWorkerError::SqlError(sqlx::Error::RowNotFound);
        let response = AppError::JobEnqueue(inner).into_response();
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        let body = response_body(response).await;
        assert_eq!(body, "job enqueue error", "response must not contain internal error details");
    }
}
