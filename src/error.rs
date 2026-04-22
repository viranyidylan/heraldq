use axum::response::IntoResponse;
use graphile_worker::errors::GraphileWorkerError;
use reqwest::StatusCode;
use tracing::error;

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    // --- infra ---
    #[error("unauthorized")]
    Unauthorized,
    // generic 5xx bucket — real cause logged, never in the response body
    #[error("internal server error")]
    Internal(#[from] sqlx::Error),
    // graphile job add failure
    #[error("job enqueue error")]
    JobEnqueue(#[from] GraphileWorkerError),

    // --- auth / signup ---
    #[error("invalid invite code")]
    InvalidInviteCode,
    #[error("invite code already redeemed")]
    InviteCodeAlreadyRedeemed,
    #[error("email already registered")]
    EmailAlreadyRegistered,
}

impl IntoResponse for AppError {
    fn into_response(self) -> axum::response::Response {
        let (status, message) = match self {
            Self::Unauthorized => (StatusCode::UNAUTHORIZED, "unauthorized"),
            Self::Internal(ref e) => {
                error!(error = ?e, "internal server error");
                (StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
            }
            Self::JobEnqueue(ref e) => {
                error!(error = ?e, "job enqueue error");
                (StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
            }
            Self::InvalidInviteCode => (StatusCode::BAD_REQUEST, "invalid invite code"),
            Self::InviteCodeAlreadyRedeemed => {
                (StatusCode::CONFLICT, "invite code already redeemed")
            }
            Self::EmailAlreadyRegistered => (
                StatusCode::CONFLICT,
                "A tenant with this email address is already registered",
            ),
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
    async fn internal_error_does_not_leak_internals() {
        let response = AppError::Internal(sqlx::Error::RowNotFound).into_response();
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        let body = response_body(response).await;
        assert_eq!(
            body, "Internal server error",
            "response must not contain internal error details"
        );
    }

    #[tokio::test]
    async fn job_enqueue_error_does_not_leak_internals() {
        let inner = GraphileWorkerError::SqlError(sqlx::Error::RowNotFound);
        let response = AppError::JobEnqueue(inner).into_response();
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        let body = response_body(response).await;
        assert_eq!(
            body, "Internal server error",
            "response must not contain internal error details"
        );
    }

    #[tokio::test]
    async fn email_already_registered_is_409() {
        let response = AppError::EmailAlreadyRegistered.into_response();
        assert_eq!(response.status(), StatusCode::CONFLICT);
    }

    #[tokio::test]
    async fn invite_code_already_redeemed_is_409() {
        let response = AppError::InviteCodeAlreadyRedeemed.into_response();
        assert_eq!(response.status(), StatusCode::CONFLICT);
    }
}
