use crate::AppState;
use crate::application::services::{self, AppError};
use crate::domain::models::{ApiTokenScope, User};
use crate::domain::ports::NewAuditLog;
use axum::{
    Json,
    extract::{Query, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Redirect},
};
use serde::{Deserialize, Serialize};

use super::record_audit_log_for as record_audit_log;

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
    let directory = match services::sign_in_source(&state.repo).await? {
        crate::domain::models::AuthMode::Ldap => services::get_ldap_config(&state.repo)
            .await?
            .map(crate::infrastructure::ldap_provider::LdapAuthProvider::new),
        _ => None,
    };
    let verified = services::verify_password_local_first(
        &state.repo,
        directory.as_ref(),
        &payload.username,
        &payload.password,
    )
    .await?;
    if services::admins_only(&mode) {
        let existing = match &verified {
            services::Verified::Local(user) | services::Verified::Directory(Some(user), _) => {
                Some(user)
            }
            services::Verified::Directory(None, _) => None,
        };
        let groups =
            crate::presentation::middleware::directory_groups_for(&state, &payload.username).await;
        if !services::may_sign_in_as_admin(
            &state.repo,
            &state.root_users,
            &payload.username,
            existing,
            &groups,
        )
        .await?
        {
            return Err(services::admins_only_refusal(&mode));
        }
    }
    let session = services::start_session(&state.repo, &payload.username, verified).await?;
    Ok(Json(LoginResponse {
        token: session.token,
    }))
}

/// What the login page offers: registration, SSO, and whether only
/// administrators can sign in. Public, like the login itself.
pub async fn auth_options(State(state): State<AppState>) -> Result<impl IntoResponse, AppError> {
    Ok(Json(services::login_options(&state.repo).await?))
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
    headers: axum::http::HeaderMap,
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
    let profile = services::user_profile(&state.repo, user.id).await?;
    // How this session signed in, so the pages offer a password change only
    // where the password was Sanshain's own. Null for an older session.
    let sign_in = match headers
        .get("Authorization")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
    {
        Some(token) => services::session_sign_in(&state.repo, token).await?,
        None => None,
    };

    Ok(Json(serde_json::json!({
        "id": user.id,
        "username": user.username,
        "approved": user.approved,
        "roles": roles,
        "permissions": permissions,
        "is_root": is_root,
        "maintains": maintains,
        "sign_in": sign_in,
        "display_name_editable": !profile.display_name_from_provider(),
        "gravatar_hash": if services::get_gravatar_enabled(&state.repo).await? {
            profile.avatar_hash.clone()
        } else {
            None
        },
        "display_name": profile.display_name,
    })))
}

#[derive(Deserialize)]
pub struct DisplayNameRequest {
    pub display_name: String,
}

/// Set the caller's own display name; blank clears it. Refused when the name
/// comes from the directory or the OIDC provider.
pub async fn auth_set_display_name(
    State(state): State<AppState>,
    axum::Extension(user): axum::Extension<User>,
    Json(payload): Json<DisplayNameRequest>,
) -> Result<impl IntoResponse, AppError> {
    let display_name =
        services::set_own_display_name(&state.repo, &user, &payload.display_name).await?;
    Ok(Json(serde_json::json!({ "display_name": display_name })))
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
                    action_type: Some("ADMIN"),
                    ..Default::default()
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
    let approved = services::register_user(
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
            action_type: Some("ADMIN"),
            ..Default::default()
        },
    )
    .await?;
    Ok((
        StatusCode::CREATED,
        Json(serde_json::json!({ "approved": approved })),
    ))
}

pub async fn list_tokens(
    State(state): State<AppState>,
    axum::Extension(actor): axum::Extension<crate::domain::permissions::Actor>,
) -> Result<impl IntoResponse, AppError> {
    Ok(Json(services::list_tokens_for(&state.repo, &actor).await?))
}

#[derive(Deserialize)]
pub struct CreateTokenRequest {
    pub name: String,
    pub expires_in_days: u64,
    /// Immutable after creation; `full` when omitted.
    #[serde(default)]
    pub scope: ApiTokenScope,
    /// A service account to issue the token for (`manage_users`); the
    /// caller's own token when absent.
    #[serde(default)]
    pub for_user: Option<String>,
}

#[derive(Serialize)]
pub struct CreateTokenResponse {
    pub id: String,
    pub name: String,
    pub scope: ApiTokenScope,
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
    axum::Extension(actor): axum::Extension<crate::domain::permissions::Actor>,
    Json(payload): Json<CreateTokenRequest>,
) -> Result<impl IntoResponse, AppError> {
    let (id, token) = services::issue_token(
        &state.repo,
        &actor,
        services::TokenRequest {
            name: &payload.name,
            expires_in_days: payload.expires_in_days,
            scope: payload.scope,
            for_user: payload.for_user.as_deref(),
        },
    )
    .await?;
    let details = match payload.for_user.as_deref().filter(|u| !u.is_empty()) {
        Some(account) => format!("Issued an API token for service account '{account}'"),
        None => "Created an API token".to_string(),
    };
    record_audit_log(
        &state.repo,
        Some(&user),
        NewAuditLog {
            action: "CREATE_TOKEN",
            details: &details,
            action_type: Some("ADMIN"),
            ..Default::default()
        },
    )
    .await?;
    Ok(Json(CreateTokenResponse {
        id,
        name: payload.name.clone(),
        scope: payload.scope,
        token,
    }))
}

