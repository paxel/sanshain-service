use axum::{
    extract::{State, Path},
    response::{IntoResponse, Html},
};
use crate::AppState;
use crate::application::services::{self, AppError};
use crate::domain::models::*;
use askama::Template;

#[derive(Template)]
#[template(path = "fragments/admin/users.html")]
struct UsersTemplate {
    users: Vec<User>,
}

pub async fn fragment_users(State(state): State<AppState>) -> Result<impl IntoResponse, AppError> {
    let users = services::list_users(&state.repo).await?;
    let tmpl = UsersTemplate { users };
    Ok(Html(tmpl.render().unwrap()))
}

pub async fn fragment_approve_user(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Result<impl IntoResponse, AppError> {
    services::approve_user(&state.repo, id).await?;
    fragment_users(State(state)).await
}

pub async fn fragment_delete_user(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Result<impl IntoResponse, AppError> {
    services::admin_delete_user(&state.repo, id).await?;
    fragment_users(State(state)).await
}

#[derive(Template)]
#[template(path = "fragments/admin/services.html")]
struct ServicesTemplate {
    services: Vec<ServiceSummary>,
}

pub async fn fragment_services(State(state): State<AppState>) -> Result<impl IntoResponse, AppError> {
    let services = services::list_services_detailed(&state.repo).await?;
    let tmpl = ServicesTemplate { services };
    Ok(Html(tmpl.render().unwrap()))
}

#[derive(Template)]
#[template(path = "fragments/admin/dev_mode.html")]
struct DevModeTemplate {
    pub enabled: bool,
}

pub async fn fragment_dev_mode(State(state): State<AppState>) -> Result<impl IntoResponse, AppError> {
    let enabled = services::get_dev_mode(&state.repo).await?;
    let tmpl = DevModeTemplate { enabled };
    Ok(Html(tmpl.render().unwrap()))
}

pub async fn fragment_dev_mode_toggle(State(state): State<AppState>) -> Result<impl IntoResponse, AppError> {
    let current = services::get_dev_mode(&state.repo).await?;
    services::set_dev_mode(&state.repo, !current).await?;
    fragment_dev_mode(State(state)).await
}

#[derive(Template)]
#[template(path = "fragments/admin/database_info.html")]
struct DatabaseInfoTemplate {
    pub backend: String,
    pub url: String,
}

pub async fn fragment_database_info(State(state): State<AppState>) -> Result<impl IntoResponse, AppError> {
    let backend = if state.db_url.starts_with("sqlite:") { "SQLite" } else { "PostgreSQL" };
    let tmpl = DatabaseInfoTemplate { 
        backend: backend.to_string(),
        url: state.db_url.clone(),
    };
    Ok(Html(tmpl.render().unwrap()))
}
