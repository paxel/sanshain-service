pub mod handlers;
pub mod middleware;

use crate::application::services::AppError;
use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
};

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, body) = match self {
            AppError::BadRequest(msg) => (StatusCode::BAD_REQUEST, msg),
            AppError::Conflict(msg) => (StatusCode::CONFLICT, msg),
            AppError::NotFound(msg) => (StatusCode::NOT_FOUND, msg),
            AppError::Unauthorized => (StatusCode::UNAUTHORIZED, "Unauthorized".to_string()),
            AppError::Forbidden => (StatusCode::FORBIDDEN, "Forbidden".to_string()),
            AppError::Internal(msg) => (StatusCode::INTERNAL_SERVER_ERROR, msg),
            AppError::BreakingChange(msg) => (StatusCode::CONFLICT, msg),
        };

        if status.is_client_error() {
            tracing::warn!("Client error: {}", body);
        } else if status.is_server_error() {
            tracing::error!("Server error: {}", body);
        }

        (status, body).into_response()
    }
}