pub async fn revoke_token(
    State(state): State<AppState>,
    axum::Extension(user): axum::Extension<User>,
    axum::Extension(actor): axum::Extension<crate::domain::permissions::Actor>,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> Result<impl IntoResponse, AppError> {
    if !services::revoke_token_as(&state.repo, &actor, &id).await? {
        return Err(AppError::NotFound("API token not found".to_string()));
    }
    record_audit_log(
        &state.repo,
        Some(&user),
        NewAuditLog {
            action: "REVOKE_TOKEN",
            details: "Revoked an API token",
            action_type: Some("ADMIN"),
            ..Default::default()
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

// --- OIDC human login (ai/improvements grill batch 2) ---

const OIDC_FLOW_COOKIE: &str = "sanshain_oidc_flow";

#[derive(Deserialize)]
pub struct OidcCallbackQuery {
    pub code: Option<String>,
    pub state: Option<String>,
    pub error: Option<String>,
}

fn cookie_value(headers: &HeaderMap, name: &str) -> Option<String> {
    let raw = headers.get(header::COOKIE)?.to_str().ok()?;
    raw.split(';').find_map(|kv| {
        let (k, v) = kv.trim().split_once('=')?;
        (k == name).then(|| v.to_string())
    })
}

/// Begin OIDC login: stash the flow state (CSRF state, nonce, PKCE verifier)
/// in a short-lived HttpOnly cookie, then redirect the browser to the
/// provider. The flow itself lives in `services::oidc_begin`.
pub async fn oidc_login(State(state): State<AppState>) -> Result<impl IntoResponse, AppError> {
    let (url, flow) = services::oidc_begin(
        &state.repo,
        &crate::infrastructure::oidc_provider::OidcProvider,
    )
    .await?;
    // Secure: OIDC runs over HTTPS in production (providers require an https
    // redirect); the flow cookie carrying the PKCE verifier/nonce must not go
    // over plaintext.
    let cookie = format!(
        "{OIDC_FLOW_COOKIE}={}; Path=/; HttpOnly; Secure; SameSite=Lax; Max-Age=600",
        flow.encode()
    );
    let mut response = Redirect::to(&url).into_response();
    response.headers_mut().insert(
        header::SET_COOKIE,
        HeaderValue::from_str(&cookie).map_err(|_| AppError::Internal("cookie".to_string()))?,
    );
    Ok(response)
}

/// OIDC callback: decode the flow cookie and hand the round-trip's second half
/// to `services::oidc_complete` (state check, code exchange, ID-token
/// verification, session mint); then set the `sanshain_token` cookie and
/// redirect into the app.
pub async fn oidc_callback(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<OidcCallbackQuery>,
) -> Result<impl IntoResponse, AppError> {
    if query.error.is_some() {
        return Err(AppError::Unauthorized);
    }
    let code = query
        .code
        .ok_or_else(|| AppError::BadRequest("missing code".to_string()))?;
    let cb_state = query
        .state
        .ok_or_else(|| AppError::BadRequest("missing state".to_string()))?;

    let raw_flow = cookie_value(&headers, OIDC_FLOW_COOKIE)
        .ok_or_else(|| AppError::BadRequest("no OIDC login in progress".to_string()))?;
    let flow = services::OidcFlowState::decode(&raw_flow).ok_or(AppError::Unauthorized)?;
    let session = services::oidc_complete(
        &state.repo,
        &state.root_users,
        &crate::infrastructure::oidc_provider::OidcProvider,
        code,
        &cb_state,
        flow,
    )
    .await?;

    // Non-HttpOnly so the SPA's getSanshainToken() can read it (matches the
    // existing token model); Secure because OIDC is HTTPS; clear the flow cookie.
    let token_cookie = format!(
        "sanshain_token={}; Path=/; Secure; SameSite=Lax; Max-Age=604800",
        session.token
    );
    let clear = format!("{OIDC_FLOW_COOKIE}=; Path=/; HttpOnly; Max-Age=0");
    let mut response = Redirect::to("/").into_response();
    let h = response.headers_mut();
    h.append(
        header::SET_COOKIE,
        HeaderValue::from_str(&token_cookie)
            .map_err(|_| AppError::Internal("cookie".to_string()))?,
    );
    h.append(
        header::SET_COOKIE,
        HeaderValue::from_str(&clear).map_err(|_| AppError::Internal("cookie".to_string()))?,
    );
    Ok(response)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cookie_value_extracts_the_named_cookie() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::COOKIE,
            HeaderValue::from_static("a=1; sanshain_oidc_flow=st|no|pk; b=2"),
        );
        assert_eq!(
            cookie_value(&headers, OIDC_FLOW_COOKIE).as_deref(),
            Some("st|no|pk")
        );
        assert_eq!(cookie_value(&headers, "missing"), None);
        assert_eq!(cookie_value(&HeaderMap::new(), OIDC_FLOW_COOKIE), None);
    }
}
