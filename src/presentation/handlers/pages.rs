use askama::Template;
use axum::response::{Html, IntoResponse, Response};

use crate::domain::models::AppError;

#[derive(Template)]
#[template(path = "index.html")]
struct IndexTemplate {}

pub async fn index_page() -> Result<Response, AppError> {
    let template = IndexTemplate {};
    Ok(Html(
        template
            .render()
            .map_err(|e| AppError::Internal(e.to_string()))?,
    )
    .into_response())
}

#[derive(Template)]
#[template(path = "dashboard.html")]
struct DashboardTemplate {
    pub username: String,
}

pub async fn dashboard_page(
    axum::Extension(user): axum::Extension<crate::domain::models::User>,
) -> Result<Response, AppError> {
    let template = DashboardTemplate {
        username: user.username,
    };
    Ok(Html(
        template
            .render()
            .map_err(|e| AppError::Internal(e.to_string()))?,
    )
    .into_response())
}

pub async fn health() -> &'static str {
    "OK"
}

#[derive(Template)]
#[template(path = "admin.html")]
struct AdminTemplate {}

pub async fn admin_page() -> Result<Response, AppError> {
    let template = AdminTemplate {};
    Ok(Html(
        template
            .render()
            .map_err(|e| AppError::Internal(e.to_string()))?,
    )
    .into_response())
}

pub async fn license_text() -> &'static str {
    include_str!("../../../LICENSE")
}
