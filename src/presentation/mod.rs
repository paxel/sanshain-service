pub mod handlers;
pub mod middleware;

use crate::application::services::AppError;
use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::{Deserialize, Serialize};

/// The body every error response carries.
///
/// Errors used to render as a bare `String`, which left nothing to attach
/// structured detail to. `error` holds the same sentence that body used to
/// contain, so the human-readable half is unchanged; the optional fields are
/// populated only by the variants that have something to say.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorBody {
    pub error: String,
    /// Set when a Provide was refused by the version rules: the next free
    /// version the Producer should publish as instead.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub proposed_version: Option<String>,
}

impl ErrorBody {
    fn message(error: String) -> Self {
        Self {
            error,
            proposed_version: None,
        }
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, body) = match self {
            AppError::BadRequest(msg) => (StatusCode::BAD_REQUEST, ErrorBody::message(msg)),
            AppError::Conflict(msg) => (StatusCode::CONFLICT, ErrorBody::message(msg)),
            AppError::NotFound(msg) => (StatusCode::NOT_FOUND, ErrorBody::message(msg)),
            AppError::Gone(msg) => (StatusCode::GONE, ErrorBody::message(msg)),
            AppError::Unauthorized => (
                StatusCode::UNAUTHORIZED,
                ErrorBody::message("Unauthorized".to_string()),
            ),
            AppError::Forbidden => (
                StatusCode::FORBIDDEN,
                ErrorBody::message("Forbidden".to_string()),
            ),
            AppError::Internal(msg) => (StatusCode::INTERNAL_SERVER_ERROR, ErrorBody::message(msg)),
            AppError::BreakingChange(msg) => (StatusCode::CONFLICT, ErrorBody::message(msg)),
            // 409 with the remedy machine-readable: the rejection is
            // self-service, so the body carries the next free version.
            AppError::VersionConflict { message, proposed } => (
                StatusCode::CONFLICT,
                ErrorBody {
                    error: message,
                    proposed_version: Some(proposed.to_string()),
                },
            ),
        };

        if status.is_client_error() {
            tracing::warn!("Client error: {}", body.error);
        } else if status.is_server_error() {
            tracing::error!("Server error: {}", body.error);
        }

        (status, Json(body)).into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::to_bytes;

    async fn render(err: AppError) -> (StatusCode, serde_json::Value) {
        let response = err.into_response();
        let status = response.status();
        let bytes = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("error body should be readable");
        let value = serde_json::from_slice(&bytes).expect("error body should be JSON");
        (status, value)
    }

    #[tokio::test]
    async fn every_variant_renders_json_with_its_status() {
        let cases = vec![
            (
                AppError::BadRequest("bad".into()),
                StatusCode::BAD_REQUEST,
                "bad",
            ),
            (
                AppError::Conflict("clash".into()),
                StatusCode::CONFLICT,
                "clash",
            ),
            (
                AppError::NotFound("absent".into()),
                StatusCode::NOT_FOUND,
                "absent",
            ),
            (
                AppError::Gone("dropped".into()),
                StatusCode::GONE,
                "dropped",
            ),
            (
                AppError::Unauthorized,
                StatusCode::UNAUTHORIZED,
                "Unauthorized",
            ),
            (AppError::Forbidden, StatusCode::FORBIDDEN, "Forbidden"),
            (
                AppError::Internal("boom".into()),
                StatusCode::INTERNAL_SERVER_ERROR,
                "boom",
            ),
            (
                AppError::BreakingChange("removed /v1/foo".into()),
                StatusCode::CONFLICT,
                "removed /v1/foo",
            ),
        ];

        for (err, expected_status, expected_message) in cases {
            let (status, value) = render(err).await;
            assert_eq!(status, expected_status);
            assert_eq!(value["error"], expected_message);
        }
    }

    /// The optional fields must stay absent rather than serialising as `null`,
    /// so a client can test for their presence.
    #[tokio::test]
    async fn optional_fields_are_omitted_when_unset() {
        let (_, value) = render(AppError::NotFound("absent".into())).await;
        let object = value.as_object().expect("body should be an object");
        assert_eq!(object.len(), 1);
        assert!(!object.contains_key("pending_id"));
        assert!(!object.contains_key("status"));
    }

    #[tokio::test]
    async fn content_type_is_json() {
        let response = AppError::Forbidden.into_response();
        let content_type = response
            .headers()
            .get(axum::http::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default();
        assert!(
            content_type.starts_with("application/json"),
            "expected JSON content type, got {content_type}"
        );
    }
}
