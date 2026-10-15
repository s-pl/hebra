//! API errors: a status, a stable machine-readable code and a message.
//! Internal failures are logged in full and reported without details.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde_json::json;

#[derive(Debug)]
pub struct ApiError {
    pub(crate) status: StatusCode,
    pub(crate) code: &'static str,
    pub(crate) message: String,
}

impl ApiError {
    pub(crate) fn new(status: StatusCode, code: &'static str, message: impl Into<String>) -> Self {
        Self {
            status,
            code,
            message: message.into(),
        }
    }
    pub(crate) fn not_found(what: impl Into<String>) -> Self {
        Self::new(StatusCode::NOT_FOUND, "not_found", what)
    }
    pub(crate) fn bad(message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, "bad_request", message)
    }
    /// A plan limit was reached (HTTP 402).
    pub(crate) fn payment(message: impl Into<String>) -> Self {
        Self::new(StatusCode::PAYMENT_REQUIRED, "plan_limit", message)
    }
    pub(crate) fn conflict(message: impl Into<String>) -> Self {
        Self::new(StatusCode::CONFLICT, "conflict", message)
    }
    pub(crate) fn forbidden(code: &'static str, message: impl Into<String>) -> Self {
        Self::new(StatusCode::FORBIDDEN, code, message)
    }
    pub(crate) fn unauthorized(message: &str) -> Self {
        Self::new(StatusCode::UNAUTHORIZED, "unauthorized", message)
    }
    /// A server-side failure: logged in full, reported without details.
    pub(crate) fn internal(code: &'static str, e: impl std::fmt::Display) -> Self {
        tracing::error!(code, error = %e, "internal error");
        Self::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            code,
            "internal server error",
        )
    }
}

impl ApiError {
    /// The human-readable message.
    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl From<hebra_core::Error> for ApiError {
    fn from(e: hebra_core::Error) -> Self {
        match e {
            hebra_core::Error::InvalidArgument(m) => Self::bad(m),
            hebra_core::Error::InvalidId(_) => Self::bad(e.to_string()),
            hebra_core::Error::CorruptObject(_) => {
                Self::new(StatusCode::BAD_REQUEST, "corrupt_object", e.to_string())
            }
            other => Self::internal("internal", other),
        }
    }
}

impl From<serde_json::Error> for ApiError {
    fn from(e: serde_json::Error) -> Self {
        Self::bad(e.to_string())
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let body = json!({ "error": { "code": self.code, "message": self.message } });
        (self.status, axum::Json(body)).into_response()
    }
}

pub type ApiResult<T> = Result<T, ApiError>;
