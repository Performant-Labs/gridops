use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde_json::json;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ApiError {
    #[error("authentication required")]
    Unauthorized,
    #[error("forbidden")]
    Forbidden,
    #[error("{0}")]
    BadRequest(String),
    #[error("{0}")]
    NotFound(String),
    #[error("{0}")]
    Conflict(String),
    /// A refusal a client can tell from other conflicts by its `code`.
    #[error("{message}")]
    Refused { code: &'static str, message: String },
    /// A request the server understood but refuses as it stands (422): its
    /// `code` tells the client why, and `details` says where (for example the
    /// positions of the job-category rules that don't compile, #197).
    #[error("{message}")]
    #[cfg_attr(not(test), allow(dead_code, reason = "#199's rule check returns it"))]
    Invalid {
        code: &'static str,
        message: String,
        details: serde_json::Value,
    },
    #[error("{0}")]
    PayloadTooLarge(String),
    #[error("{0}")]
    ServiceUnavailable(String),
    #[error(transparent)]
    Internal(#[from] anyhow::Error),
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, message) = match &self {
            Self::Unauthorized => (StatusCode::UNAUTHORIZED, self.to_string()),
            Self::Forbidden => (StatusCode::FORBIDDEN, self.to_string()),
            Self::BadRequest(message) => (StatusCode::BAD_REQUEST, message.clone()),
            Self::NotFound(message) => (StatusCode::NOT_FOUND, message.clone()),
            Self::Conflict(message) => (StatusCode::CONFLICT, message.clone()),
            Self::Refused { message, .. } => (StatusCode::CONFLICT, message.clone()),
            Self::Invalid { message, .. } => (StatusCode::UNPROCESSABLE_ENTITY, message.clone()),
            Self::PayloadTooLarge(message) => (StatusCode::PAYLOAD_TOO_LARGE, message.clone()),
            Self::ServiceUnavailable(message) => (StatusCode::SERVICE_UNAVAILABLE, message.clone()),
            Self::Internal(error) => {
                tracing::error!(error = ?error, "request failed");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "GridOps hit an unexpected error. Try again, and check the API logs if it keeps happening.".into(),
                )
            }
        };
        let mut body = json!({ "error": message });
        match &self {
            Self::Refused { code, .. } => body["code"] = json!(code),
            Self::Invalid { code, details, .. } => {
                body["code"] = json!(code);
                body["details"] = details.clone();
            }
            _ => {}
        }
        (status, Json(body)).into_response()
    }
}

impl From<sqlx::Error> for ApiError {
    fn from(value: sqlx::Error) -> Self {
        Self::Internal(value.into())
    }
}

impl From<reqwest::Error> for ApiError {
    fn from(value: reqwest::Error) -> Self {
        Self::Internal(value.into())
    }
}

pub type ApiResult<T> = Result<T, ApiError>;
