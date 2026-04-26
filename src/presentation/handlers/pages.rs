use axum::response::{IntoResponse, Html};
use askama::Template;

#[derive(Template)]
#[template(path = "index.html")]
struct IndexTemplate {}

pub async fn index_page() -> impl IntoResponse {
    let template = IndexTemplate {};
    Html(template.render().unwrap())
}

#[derive(Template)]
#[template(path = "dashboard.html")]
struct DashboardTemplate {
    pub username: String,
}

pub async fn dashboard_page(axum::Extension(user): axum::Extension<crate::domain::models::User>) -> impl IntoResponse {
    let template = DashboardTemplate {
        username: user.username,
    };
    Html(template.render().unwrap())
}

pub async fn health() -> &'static str {
    "OK"
}

#[derive(Template)]
#[template(path = "admin.html")]
struct AdminTemplate {}

pub async fn admin_page() -> impl IntoResponse {
    let template = AdminTemplate {};
    Html(template.render().unwrap())
}

pub async fn license_text() -> &'static str {
    include_str!("../../../LICENSE")
}
