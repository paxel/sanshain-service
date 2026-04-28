use crate::AppState;
use crate::application::services::{self, AppError};
use crate::domain::models::User;
use axum::{Json, extract::State, http::StatusCode, response::IntoResponse};
use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
pub struct LoginRequest {
    pub username: String,
    pub password: String,
}

#[derive(Serialize)]
pub struct LoginResponse {
    pub token: String,
    pub is_admin: bool,
}

#[derive(Serialize)]
pub struct ChangePasswordResponse {
    pub token: String,
}

pub async fn auth_login(
    State(state): State<AppState>,
    Json(payload): Json<LoginRequest>,
) -> Result<impl IntoResponse, AppError> {
    let session = services::login(&state.repo, &payload.username, &payload.password).await?;
    let user = services::list_users(&state.repo)
        .await?
        .into_iter()
        .find(|u| u.id == session.user_id)
        .ok_or_else(|| AppError::Internal("User not found after login".to_string()))?;

    Ok(Json(LoginResponse {
        token: session.token,
        is_admin: user.is_admin,
    }))
}

pub async fn auth_logout(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
) -> Result<impl IntoResponse, AppError> {
    if let Some(auth_header) = headers.get("Authorization")
        && let Ok(auth_str) = auth_header.to_str()
        && let Some(token) = auth_str.strip_prefix("Bearer ")
    {
        services::logout(&state.repo, token).await?;
    }
    Ok(StatusCode::OK)
}

pub async fn auth_me(
    axum::Extension(user): axum::Extension<User>,
) -> Result<impl IntoResponse, AppError> {
    Ok(Json(user))
}

#[derive(Deserialize)]
pub struct ChangePasswordRequest {
    pub old_password: String,
    pub new_password: String,
}

pub async fn auth_change_password(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    axum::Extension(user): axum::Extension<User>,
    Json(payload): Json<ChangePasswordRequest>,
) -> Result<impl IntoResponse, AppError> {
    let current_token = headers
        .get("Authorization")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "));

    let session = services::change_password(
        &state.repo,
        &user,
        current_token,
        &payload.old_password,
        &payload.new_password,
    )
    .await?;

    match session {
        Some(session) => Ok((
            StatusCode::OK,
            Json(ChangePasswordResponse {
                token: session.token,
            }),
        )),
        None => Err(AppError::Internal(
            "Password updated without an authenticated session token".to_string(),
        )),
    }
}

pub async fn auth_register(
    State(state): State<AppState>,
    Json(payload): Json<LoginRequest>,
) -> Result<impl IntoResponse, AppError> {
    services::register_user(&state.repo, &payload.username, &payload.password).await?;
    Ok(StatusCode::CREATED)
}

pub async fn list_tokens(
    State(state): State<AppState>,
    axum::Extension(user): axum::Extension<User>,
) -> Result<impl IntoResponse, AppError> {
    let tokens = services::list_api_tokens(&state.repo, user.id).await?;
    Ok(Json(tokens))
}

#[derive(Deserialize)]
pub struct CreateTokenRequest {
    pub name: String,
    pub expires_in_days: u64,
}

#[derive(Serialize)]
pub struct CreateTokenResponse {
    pub id: String,
    pub name: String,
    pub token: String,
}

#[derive(Serialize)]
pub struct CsrfResponse {
    pub csrf_token: String,
}

pub async fn get_csrf_token(State(state): State<AppState>) -> impl IntoResponse {
    use rand::distr::{Alphanumeric, SampleString};
    let token = Alphanumeric.sample_string(&mut rand::rng(), 32);
    let expires_at = chrono::Utc::now() + chrono::Duration::hours(1);

    let mut tokens = state.csrf_tokens.write().await;
    tokens.insert(token.clone(), expires_at);
    // Optional: cleanup old tokens
    if tokens.len() > 1000 {
        let now = chrono::Utc::now();
        tokens.retain(|_, v| *v > now);
    }

    Json(CsrfResponse { csrf_token: token })
}

pub async fn create_token(
    State(state): State<AppState>,
    axum::Extension(user): axum::Extension<User>,
    Json(payload): Json<CreateTokenRequest>,
) -> Result<impl IntoResponse, AppError> {
    let (id, token) =
        services::create_api_token(&state.repo, user.id, &payload.name, payload.expires_in_days)
            .await?;
    Ok(Json(CreateTokenResponse {
        id,
        name: payload.name.clone(),
        token,
    }))
}

pub async fn revoke_token(
    State(state): State<AppState>,
    axum::Extension(user): axum::Extension<User>,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> Result<impl IntoResponse, AppError> {
    services::revoke_api_token(&state.repo, &id, user.id).await?;
    Ok(StatusCode::OK)
}
