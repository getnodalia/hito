//! Every handler returns [`AppError`] on failure: a status code and a JSON [`ApiError`]. Internal
//! failures are logged with their cause and shown to the client without it.

use hito_core::ValidationError;
use hito_protocol::{ApiError, ErrorCode};
use hito_storage::StoreError;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;

#[derive(Debug)]
pub struct AppError {
    pub status: StatusCode,
    pub body: ApiError,
}

pub type AppResult<T> = Result<T, AppError>;

impl AppError {
    pub fn new(status: StatusCode, code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            status,
            body: ApiError {
                code,
                message: message.into(),
                field: None,
            },
        }
    }
    pub fn invalid(message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, ErrorCode::Invalid, message)
    }
    pub fn unauthenticated() -> Self {
        Self::new(
            StatusCode::UNAUTHORIZED,
            ErrorCode::Unauthenticated,
            "log in to continue",
        )
    }
    pub fn forbidden(message: impl Into<String>) -> Self {
        Self::new(StatusCode::FORBIDDEN, ErrorCode::Forbidden, message)
    }
    /// Also used when something exists but the caller may not know it does.
    pub fn not_found() -> Self {
        Self::new(StatusCode::NOT_FOUND, ErrorCode::NotFound, "not found")
    }
    pub fn conflict(message: impl Into<String>) -> Self {
        Self::new(StatusCode::CONFLICT, ErrorCode::Conflict, message)
    }
    pub fn rate_limited() -> Self {
        Self::new(
            StatusCode::TOO_MANY_REQUESTS,
            ErrorCode::RateLimited,
            "too many requests; try again shortly",
        )
    }
    pub fn stale_epoch() -> Self {
        Self::new(
            StatusCode::CONFLICT,
            ErrorCode::StaleEpoch,
            "the conversation's encryption moved on; catch up and encrypt again",
        )
    }
    pub fn internal() -> Self {
        Self::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            ErrorCode::Internal,
            "the server failed; see its log",
        )
    }
}

impl From<ValidationError> for AppError {
    fn from(e: ValidationError) -> Self {
        let mut err = AppError::invalid(e.to_string());
        err.body.field = Some(e.field().to_owned());
        err
    }
}

impl From<StoreError> for AppError {
    fn from(e: StoreError) -> Self {
        match e {
            StoreError::Conflict => AppError::conflict("it already exists"),
            StoreError::NotFound => AppError::not_found(),
            StoreError::InitializationDenied => AppError::new(
                StatusCode::FORBIDDEN,
                ErrorCode::RegistrationClosed,
                "server initialization is not authorized or already completed",
            ),
            StoreError::StaleEpoch => AppError::stale_epoch(),
            StoreError::InviteExhausted => AppError::new(
                StatusCode::GONE,
                ErrorCode::NotFound,
                "this invitation has expired or been used up",
            ),
            other => {
                tracing::error!(error = %other, "storage failure");
                AppError::internal()
            }
        }
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        (self.status, Json(self.body)).into_response()
    }
}
