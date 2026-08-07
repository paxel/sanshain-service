use crate::AppState;
use crate::application::services::{self, AppError};
use crate::domain::models::User;
use crate::domain::ports::{NewAuditLog, SpecRepository};
use axum::{Json, extract::State, http::StatusCode, response::IntoResponse};
use serde::{Deserialize, Serialize};

async fn record_audit_log(
    repo: &impl crate::domain::ports::SpecRepository,
    user: Option<&User>,
    log: NewAuditLog<'_>,
) -> Result<(), AppError> {
    let actor = if let Some(u) = user {
        u.username.clone()
    } else {
        "DevMode/Anonymous".to_string()
    };
    repo.insert_audit_log(&actor, log)
        .await
        .map_err(|e| AppError::Internal(e.to_string()))
}

#[derive(Deserialize)]
pub struct LoginRequest {
    pub username: String,
    pub password: String,
}

#[derive(Serialize)]
/// The result of a successful login.
///
/// Carries the session token only. What the caller may do is answered by
/// `/auth/me`, which reports the permissions they actually hold — a single
/// boolean here could not express a partial administrator.
pub struct LoginResponse {
    pub token: String,
}

#[derive(Serialize)]
pub struct ChangePasswordResponse {
    pub token: String,
}

pub async fn auth_login(
    State(state): State<AppState>,
    Json(payload): Json<LoginRequest>,
) -> Result<impl IntoResponse, AppError> {
    let mode = services::get_auth_mode(&state.repo).await?;

    match mode {
        crate::domain::models::AuthMode::Disabled => Err(AppError::Forbidden),
        crate::domain::models::AuthMode::Ldap => {
            if let Ok(Some(config)) = services::get_ldap_config(&state.repo).await {
                let provider = crate::infrastructure::ldap_provider::LdapAuthProvider::new(config);
                // Try LDAP login
                if let Ok(session) = services::login_with_provider(
                    &state.repo,
                    &provider,
                    &payload.username,
                    &payload.password,
                )
                .await
                {
                    // The local record must exist by now — the provider login
                    // creates one — and confirming it here keeps a session from
                    // being handed out for a user nothing else can resolve.
                    state
                        .repo
                        .find_user(&payload.username)
                        .await?
                        .ok_or(AppError::Internal("Shadow user missing".to_string()))?;
                    return Ok(Json(LoginResponse {
                        token: session.token,
                    }));
                }
            }
            // Fallback to local login for root or if LDAP fails
            let (session, _user) =
                services::login(&state.repo, &payload.username, &payload.password).await?;
            Ok(Json(LoginResponse {
                token: session.token,
            }))
        }
        crate::domain::models::AuthMode::Local | crate::domain::models::AuthMode::Dev => {
            let (session, _user) =
                services::login(&state.repo, &payload.username, &payload.password).await?;

            Ok(Json(LoginResponse {
                token: session.token,
            }))
        }
    }
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

/// The signed-in caller, with what they may do.
///
/// The UI gates on `permissions` rather than on role names, so it does not
/// encode the role bundles a second time and cannot drift from what the server
/// enforces. `roles` is reported for display only.
pub async fn auth_me(
    State(state): State<AppState>,
    axum::Extension(user): axum::Extension<User>,
    actor: Option<axum::Extension<crate::domain::permissions::Actor>>,
) -> Result<impl IntoResponse, AppError> {
    let (roles, permissions, is_root) = match &actor {
        Some(axum::Extension(actor)) => (
            actor
                .roles
                .iter()
                .map(|r| r.as_str().to_string())
                .collect::<Vec<_>>(),
            crate::application::authz::permission_names(actor),
            actor.is_root,
        ),
        None => (Vec::new(), Vec::new(), false),
    };

    // The Producers this caller maintains, so pages tied to one Producer (the
    // endpoint editor) can admit a maintainer whose permission is scoped rather
    // than global. Root is not expanded: it maintains everything, and the UI
    // already knows that from `is_root`. Grants made to the caller's directory
    // groups are included, matching what the server would actually allow.
    let maintains = match &actor {
        Some(axum::Extension(actor)) => {
            crate::application::authz::maintained_producers_for_actor(&state.repo, actor).await?
        }
        None => crate::application::authz::maintained_producers(&state.repo, user.id).await?,
    };

    Ok(Json(serde_json::json!({
        "id": user.id,
        "username": user.username,
        "approved": user.approved,
        "roles": roles,
        "permissions": permissions,
        "is_root": is_root,
        "maintains": maintains,
    })))
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
        Some(session) => {
            record_audit_log(
                &state.repo,
                Some(&user),
                NewAuditLog {
                    action: "CHANGE_PASSWORD",
                    details: "Successfully changed user password",
                    service: None,
                    version: None,
                    action_type: Some("ADMIN"),
                    diff: None,
                },
            )
            .await?;
            Ok((
                StatusCode::OK,
                Json(ChangePasswordResponse {
                    token: session.token,
                }),
            ))
        }
        None => Err(AppError::Internal(
            "Password updated without an authenticated session token".to_string(),
        )),
    }
}

pub async fn auth_register(
    State(state): State<AppState>,
    Json(payload): Json<LoginRequest>,
) -> Result<impl IntoResponse, AppError> {
    services::register_user(
        &state.repo,
        &state.root_users,
        &payload.username,
        &payload.password,
    )
    .await?;
    record_audit_log(
        &state.repo,
        None,
        NewAuditLog {
            action: "REGISTER_USER",
            details: &format!("Registered user '{}'", payload.username),
            service: None,
            version: None,
            action_type: Some("ADMIN"),
            diff: None,
        },
    )
    .await?;
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
    record_audit_log(
        &state.repo,
        Some(&user),
        NewAuditLog {
            action: "CREATE_TOKEN",
            details: "Created an API token",
            service: None,
            version: None,
            action_type: Some("ADMIN"),
            diff: None,
        },
    )
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
    if !services::revoke_api_token(&state.repo, &id, user.id).await? {
        return Err(AppError::NotFound("API token not found".to_string()));
    }
    record_audit_log(
        &state.repo,
        Some(&user),
        NewAuditLog {
            action: "REVOKE_TOKEN",
            details: "Revoked an API token",
            service: None,
            version: None,
            action_type: Some("ADMIN"),
            diff: None,
        },
    )
    .await?;
    Ok(StatusCode::OK)
}

pub async fn get_favorites(
    State(state): State<AppState>,
    axum::Extension(user): axum::Extension<User>,
) -> Result<impl IntoResponse, AppError> {
    let res = services::get_user_favorites(&state.repo, user.id).await?;
    Ok(Json(res))
}

pub async fn add_favorite(
    State(state): State<AppState>,
    axum::Extension(user): axum::Extension<User>,
    axum::extract::Path((item_type, item_name)): axum::extract::Path<(String, String)>,
) -> Result<impl IntoResponse, AppError> {
    services::add_user_favorite(&state.repo, user.id, &item_type, &item_name).await?;
    Ok(StatusCode::OK)
}

pub async fn remove_favorite(
    State(state): State<AppState>,
    axum::Extension(user): axum::Extension<User>,
    axum::extract::Path((item_type, item_name)): axum::extract::Path<(String, String)>,
) -> Result<impl IntoResponse, AppError> {
    services::remove_user_favorite(&state.repo, user.id, &item_type, &item_name).await?;
    Ok(StatusCode::OK)
}
