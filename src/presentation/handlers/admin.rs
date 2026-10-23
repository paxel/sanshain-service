use crate::AppState;
use crate::application::services::{self, AppError};
use crate::domain::models::{ApiType, AuditLogEntry, AuthMode, LdapConfig, OidcConfig};
use crate::domain::ports::{NewAuditLog, SpecRepository};
use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
};
use serde::Deserialize;
use serde_json::json;
use std::str::FromStr;

use super::{DEV_MODE_ACTOR, actor_or_dev_mode, record_audit_log};

pub async fn admin_list_producers(
    State(state): State<AppState>,
    user: Option<axum::Extension<crate::domain::models::User>>,
) -> Result<impl IntoResponse, AppError> {
    let user_id = user.map(|axum::Extension(u)| u.id);
    let res = services::list_producers_detailed(&state.repo, user_id).await?;
    Ok(Json(res))
}

pub async fn admin_list_consumers(
    State(state): State<AppState>,
    user: Option<axum::Extension<crate::domain::models::User>>,
) -> Result<impl IntoResponse, AppError> {
    let user_id = user.map(|axum::Extension(u)| u.id);
    let res = services::list_consumers(&state.repo, user_id).await?;
    Ok(Json(res))
}

/// `GET /admin/producers/{name}` — the Producer's context (#48): owner,
/// description, source location and links from its catalog-info.
pub async fn admin_get_producer(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    Ok(Json(services::producer_context(&state.repo, &name).await?))
}

/// `GET /admin/consumers/{name}` — the Consumer's context (#48).
pub async fn admin_get_consumer(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    Ok(Json(services::consumer_context(&state.repo, &name).await?))
}

pub async fn admin_list_consumer_endpoints(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    let res = services::list_consumer_endpoints(&state.repo, &name).await?;
    Ok(Json(res))
}

#[derive(Deserialize)]
pub struct VersionedSpecQuery {
    pub api_type: Option<ApiType>,
    pub version: crate::domain::models::SemVer,
}

pub async fn admin_list_producer_endpoints(
    State(state): State<AppState>,
    Path(name): Path<String>,
    Query(query): Query<VersionedSpecQuery>,
) -> Result<impl IntoResponse, AppError> {
    let res = services::list_producer_endpoints_described(
        &state.repo,
        &name,
        query.api_type.unwrap_or(ApiType::OpenApi),
        query.version,
    )
    .await?;
    Ok(Json(res))
}

pub async fn admin_list_producer_versions(
    State(state): State<AppState>,
    Path(name): Path<String>,
    Query(query): Query<FullSpecQuery>,
) -> Result<impl IntoResponse, AppError> {
    let res = services::list_producer_versions(&state.repo, &name, query.api_type).await?;
    Ok(Json(res))
}

#[derive(Deserialize)]
pub struct DiffQuery {
    pub api_type: Option<ApiType>,
    pub from: crate::domain::models::SemVer,
    pub to: crate::domain::models::SemVer,
    /// Diff one endpoint's snippet instead of the whole document; both or neither.
    pub path: Option<String>,
    pub method: Option<String>,
    #[serde(default)]
    pub ignore_whitespace: bool,
    /// Unchanged lines around each change (default 3), or `all`.
    pub context: Option<String>,
    /// `text` (default, a unified-diff patch) or `json` (structured hunks).
    pub format: Option<String>,
}

/// Diff between two versions of a line (default adjacent-semantic is the UI's
/// business — the API takes the two explicit endpoints of the diff).
pub async fn admin_diff_versions(
    State(state): State<AppState>,
    Path(name): Path<String>,
    Query(query): Query<DiffQuery>,
) -> Result<axum::response::Response, AppError> {
    let endpoint = match (query.path.as_deref(), query.method.as_deref()) {
        (Some(path), Some(method)) => Some((path, method)),
        (None, None) => None,
        _ => {
            return Err(AppError::BadRequest(
                "path and method go together: give both to diff an endpoint, or neither".into(),
            ));
        }
    };
    let context = match query.context.as_deref() {
        None => Some(3),
        Some("all") => None,
        Some(n) => Some(n.parse::<usize>().map_err(|_| {
            AppError::BadRequest(format!(
                "context must be a non-negative number of lines or 'all', not '{n}'"
            ))
        })?),
    };
    let json = match query.format.as_deref() {
        None | Some("text") => false,
        Some("json") => true,
        Some(other) => {
            return Err(AppError::BadRequest(format!(
                "format must be 'text' or 'json', not '{other}'"
            )));
        }
    };
    let request = services::DiffRequest {
        api_type: query.api_type.unwrap_or(ApiType::OpenApi),
        from: query.from,
        to: query.to,
        endpoint,
        options: services::DiffOptions {
            ignore_whitespace: query.ignore_whitespace,
            context,
        },
    };
    let diff = services::diff_versions(&state.repo, &name, &request).await?;
    if json {
        return Ok(Json(diff).into_response());
    }
    let patch = services::render_unified(
        &diff.hunks,
        &format!("{} {}", name, diff.from),
        &format!("{} {}", name, diff.to),
    );
    Ok((
        [(
            axum::http::header::CONTENT_TYPE,
            "text/plain; charset=utf-8",
        )],
        patch,
    )
        .into_response())
}

#[derive(Deserialize)]
pub struct BlameQuery {
    pub api_type: Option<ApiType>,
    pub version: crate::domain::models::SemVer,
    /// Blame one endpoint's snippet instead of the whole document; both or neither.
    pub path: Option<String>,
    pub method: Option<String>,
}

/// Blame blocks of one version of a line (whole document or one endpoint).
pub async fn admin_blame_version(
    State(state): State<AppState>,
    Path(name): Path<String>,
    Query(query): Query<BlameQuery>,
) -> Result<impl IntoResponse, AppError> {
    let endpoint = match (query.path.as_deref(), query.method.as_deref()) {
        (Some(path), Some(method)) => Some((path, method)),
        (None, None) => None,
        _ => {
            return Err(AppError::BadRequest(
                "path and method go together: give both to blame an endpoint, or neither".into(),
            ));
        }
    };
    let blocks = services::blame_version(
        &state.repo,
        &name,
        query.api_type.unwrap_or(ApiType::OpenApi),
        query.version,
        endpoint,
    )
    .await?;
    Ok(Json(blocks))
}

#[derive(Deserialize)]
pub struct StructureQuery {
    pub api_type: Option<ApiType>,
    pub version: crate::domain::models::SemVer,
    /// One endpoint with its children instead of the whole document; both or neither.
    pub path: Option<String>,
    pub method: Option<String>,
}

/// The structure view of one version: every endpoint without its children,
/// or one endpoint with them and an example per body.
pub async fn admin_structure_version(
    State(state): State<AppState>,
    Path(name): Path<String>,
    Query(query): Query<StructureQuery>,
) -> Result<impl IntoResponse, AppError> {
    let endpoint = match (query.path.as_deref(), query.method.as_deref()) {
        (Some(path), Some(method)) => Some((path, method)),
        (None, None) => None,
        _ => {
            return Err(AppError::BadRequest(
                "path and method go together: give both for one endpoint, or neither".into(),
            ));
        }
    };
    let view = services::structure_version(
        &state.repo,
        &name,
        services::StructureRequest {
            api_type: query.api_type.unwrap_or(ApiType::OpenApi),
            version: query.version,
            endpoint,
            with_examples: true,
        },
    )
    .await?;
    Ok(Json(view))
}

#[derive(Deserialize)]
pub struct FullSpecQuery {
    pub api_type: Option<ApiType>,
}

pub async fn admin_get_full_spec(
    State(state): State<AppState>,
    Path(name): Path<String>,
    Query(query): Query<VersionedSpecQuery>,
) -> Result<impl IntoResponse, AppError> {
    let api_type = query.api_type.unwrap_or(ApiType::OpenApi);
    let spec = services::get_full_spec(&state.repo, &name, api_type, query.version).await?;
    // Served as a download: name the file after the service/version it came
    // from, and use the media type matching the stored document.
    let extension = match api_type {
        ApiType::Proto => "proto",
        _ => "yaml",
    };
    let content_type = match api_type {
        ApiType::Proto => "text/plain; charset=utf-8",
        _ => "application/yaml; charset=utf-8",
    };
    let filename = format!(
        "{}-{}.{}",
        sanitize_filename_part(&name),
        sanitize_filename_part(&query.version.to_string()),
        extension
    );
    Ok((
        [
            (axum::http::header::CONTENT_TYPE, content_type.to_string()),
            (
                axum::http::header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"{}\"", filename),
            ),
        ],
        spec,
    ))
}

/// Reduce a service or version name to characters that are safe in a
/// `Content-Disposition` filename. Service names are client-supplied
/// and unvalidated, so anything outside this set — quotes, path separators, and
/// notably control characters like CR/LF, which would make the header value
/// invalid and fail the response — becomes `-`. Whitelisted rather than
/// blacklisted so a character nobody thought of cannot slip through.
fn sanitize_filename_part(value: &str) -> String {
    let cleaned: String = value
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
                c
            } else {
                '-'
            }
        })
        .collect();
    if cleaned.is_empty() {
        "spec".to_string()
    } else {
        cleaned
    }
}

#[derive(Deserialize)]
pub struct AdminEndpointYamlQuery {
    pub producername: String,
    pub version: crate::domain::models::SemVer,
    pub api_type: ApiType,
    pub path: String,
    pub method: String,
}

pub async fn admin_get_endpoint_yaml(
    State(state): State<AppState>,
    Query(query): Query<AdminEndpointYamlQuery>,
) -> Result<impl IntoResponse, AppError> {
    let res = services::get_endpoint_yaml(
        &state.repo,
        &query.producername,
        query.version,
        query.api_type,
        &query.path,
        &query.method,
    )
    .await?;
    // Always 200 with a discriminated body: the view is told what happened
    // (served / absent / unknown) and which stability served it, instead of
    // having to infer a state from an error.
    Ok(Json(res))
}

#[derive(Deserialize)]
pub struct AdminEndpointHistoryQuery {
    pub producername: String,
    pub api_type: ApiType,
    pub path: String,
    pub method: String,
}

pub async fn admin_get_endpoint_versions(
    State(state): State<AppState>,
    Query(query): Query<AdminEndpointHistoryQuery>,
) -> Result<impl IntoResponse, AppError> {
    let res = services::get_endpoint_history(
        &state.repo,
        &query.producername,
        query.api_type,
        &query.path,
        &query.method,
    )
    .await?;
    Ok(Json(res))
}

pub async fn admin_delete_producer(
    State(state): State<AppState>,
    user: Option<axum::Extension<crate::domain::models::User>>,
    Path(name): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    if services::delete_producer(&state.repo, &name).await? {
        let _ = state.spec_updated_tx.send(());
        record_audit_log(
            &state.repo,
            user,
            NewAuditLog {
                action: "DELETE_SERVICE",
                details: &format!("Deleted service '{}'", name),
                service: Some(&name),
                action_type: Some("WRITE"),
                ..Default::default()
            },
        )
        .await?;
        Ok(Json(json!({ "deleted": name })))
    } else {
        Err(AppError::NotFound(format!("Service not found: {}", name)))
    }
}

/// The sole escape hatch from GA immutability (ADR-0003). The UI calls the
/// dependents listing first and shows who is pinned before confirming; the
/// delete response names them again so the audit trail is complete.
pub async fn admin_get_version_dependents(
    State(state): State<AppState>,
    Path((name, api_type, version)): Path<(String, String, String)>,
) -> Result<impl IntoResponse, AppError> {
    let api_type = ApiType::from_str(&api_type).map_err(AppError::BadRequest)?;
    let version = crate::domain::models::SemVer::parse_spec_version(&version)
        .map_err(AppError::BadRequest)?;
    let dependents =
        services::list_version_dependents(&state.repo, &name, api_type, version).await?;
    Ok(Json(dependents))
}

/// Release a stored snapshot in place (#28) — the dashboard's one-click
/// promote. Authorisation is the same GA gate every release goes through
/// ([`Permission::ReleaseGa`] inside the provide path), so an unauthorized
/// click gets the instructive 403 plus the `ga_requires_releaser` telemetry a
/// bare route guard could not produce. The shared path also writes the
/// `VERSION_PROMOTED` audit entry; nothing is logged twice here.
///
/// [`Permission::ReleaseGa`]: crate::domain::permissions::Permission::ReleaseGa
pub async fn admin_promote_version(
    State(state): State<AppState>,
    caller: Option<axum::Extension<crate::domain::permissions::Actor>>,
    Path((name, api_type, version)): Path<(String, String, String)>,
) -> Result<impl IntoResponse, AppError> {
    let api_type = ApiType::from_str(&api_type).map_err(AppError::BadRequest)?;
    let version = crate::domain::models::SemVer::parse_spec_version(&version)
        .map_err(AppError::BadRequest)?;
    let res = services::promote_version(
        &state.repo,
        &name,
        api_type,
        version,
        caller.map(|axum::Extension(a)| a),
    )
    .await?;
    // The stability flip is a state change listeners care about even though
    // the content is byte-identical; the no-op case broadcasts nothing.
    if res.promoted {
        let _ = state.spec_updated_tx.send(());
    }
    Ok((StatusCode::ACCEPTED, Json(res)))
}

pub async fn admin_delete_version(
    State(state): State<AppState>,
    user: Option<axum::Extension<crate::domain::models::User>>,
    Path((name, api_type, version)): Path<(String, String, String)>,
) -> Result<impl IntoResponse, AppError> {
    let api_type = ApiType::from_str(&api_type).map_err(AppError::BadRequest)?;
    let version = crate::domain::models::SemVer::parse_spec_version(&version)
        .map_err(AppError::BadRequest)?;
    let dependents = services::delete_version(&state.repo, &name, api_type, version).await?;
    let _ = state.spec_updated_tx.send(());
    let version_str = version.to_string();
    let details = if dependents.is_empty() {
        format!(
            "Deleted {} version {} of '{}' (no consumers were pinned to it)",
            api_type.as_str(),
            version,
            name
        )
    } else {
        format!(
            "Deleted {} version {} of '{}' — consumers pinned to it: {}",
            api_type.as_str(),
            version,
            name,
            dependents.join(", ")
        )
    };
    record_audit_log(
        &state.repo,
        user,
        NewAuditLog {
            action: "DELETE_VERSION",
            details: &details,
            service: Some(&name),
            version: Some(&version_str),
            action_type: Some("WRITE"),
            ..Default::default()
        },
    )
    .await?;
    Ok(Json(
        serde_json::json!({ "deleted": version_str, "dependents": dependents }),
    ))
}

pub async fn admin_delete_consumer(
    State(state): State<AppState>,
    user: Option<axum::Extension<crate::domain::models::User>>,
    Path(name): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    if services::delete_consumer(&state.repo, &name).await? {
        record_audit_log(
            &state.repo,
            user,
            NewAuditLog {
                action: "DELETE_CLIENT",
                details: &format!("Deleted client '{}'", name),
                action_type: Some("WRITE"),
                ..Default::default()
            },
        )
        .await?;
        Ok(Json(json!({ "deleted": name })))
    } else {
        Err(AppError::NotFound(format!("Client not found: {}", name)))
    }
}

#[derive(Deserialize)]
pub struct EnabledRequest {
    pub enabled: bool,
}

pub async fn get_gravatar_enabled(
    State(state): State<AppState>,
) -> Result<impl IntoResponse, AppError> {
    let enabled = services::get_gravatar_enabled(&state.repo).await?;
    Ok(Json(json!({ "gravatar_enabled": enabled })))
}

pub async fn set_gravatar_enabled(
    State(state): State<AppState>,
    user: Option<axum::Extension<crate::domain::models::User>>,
    Json(payload): Json<EnabledRequest>,
) -> Result<impl IntoResponse, AppError> {
    services::set_gravatar_enabled(&state.repo, payload.enabled).await?;
    record_audit_log(
        &state.repo,
        user,
        NewAuditLog {
            action: "SET_GRAVATAR",
            details: &format!(
                "Gravatar pictures {}",
                if payload.enabled { "on" } else { "off" }
            ),
            action_type: Some("ADMIN"),
            ..Default::default()
        },
    )
    .await?;
    Ok(StatusCode::OK)
}

pub async fn get_auto_approve_users(
    State(state): State<AppState>,
) -> Result<impl IntoResponse, AppError> {
    let res = services::get_auto_approve_users(&state.repo).await?;
    Ok(Json(json!({ "auto_approve_users": res })))
}

pub async fn set_auto_approve_users(
    State(state): State<AppState>,
    user: Option<axum::Extension<crate::domain::models::User>>,
    Json(payload): Json<EnabledRequest>,
) -> Result<impl IntoResponse, AppError> {
    services::set_auto_approve_users(&state.repo, payload.enabled).await?;
    record_audit_log(
        &state.repo,
        user,
        NewAuditLog {
            action: "SET_AUTO_APPROVE",
            details: &format!("Set auto-approve-users to {}", payload.enabled),
            action_type: Some("ADMIN"),
            ..Default::default()
        },
    )
    .await?;
    Ok(StatusCode::OK)
}

pub async fn set_registration_enabled(
    State(state): State<AppState>,
    user: Option<axum::Extension<crate::domain::models::User>>,
    Json(payload): Json<EnabledRequest>,
) -> Result<impl IntoResponse, AppError> {
    services::set_registration_enabled(&state.repo, payload.enabled).await?;
    record_audit_log(
        &state.repo,
        user,
        NewAuditLog {
            action: "SET_REGISTRATION",
            details: &format!("Set registration to {}", payload.enabled),
            action_type: Some("ADMIN"),
            ..Default::default()
        },
    )
    .await?;
    Ok(StatusCode::OK)
}

pub async fn get_auth_config(State(state): State<AppState>) -> Result<impl IntoResponse, AppError> {
    let mode = services::get_auth_mode(&state.repo).await?;
    let ldap = services::get_ldap_config(&state.repo).await?;

    let ldap_val = ldap.map(|mut l| {
        if l.bind_password.is_some() {
            l.bind_password = Some("****".to_string());
        }
        l
    });

    let oidc = services::get_oidc_config(&state.repo).await?;
    let oidc_val = oidc.map(|mut o| {
        if o.client_secret.is_some() {
            o.client_secret = Some("****".to_string());
        }
        o
    });

    Ok(Json(serde_json::json!({
        "auth_mode": mode,
        "ldap_config": ldap_val,
        "oidc_config": oidc_val,
    })))
}

#[derive(Deserialize)]
pub struct AuthConfigRequest {
    pub auth_mode: String,
    pub ldap_config: Option<LdapConfig>,
    #[serde(default)]
    pub oidc_config: Option<OidcConfig>,
}

pub async fn set_auth_config(
    State(state): State<AppState>,
    user: Option<axum::Extension<crate::domain::models::User>>,
    Json(payload): Json<AuthConfigRequest>,
) -> Result<impl IntoResponse, AppError> {
    let mode = AuthMode::from_str(&payload.auth_mode)
        .map_err(|_| AppError::BadRequest(format!("Invalid auth mode: {}", payload.auth_mode)))?;

    if mode == AuthMode::Ldap && payload.ldap_config.is_none() {
        return Err(AppError::BadRequest(
            "LDAP config required for ldap mode".to_string(),
        ));
    }
    // Checked only when LDAP is the chosen mode: switching away from LDAP must
    // stay possible even with a broken URL stored.
    if let Some(ldap) = &payload.ldap_config
        && mode == AuthMode::Ldap
    {
        ldap.validate_server_url().map_err(AppError::BadRequest)?;
    }
    if mode == AuthMode::Oidc
        && payload.oidc_config.is_none()
        && services::get_oidc_config(&state.repo).await?.is_none()
    {
        return Err(AppError::BadRequest(
            "OIDC config required for oidc mode".to_string(),
        ));
    }

    // Validated before anything is stored, so a refused form leaves the mode
    // as it was.
    if let Some(oidc) = &payload.oidc_config {
        oidc.validate().map_err(AppError::BadRequest)?;
    }
    services::switch_auth_mode(&state.repo, mode).await?;
    if let Some(mut ldap) = payload.ldap_config {
        if let Some(ref pass) = ldap.bind_password
            && pass == "****"
            && let Ok(Some(old_config)) = services::get_ldap_config(&state.repo).await
        {
            ldap.bind_password = old_config.bind_password;
        }
        services::set_ldap_config(&state.repo, &ldap).await?;
    }
    if let Some(mut oidc) = payload.oidc_config {
        // A masked secret means "keep the stored one" — never overwrite it with
        // the placeholder the read endpoint returns. Propagate a read failure
        // rather than storing the literal "****" as the secret.
        if oidc.client_secret.as_deref() == Some("****")
            && let Some(old) = services::get_oidc_config(&state.repo).await?
        {
            oidc.client_secret = old.client_secret;
        }
        services::set_oidc_config(&state.repo, &oidc).await?;
    }
    record_audit_log(
        &state.repo,
        user,
        NewAuditLog {
            action: "UPDATE_SETTINGS",
            details: &format!(
                "Updated auth mode to '{}' and LDAP configurations",
                payload.auth_mode
            ),
            action_type: Some("ADMIN"),
            ..Default::default()
        },
    )
    .await?;
    Ok(StatusCode::OK)
}

/// Connect and bind with the service account of the submitted configuration.
///
/// A directory that could not be reached at all gets the network diagnosis
/// appended to the `400`, one line each, so the admin sees the container's
/// own view of DNS, routes and the port. Any failure the directory itself
/// answered or stalled on is reported as it is: the network was proven then.
pub async fn test_auth_config(
    State(_state): State<AppState>,
    Json(config): Json<LdapConfig>,
) -> Result<impl IntoResponse, AppError> {
    use crate::domain::ports::{AuthProvider, AuthProviderError};
    use crate::infrastructure::ldap_provider::{LdapAuthProvider, directory_host_and_port};
    use crate::infrastructure::net_diag;

    config.validate_server_url().map_err(AppError::BadRequest)?;
    let server_url = config.server_url.clone();
    let provider = LdapAuthProvider::new(config);
    match provider.test_connection().await {
        Ok(()) => Ok(StatusCode::OK.into_response()),
        Err(AuthProviderError::Unreachable(message)) => {
            let diagnostics = match directory_host_and_port(&server_url) {
                Some((host, port)) => net_diag::diagnose(&host, port).await,
                None => Vec::new(),
            };
            Ok((
                StatusCode::BAD_REQUEST,
                Json(json!({
                    "error": format!("LDAP connection failed: {message}"),
                    "diagnostics": diagnostics,
                })),
            )
                .into_response())
        }
        Err(e) => Err(AppError::BadRequest(format!("LDAP connection failed: {e}"))),
    }
}

/// The ticket URL template, readable by every signed-in user: the pages
/// render ticket ids of a version's origin as links with it.
pub async fn get_ticket_url_template(
    State(state): State<AppState>,
) -> Result<impl IntoResponse, AppError> {
    let template = services::get_ticket_url_template(&state.repo).await?;
    Ok(Json(json!({ "template": template })))
}

#[derive(Deserialize)]
pub struct TicketUrlTemplatePayload {
    pub template: Option<String>,
}

pub async fn set_ticket_url_template(
    State(state): State<AppState>,
    user: Option<axum::Extension<crate::domain::models::User>>,
    Json(payload): Json<TicketUrlTemplatePayload>,
) -> Result<impl IntoResponse, AppError> {
    let template =
        services::set_ticket_url_template(&state.repo, payload.template.as_deref()).await?;
    record_audit_log(
        &state.repo,
        user,
        NewAuditLog {
            action: "SET_TICKET_URL_TEMPLATE",
            details: &match &template {
                Some(t) => format!("Set the ticket URL template to {t}"),
                None => "Cleared the ticket URL template".to_string(),
            },
            action_type: Some("ADMIN"),
            ..Default::default()
        },
    )
    .await?;
    Ok(Json(json!({ "template": template })))
}

pub async fn get_snapshot_max_age(
    State(state): State<AppState>,
) -> Result<impl IntoResponse, AppError> {
    let days = services::get_snapshot_max_age_days(&state.repo).await?;
    Ok(Json(json!({ "days": days })))
}

#[derive(Deserialize)]
pub struct MaxAgeDaysPayload {
    pub days: u64,
}

pub async fn set_snapshot_max_age(
    State(state): State<AppState>,
    user: Option<axum::Extension<crate::domain::models::User>>,
    Json(payload): Json<MaxAgeDaysPayload>,
) -> Result<impl IntoResponse, AppError> {
    services::set_snapshot_max_age_days(&state.repo, payload.days).await?;
    record_audit_log(
        &state.repo,
        user,
        NewAuditLog {
            action: "SET_SNAPSHOT_MAX_AGE",
            details: &format!("Set snapshot max age to {} days", payload.days),
            action_type: Some("ADMIN"),
            ..Default::default()
        },
    )
    .await?;
    Ok(StatusCode::OK)
}

pub async fn trigger_snapshot_cleanup(
    State(state): State<AppState>,
    user: Option<axum::Extension<crate::domain::models::User>>,
) -> Result<impl IntoResponse, AppError> {
    let deleted = services::cleanup_expired_snapshots(&state.repo).await?;
    record_audit_log(
        &state.repo,
        user,
        NewAuditLog {
            action: "SNAPSHOT_CLEANUP",
            details: &format!(
                "Manually triggered snapshot cleanup, removed {} snapshots",
                deleted
            ),
            action_type: Some("ADMIN"),
            ..Default::default()
        },
    )
    .await?;
    Ok(Json(json!({ "deleted": deleted })))
}

pub async fn get_dependency_max_age(
    State(state): State<AppState>,
) -> Result<impl IntoResponse, AppError> {
    let res = services::get_dependency_max_age_days(&state.repo).await?;
    Ok(Json(json!({ "days": res })))
}

pub async fn set_dependency_max_age(
    State(state): State<AppState>,
    user: Option<axum::Extension<crate::domain::models::User>>,
    Json(payload): Json<MaxAgeDaysPayload>,
) -> Result<impl IntoResponse, AppError> {
    if payload.days == 0 {
        return Err(AppError::BadRequest(
            "days must be greater than 0".to_string(),
        ));
    }
    services::set_dependency_max_age_days(&state.repo, payload.days).await?;
    record_audit_log(
        &state.repo,
        user,
        NewAuditLog {
            action: "UPDATE_SETTINGS",
            details: &format!("Set dependency max age to {} days", payload.days),
            action_type: Some("ADMIN"),
            ..Default::default()
        },
    )
    .await?;
    Ok(StatusCode::OK)
}

pub async fn trigger_dependency_cleanup(
    State(state): State<AppState>,
    user: Option<axum::Extension<crate::domain::models::User>>,
) -> Result<impl IntoResponse, AppError> {
    let res = services::cleanup_stale_dependencies(&state.repo).await?;
    record_audit_log(
        &state.repo,
        user,
        NewAuditLog {
            action: "DEPENDENCY_CLEANUP",
            details: &format!(
                "Triggered dependency cleanup, deleted {} stale dependencies",
                res
            ),
            action_type: Some("ADMIN"),
            ..Default::default()
        },
    )
    .await?;
    Ok(Json(json!({ "deleted": res })))
}

/// Users, each with the roles granted to them.
///
/// The roles come along because the management UI shows them as badges; without
/// them the list could only say whether someone was an administrator, which is
/// the distinction the permission model replaced.
pub async fn admin_list_users(
    State(state): State<AppState>,
) -> Result<impl IntoResponse, AppError> {
    let users = services::list_users(&state.repo).await?;
    let kinds = services::account_kinds(&state.repo, &users).await?;
    let names = services::display_names(&state.repo).await?;
    let gravatars = services::gravatar_keys(&state.repo).await?;
    let mut out = Vec::with_capacity(users.len());
    for user in users {
        let roles = crate::application::authz::list_user_roles(&state.repo, user.id).await?;
        out.push(json!({
            "id": user.id,
            "username": user.username,
            "approved": user.approved,
            "service_account": user.is_service_account(),
            "kind": kinds.get(&user.id).copied().unwrap_or("unknown"),
            "display_name": names.get(&user.id),
            "gravatar_hash": gravatars.get(&user.id),
            "roles": roles,
        }));
    }
    Ok(Json(out))
}

#[derive(Deserialize)]
pub struct ServiceAccountRequest {
    pub username: String,
}

/// Create a service account: no password, ready at once, its tokens issued by
/// holders of `manage_users`.
pub async fn admin_create_service_account(
    State(state): State<AppState>,
    user: Option<axum::Extension<crate::domain::models::User>>,
    Json(payload): Json<ServiceAccountRequest>,
) -> Result<impl IntoResponse, AppError> {
    let account =
        services::create_service_account(&state.repo, &state.root_users, &payload.username).await?;
    record_audit_log(
        &state.repo,
        user,
        NewAuditLog {
            action: "CREATE_SERVICE_ACCOUNT",
            details: &format!("Created service account '{}'", account.username),
            action_type: Some("ADMIN"),
            ..Default::default()
        },
    )
    .await?;
    Ok((
        StatusCode::CREATED,
        Json(json!({ "id": account.id, "username": account.username, "service_account": true })),
    ))
}

/// Set a service account's display name; blank clears it.
pub async fn admin_set_display_name(
    State(state): State<AppState>,
    user: Option<axum::Extension<crate::domain::models::User>>,
    Path(id): Path<i64>,
    Json(payload): Json<crate::presentation::handlers::auth::DisplayNameRequest>,
) -> Result<impl IntoResponse, AppError> {
    let (username, display_name) =
        services::set_service_account_display_name(&state.repo, id, &payload.display_name).await?;
    record_audit_log(
        &state.repo,
        user,
        NewAuditLog {
            action: "SET_DISPLAY_NAME",
            details: &match &display_name {
                Some(name) => format!("Named service account '{username}' '{name}'"),
                None => format!("Cleared the display name of service account '{username}'"),
            },
            action_type: Some("ADMIN"),
            ..Default::default()
        },
    )
    .await?;
    Ok(Json(json!({ "display_name": display_name })))
}

pub async fn admin_approve_user(
    State(state): State<AppState>,
    user: Option<axum::Extension<crate::domain::models::User>>,
    Path(id): Path<i64>,
) -> Result<impl IntoResponse, AppError> {
    let target_username = if let Ok(users) = services::list_users(&state.repo).await {
        users
            .iter()
            .find(|u| u.id == id)
            .map(|u| u.username.clone())
            .unwrap_or_else(|| format!("User ID {}", id))
    } else {
        format!("User ID {}", id)
    };

    if services::approve_user(&state.repo, id).await? {
        record_audit_log(
            &state.repo,
            user,
            NewAuditLog {
                action: "APPROVE_USER",
                details: &format!("Approved user '{}'", target_username),
                action_type: Some("ADMIN"),
                ..Default::default()
            },
        )
        .await?;
        Ok(StatusCode::OK)
    } else {
        Err(AppError::NotFound(format!("User with ID {} not found", id)))
    }
}

pub async fn admin_delete_user_handler(
    State(state): State<AppState>,
    user: Option<axum::Extension<crate::domain::models::User>>,
    actor: Option<axum::Extension<crate::domain::permissions::Actor>>,
    Path(id): Path<i64>,
) -> Result<impl IntoResponse, AppError> {
    let target_username = if let Ok(users) = services::list_users(&state.repo).await {
        users
            .iter()
            .find(|u| u.id == id)
            .map(|u| u.username.clone())
            .unwrap_or_else(|| format!("User ID {}", id))
    } else {
        format!("User ID {}", id)
    };

    let actor = actor.map(|axum::Extension(a)| a);
    if services::admin_delete_user(&state.repo, &state.root_users, actor.as_ref(), id).await? {
        record_audit_log(
            &state.repo,
            user,
            NewAuditLog {
                action: "DELETE_USER",
                details: &format!("Deleted user '{}'", target_username),
                action_type: Some("ADMIN"),
                ..Default::default()
            },
        )
        .await?;
        Ok(StatusCode::OK)
    } else {
        Err(AppError::NotFound(format!("User with ID {} not found", id)))
    }
}

#[derive(Deserialize)]
pub struct NukeConfirmPayload {
    pub confirmation: String,
}

pub async fn admin_nuke_producers(
    State(state): State<AppState>,
    user: Option<axum::Extension<crate::domain::models::User>>,
    Json(payload): Json<NukeConfirmPayload>,
) -> Result<impl IntoResponse, AppError> {
    if payload.confirmation != "DELETE ALL SERVICES" {
        return Err(AppError::BadRequest("Invalid confirmation".to_string()));
    }
    let res = services::delete_all_services(&state.repo).await?;
    let _ = state.spec_updated_tx.send(());
    record_audit_log(
        &state.repo,
        user,
        NewAuditLog {
            action: "NUKE_DATABASE",
            details: &format!("Nuked all services, deleted {} services", res),
            action_type: Some("ADMIN"),
            ..Default::default()
        },
    )
    .await?;
    Ok(Json(json!({ "deleted": res })))
}

pub async fn admin_nuke_consumers(
    State(state): State<AppState>,
    user: Option<axum::Extension<crate::domain::models::User>>,
    Json(payload): Json<NukeConfirmPayload>,
) -> Result<impl IntoResponse, AppError> {
    if payload.confirmation != "DELETE ALL CLIENTS" {
        return Err(AppError::BadRequest("Invalid confirmation".to_string()));
    }
    let res = services::delete_all_clients(&state.repo).await?;
    let _ = state.spec_updated_tx.send(());
    record_audit_log(
        &state.repo,
        user,
        NewAuditLog {
            action: "NUKE_DATABASE",
            details: &format!("Nuked all clients, deleted {} clients", res),
            action_type: Some("ADMIN"),
            ..Default::default()
        },
    )
    .await?;
    Ok(Json(json!({ "deleted": res })))
}

pub async fn admin_nuke_users(
    State(state): State<AppState>,
    user: Option<axum::Extension<crate::domain::models::User>>,
    Json(payload): Json<NukeConfirmPayload>,
) -> Result<impl IntoResponse, AppError> {
    if payload.confirmation != "DELETE ALL USERS" {
        return Err(AppError::BadRequest("Invalid confirmation".to_string()));
    }
    let res = services::delete_all_non_admin_users(&state.repo, &state.root_users).await?;
    let _ = state.spec_updated_tx.send(());
    record_audit_log(
        &state.repo,
        user,
        NewAuditLog {
            action: "NUKE_DATABASE",
            details: &format!("Nuked all non-admin users, deleted {} users", res),
            action_type: Some("ADMIN"),
            ..Default::default()
        },
    )
    .await?;
    Ok(Json(json!({ "deleted": res })))
}

pub async fn admin_nuke_database(
    State(state): State<AppState>,
    user: Option<axum::Extension<crate::domain::models::User>>,
    Json(payload): Json<NukeConfirmPayload>,
) -> Result<impl IntoResponse, AppError> {
    if payload.confirmation != "NUKE DATABASE" {
        return Err(AppError::BadRequest("Invalid confirmation".to_string()));
    }
    let user_id = user.as_ref().map(|axum::Extension(u)| u.id);
    services::nuke_database(&state.repo, user_id).await?;
    let _ = state.spec_updated_tx.send(());
    record_audit_log(
        &state.repo,
        user,
        NewAuditLog {
            action: "NUKE_DATABASE",
            details: "Nuked complete database (Full reset)",
            action_type: Some("ADMIN"),
            ..Default::default()
        },
    )
    .await?;
    Ok(StatusCode::OK)
}

/// `GET /admin/backup/sqlite` (#50): a consistent snapshot of the SQLite
/// database, produced by `VACUUM INTO` and streamed from a temporary file in
/// the data directory that is removed once the download ends — also when the
/// client disconnects. One snapshot at a time; a PostgreSQL instance answers
/// `409` and names `pg_dump`.
#[tracing::instrument(skip_all)]
pub async fn download_sqlite_snapshot(
    State(state): State<AppState>,
    user: Option<axum::Extension<crate::domain::models::User>>,
) -> Result<axum::response::Response, AppError> {
    use crate::infrastructure::database::DatabaseRepo;
    use crate::infrastructure::sqlite_snapshot::{self, SnapshotLease, SqliteSnapshot};

    let DatabaseRepo::Sqlite(repo) = state.repo.inner() else {
        return Err(AppError::Conflict(
            "This instance runs PostgreSQL, which has no snapshot download; take backups with pg_dump (see the deployment guide, Backup and restore)".to_string(),
        ));
    };
    let lease = SnapshotLease::acquire(&state.snapshot_in_progress).ok_or_else(|| {
        AppError::Conflict(
            "A database snapshot is already being produced; retry when it has finished".to_string(),
        )
    })?;
    let snapshot =
        SqliteSnapshot::create(&repo.pool, &sqlite_snapshot::data_dir(&state.db_url), lease)
            .await?;
    let size = snapshot.size();
    let filename = format!(
        "sanshain-{}-{}.db",
        env!("CARGO_PKG_VERSION"),
        chrono::Utc::now().format("%Y%m%dT%H%M%SZ")
    );
    record_audit_log(
        &state.repo,
        user,
        NewAuditLog {
            action: "DATABASE_SNAPSHOT",
            details: &format!("Produced database snapshot {filename} ({size} bytes)"),
            action_type: Some("ADMIN"),
            ..Default::default()
        },
    )
    .await?;
    let stream = snapshot.into_stream().await?;
    let headers = [
        (
            axum::http::header::CONTENT_TYPE,
            "application/vnd.sqlite3".to_string(),
        ),
        (axum::http::header::CONTENT_LENGTH, size.to_string()),
        (
            axum::http::header::CONTENT_DISPOSITION,
            format!("attachment; filename=\"{filename}\""),
        ),
    ];
    Ok((headers, axum::body::Body::from_stream(stream)).into_response())
}

pub async fn export_audit_logs_csv(
    State(state): State<AppState>,
) -> Result<impl IntoResponse, AppError> {
    let logs: Vec<AuditLogEntry> = state.repo.get_recent_audit_logs(1000).await?;
    let mut csv =
        String::from("id,timestamp,username,action,details,service,version,action_type,stream\n");
    for log in logs {
        let esc_user = log.username.replace('"', "\"\"");
        let esc_action = log.action.replace('"', "\"\"");
        let esc_details = log.details.replace('"', "\"\"");
        let esc_stream = log.stream.as_deref().unwrap_or("").replace('"', "\"\"");
        csv.push_str(&format!(
            "{},\"{}\",\"{}\",\"{}\",\"{}\",\"{}\",\"{}\",\"{}\",\"{}\"\n",
            log.id,
            log.timestamp,
            esc_user,
            esc_action,
            esc_details,
            log.service.as_deref().unwrap_or(""),
            log.version.as_deref().unwrap_or(""),
            log.action_type.as_deref().unwrap_or(""),
            esc_stream
        ));
    }
    let headers = [
        (axum::http::header::CONTENT_TYPE, "text/csv"),
        (
            axum::http::header::CONTENT_DISPOSITION,
            "attachment; filename=\"audit_logs.csv\"",
        ),
    ];
    Ok((headers, csv))
}

#[derive(Deserialize)]
pub struct UpdateProducerMetadataRequest {
    pub name: String,
    pub icon: Option<String>,
    pub domain: Option<String>,
}

/// The admin page's provide (#65): the paste form's body, one shape for all
/// three families.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdminProvideRequest {
    pub producername: String,
    pub api_type: ApiType,
    pub content: String,
    pub stability: crate::domain::models::Stability,
    #[serde(default)]
    pub dry_run: bool,
}

/// Provide from the admin page. The same publish as `POST /provide` and its
/// siblings; a Producer it creates is an outside API and is marked external
/// before the answer goes back, so the mark cannot be lost on the way.
pub async fn admin_provide(
    State(state): State<AppState>,
    user: Option<axum::Extension<crate::domain::models::User>>,
    caller: Option<axum::Extension<crate::domain::permissions::Actor>>,
    Json(payload): Json<AdminProvideRequest>,
) -> Result<axum::response::Response, AppError> {
    let creates =
        services::provide_creates_producer(&state.repo, &payload.producername, payload.dry_run)
            .await?;
    let response = super::api::provide_common(
        &state,
        user.clone(),
        caller,
        super::api::ProvideCommon {
            api_type: payload.api_type,
            producername: &payload.producername,
            content: Some(&payload.content),
            content_field: "content",
            stability: Some(payload.stability),
            dry_run: payload.dry_run,
            trunk: false,
            tag: None,
            retired: false,
            origin: crate::domain::models::VersionOrigin::default(),
        },
    )
    .await?;
    if creates && response.status() == StatusCode::ACCEPTED {
        services::set_producer_external(&state.repo, &payload.producername, true).await?;
        let _ = state.spec_updated_tx.send(());
        record_audit_log(
            &state.repo,
            user,
            NewAuditLog {
                action: "SET_PRODUCER_EXTERNAL",
                details: &format!("Marked Producer '{}' as external", payload.producername),
                service: Some(&payload.producername),
                action_type: Some("WRITE"),
                ..Default::default()
            },
        )
        .await?;
    }
    Ok(response)
}

#[derive(Deserialize)]
pub struct ExternalRequest {
    pub external: bool,
}

/// Mark a Producer as external, or as the company's own again.
pub async fn admin_set_producer_external(
    State(state): State<AppState>,
    user: Option<axum::Extension<crate::domain::models::User>>,
    Path(name): Path<String>,
    Json(payload): Json<ExternalRequest>,
) -> Result<impl IntoResponse, AppError> {
    services::set_producer_external(&state.repo, &name, payload.external).await?;
    let _ = state.spec_updated_tx.send(());
    record_audit_log(
        &state.repo,
        user,
        NewAuditLog {
            action: "SET_PRODUCER_EXTERNAL",
            details: &format!(
                "Marked Producer '{}' as {}",
                name,
                if payload.external {
                    "external"
                } else {
                    "the company's own"
                }
            ),
            service: Some(&name),
            action_type: Some("WRITE"),
            ..Default::default()
        },
    )
    .await?;
    Ok(StatusCode::OK)
}

pub async fn admin_update_producer_metadata(
    State(state): State<AppState>,
    user: Option<axum::Extension<crate::domain::models::User>>,
    Json(payload): Json<UpdateProducerMetadataRequest>,
) -> Result<impl IntoResponse, AppError> {
    services::update_producer_metadata(
        &state.repo,
        &payload.name,
        payload.icon.as_deref(),
        payload.domain.as_deref(),
    )
    .await?;

    let _ = state.spec_updated_tx.send(());

    record_audit_log(
        &state.repo,
        user,
        NewAuditLog {
            action: "UPDATE_SERVICE_METADATA",
            details: &format!("Updated metadata for service '{}'", payload.name),
            service: Some(&payload.name),
            action_type: Some("WRITE"),
            ..Default::default()
        },
    )
    .await?;

    Ok(StatusCode::OK)
}

pub async fn get_observability_stats(
    State(state): State<AppState>,
) -> Result<impl IntoResponse, AppError> {
    use crate::domain::models::SystemStats;
    let mut system = state
        .system
        .lock()
        .map_err(|_| AppError::Internal("System stats lock poisoned".to_string()))?;
    system.refresh_cpu_usage();
    system.refresh_memory();

    let stats = SystemStats {
        cpu_usage: system.global_cpu_usage(),
        memory_used: system.used_memory(),
        memory_total: system.total_memory(),
        system_uptime: sysinfo::System::uptime(),
        process_uptime: (chrono::Utc::now() - state.process_start_time)
            .num_seconds()
            .max(0) as u64,
        requests_total: state
            .requests_total
            .load(std::sync::atomic::Ordering::Relaxed),
        failures_total: state
            .failures_total
            .load(std::sync::atomic::Ordering::Relaxed),
    };
    Ok(Json(stats))
}

pub async fn get_observability_logs(
    State(state): State<AppState>,
) -> Result<impl IntoResponse, AppError> {
    use crate::domain::models::LogResponse;
    let errors = state
        .error_buffer
        .lock()
        .map_err(|_| AppError::Internal("Error buffer lock poisoned".to_string()))?
        .iter()
        .cloned()
        .collect();
    let warnings = state
        .warn_buffer
        .lock()
        .map_err(|_| AppError::Internal("Warn buffer lock poisoned".to_string()))?
        .iter()
        .cloned()
        .collect();
    let infos = state
        .info_buffer
        .lock()
        .map_err(|_| AppError::Internal("Info buffer lock poisoned".to_string()))?
        .iter()
        .cloned()
        .collect();
    let debugs = state
        .debug_buffer
        .lock()
        .map_err(|_| AppError::Internal("Debug buffer lock poisoned".to_string()))?
        .iter()
        .cloned()
        .collect();
    Ok(Json(LogResponse {
        errors,
        warnings,
        infos,
        debugs,
    }))
}

pub async fn get_debug_config(
    State(state): State<AppState>,
) -> Result<impl IntoResponse, AppError> {
    use crate::domain::models::DebugConfig;
    Ok(Json(DebugConfig {
        business_logic_debug: state
            .business_logic_debug
            .load(std::sync::atomic::Ordering::Relaxed),
        admin_user_debug: state
            .admin_user_debug
            .load(std::sync::atomic::Ordering::Relaxed),
    }))
}

pub async fn set_debug_config(
    State(state): State<AppState>,
    user: Option<axum::Extension<crate::domain::models::User>>,
    Json(config): Json<crate::domain::models::DebugConfig>,
) -> Result<impl IntoResponse, AppError> {
    // Authorisation is the route guard's job; the handler only records who
    // acted. Leaving a second check here would be a place for the two to drift.
    state.business_logic_debug.store(
        config.business_logic_debug,
        std::sync::atomic::Ordering::Relaxed,
    );
    state.admin_user_debug.store(
        config.admin_user_debug,
        std::sync::atomic::Ordering::Relaxed,
    );
    record_audit_log(
        &state.repo,
        user,
        NewAuditLog {
            action: "UPDATE_SETTINGS",
            details: &format!(
                "Updated debug config: business_logic_debug={}, admin_user_debug={}",
                config.business_logic_debug, config.admin_user_debug
            ),
            action_type: Some("ADMIN"),
            ..Default::default()
        },
    )
    .await?;
    Ok(StatusCode::OK)
}

#[derive(serde::Serialize)]
pub struct DatabaseInfoResponse {
    pub backend: &'static str,
    pub url: String,
}

/// Which database this instance is connected to, for the admin "Database
/// Configuration" panel. The URL is credential-stripped — see
/// [`redact_database_url`]; the raw `DATABASE_URL` never leaves the process.
pub async fn get_database_info(State(state): State<AppState>) -> impl IntoResponse {
    let backend =
        if state.db_url.starts_with("postgres://") || state.db_url.starts_with("postgresql://") {
            "postgres"
        } else {
            "sqlite"
        };
    Json(DatabaseInfoResponse {
        backend,
        url: redact_database_url(&state.db_url),
    })
}

/// Remove credentials from a database connection string so it can be displayed.
///
/// A `DATABASE_URL` normally carries the database password
/// (`postgres://user:password@host:5432/dbname`). This keeps the parts that make
/// the value useful as a diagnostic — scheme, user, host, port, database — and
/// drops anything secret:
///
/// - the password in the userinfo section (`user:password@` becomes `user@`)
/// - any query parameter whose name contains `password` (e.g. `?sslpassword=`)
///
/// Splits userinfo at the *last* `@` in the authority, so a password that itself
/// contains `@` cannot smuggle part of itself into the host.
fn redact_database_url(url: &str) -> String {
    let Some((scheme, rest)) = url.split_once("://") else {
        // No authority section (e.g. `sqlite:sanshain.db`, `sqlite::memory:`),
        // so there are no userinfo credentials — only query params to check.
        return redact_query_password(url);
    };

    // The authority ends at the first '/', '?' or '#'.
    let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let (authority, tail) = rest.split_at(end);

    let authority = match authority.rsplit_once('@') {
        Some((userinfo, host)) => {
            let user = userinfo.split_once(':').map_or(userinfo, |(u, _)| u);
            if user.is_empty() {
                host.to_string()
            } else {
                format!("{}@{}", user, host)
            }
        }
        None => authority.to_string(),
    };

    format!("{}://{}{}", scheme, authority, redact_query_password(tail))
}

/// Replace the value of any query parameter whose name mentions `password`.
fn redact_query_password(s: &str) -> String {
    let Some((before, query)) = s.split_once('?') else {
        return s.to_string();
    };
    let redacted: Vec<String> = query
        .split('&')
        .map(|pair| match pair.split_once('=') {
            Some((key, _)) if key.to_lowercase().contains("password") => {
                format!("{}=***", key)
            }
            _ => pair.to_string(),
        })
        .collect();
    format!("{}?{}", before, redacted.join("&"))
}

/// The instance settings every signed-in user may read: what the admin page
/// shows as a greyed-out value to someone who may not change it, so no control
/// ever shows a value it did not receive. Only harmless values: the auth
/// configuration, the webhook allow-list and the database stay behind their
/// own permissions.
pub async fn get_instance_settings(
    State(state): State<AppState>,
) -> Result<impl IntoResponse, AppError> {
    let stats = state.repo.cache_stats().await;
    Ok(Json(json!({
        "auto_approve_users": services::get_auto_approve_users(&state.repo).await?,
        "registration_enabled": services::get_registration_enabled(&state.repo).await?,
        "gravatar_enabled": services::get_gravatar_enabled(&state.repo).await?,
        "dev_mode_allowed": services::dev_mode_gate_open(),
        "snapshot_max_age_days": services::get_snapshot_max_age_days(&state.repo).await?,
        "dependency_max_age_days": services::get_dependency_max_age_days(&state.repo).await?,
        "trunk_max_age_days": services::get_trunk_max_age_days(&state.repo).await?,
        "ticket_url_template": services::get_ticket_url_template(&state.repo).await?,
        "cache": {
            "enabled": stats.enabled,
            "memory_limit_mb": stats.memory_limit_mb,
            "memory_used_mb": stats.estimated_memory_used_bytes as f64 / 1024.0 / 1024.0,
            "entry_count": stats.entry_count,
            "hit_count": stats.hit_count,
            "miss_count": stats.miss_count,
            "hit_rate": stats.hit_rate_percent,
        },
    })))
}

pub async fn get_cache_config(
    State(state): State<AppState>,
) -> Result<impl IntoResponse, AppError> {
    let stats = state.repo.cache_stats().await;
    Ok(Json(json!({
        "enabled": stats.enabled,
        "memory_limit_mb": stats.memory_limit_mb,
        "memory_used_mb": stats.estimated_memory_used_bytes as f64 / 1024.0 / 1024.0,
        "entry_count": stats.entry_count,
        "hit_count": stats.hit_count,
        "miss_count": stats.miss_count,
        "hit_rate": stats.hit_rate_percent,
    })))
}

#[derive(Deserialize)]
pub struct CacheConfigPayload {
    pub memory_mb: u64,
}

pub async fn set_cache_config(
    State(state): State<AppState>,
    user: Option<axum::Extension<crate::domain::models::User>>,
    Json(payload): Json<CacheConfigPayload>,
) -> Result<impl IntoResponse, AppError> {
    state.repo.rebuild_caches(payload.memory_mb);
    record_audit_log(
        &state.repo,
        user,
        NewAuditLog {
            action: "UPDATE_SETTINGS",
            details: &format!("Updated cache memory limit to {} MB", payload.memory_mb),
            action_type: Some("ADMIN"),
            ..Default::default()
        },
    )
    .await?;
    Ok(Json(json!({ "memory_mb": payload.memory_mb })))
}

pub async fn clear_cache(
    State(state): State<AppState>,
    user: Option<axum::Extension<crate::domain::models::User>>,
) -> Result<impl IntoResponse, AppError> {
    let limit = state.repo.cache_stats().await.memory_limit_mb;
    state.repo.rebuild_caches(limit);
    record_audit_log(
        &state.repo,
        user,
        NewAuditLog {
            action: "CLEAR_CACHE",
            details: "Cleared spec caches",
            action_type: Some("ADMIN"),
            ..Default::default()
        },
    )
    .await?;
    Ok(Json(json!({ "cleared": true })))
}

#[derive(Deserialize)]
pub struct ObservabilityAuditQuery {
    /// How many entries to return. The panel asks for more than it shows so a
    /// burst of rejections cannot push every real change out of the window.
    pub limit: Option<u32>,
}

/// Upper bound for the observability panel: this is a glance, not the audit
/// timeline, and an unbounded limit would let one page pull the whole table.
const OBSERVABILITY_AUDIT_MAX: u32 = 200;

pub async fn get_observability_audit_logs(
    State(state): State<AppState>,
    Query(query): Query<ObservabilityAuditQuery>,
) -> Result<impl IntoResponse, AppError> {
    let limit = query.limit.unwrap_or(100).clamp(1, OBSERVABILITY_AUDIT_MAX);
    let logs: Vec<AuditLogEntry> = state.repo.get_recent_audit_logs(limit).await?;
    Ok(Json(logs))
}

// ── Sanshain-branches (ADR-0005) ─────────────────────────────────────────

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateBranchRequest {
    pub name: String,
    /// An existing branch's name; trunk when omitted.
    pub source: Option<String>,
    /// RFC 3339 instant; now when omitted (retroactive creation).
    pub as_of: Option<String>,
}

pub async fn admin_create_branch(
    State(state): State<AppState>,
    caller: Option<axum::Extension<crate::domain::permissions::Actor>>,
    Json(payload): Json<CreateBranchRequest>,
) -> Result<impl IntoResponse, AppError> {
    let created_by = caller
        .as_ref()
        .map(|axum::Extension(a)| a.username.clone())
        .unwrap_or_else(|| DEV_MODE_ACTOR.to_string());
    let branch = services::create_branch(
        &state.repo,
        services::CreateBranchParams {
            name: &payload.name,
            source: payload.source.as_deref(),
            as_of: payload.as_of.as_deref(),
            created_by: &created_by,
        },
    )
    .await?;
    Ok((StatusCode::CREATED, Json(branch)))
}

pub async fn admin_list_branches(
    State(state): State<AppState>,
) -> Result<impl IntoResponse, AppError> {
    Ok(Json(services::list_branches(&state.repo).await?))
}

#[derive(Deserialize)]
pub struct BranchGraphQuery {
    /// Render the branch's graph as it was at this RFC 3339 instant.
    pub at: Option<String>,
}

pub async fn admin_branch_graph(
    State(state): State<AppState>,
    Path(name): Path<String>,
    Query(query): Query<BranchGraphQuery>,
) -> Result<impl IntoResponse, AppError> {
    let pins = services::get_branch_graph(&state.repo, &name, query.at.as_deref()).await?;
    Ok(Json(pins))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RenameBranchRequest {
    pub new_name: String,
}

pub async fn admin_rename_branch(
    State(state): State<AppState>,
    user: Option<axum::Extension<crate::domain::models::User>>,
    Path(name): Path<String>,
    Json(payload): Json<RenameBranchRequest>,
) -> Result<impl IntoResponse, AppError> {
    let actor = actor_or_dev_mode(user.as_deref());
    let branch = services::rename_branch(&state.repo, &name, &payload.new_name, &actor).await?;
    Ok(Json(branch))
}

pub async fn admin_delete_branch(
    State(state): State<AppState>,
    user: Option<axum::Extension<crate::domain::models::User>>,
    Path(name): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    let actor = actor_or_dev_mode(user.as_deref());
    services::delete_branch(&state.repo, &name, &actor).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn get_trunk_max_age(
    State(state): State<AppState>,
) -> Result<impl IntoResponse, AppError> {
    let res = services::get_trunk_max_age_days(&state.repo).await?;
    Ok(Json(json!({ "days": res })))
}

pub async fn set_trunk_max_age(
    State(state): State<AppState>,
    user: Option<axum::Extension<crate::domain::models::User>>,
    Json(payload): Json<MaxAgeDaysPayload>,
) -> Result<impl IntoResponse, AppError> {
    services::set_trunk_max_age_days(&state.repo, payload.days).await?;
    record_audit_log(
        &state.repo,
        user,
        NewAuditLog {
            action: "SETTINGS_CHANGED",
            details: &format!("Set trunk max age to {} days", payload.days),
            action_type: Some("ADMIN"),
            ..Default::default()
        },
    )
    .await?;
    Ok(Json(json!({ "days": payload.days })))
}

pub async fn trigger_trunk_cleanup(
    State(state): State<AppState>,
    user: Option<axum::Extension<crate::domain::models::User>>,
) -> Result<impl IntoResponse, AppError> {
    let closed = services::cleanup_stale_trunk_data(&state.repo).await?;
    record_audit_log(
        &state.repo,
        user,
        NewAuditLog {
            action: "TRUNK_CLEANUP",
            details: &format!("Triggered trunk cleanup, closed {closed} stale trunk pins"),
            action_type: Some("ADMIN"),
            ..Default::default()
        },
    )
    .await?;
    Ok(Json(json!({ "closed": closed })))
}

/// The main graph — current, or as it was at `at` (the timeline, ADR-0005).
pub async fn admin_trunk_graph(
    State(state): State<AppState>,
    Query(query): Query<BranchGraphQuery>,
) -> Result<impl IntoResponse, AppError> {
    let pins = services::get_trunk_graph(&state.repo, query.at.as_deref()).await?;
    Ok(Json(pins))
}

#[derive(Deserialize)]
pub struct PeripheryQuery {
    pub scope: Option<String>,
}

/// The periphery edges (#46) of one graph scope — `dev` (default),
/// `main[@rfc3339]` or `<branch>[@rfc3339]` — for the graph's main, branch
/// and timeline views, whose pin sets come from the bare-array endpoints.
pub async fn admin_periphery(
    State(state): State<AppState>,
    Query(query): Query<PeripheryQuery>,
) -> Result<impl IntoResponse, AppError> {
    let scope = services::ReportScope::parse(query.scope.as_deref())?;
    Ok(Json(services::get_periphery(&state.repo, &scope).await?))
}

/// What the graph needs of one scope beyond its pins (`dev` default,
/// `main[@rfc3339]` or `<branch>[@rfc3339]`): the producers in it at its
/// instant with their version lines, and its harvested subscriptions.
pub async fn admin_graph_scope(
    State(state): State<AppState>,
    Query(query): Query<PeripheryQuery>,
) -> Result<impl IntoResponse, AppError> {
    let scope = services::ReportScope::parse(query.scope.as_deref())?;
    Ok(Json(services::get_graph_scope(&state.repo, &scope).await?))
}

/// The instants the main graph changed — the timeline slider's markers.
pub async fn admin_trunk_timeline(
    State(state): State<AppState>,
) -> Result<impl IntoResponse, AppError> {
    Ok(Json(state.repo.list_graph_change_dates(None).await?))
}

/// The instants a branch's graph changed.
pub async fn admin_branch_timeline(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    let branch = state
        .repo
        .find_branch(&name)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("no sanshain-branch '{name}'")))?;
    Ok(Json(
        state.repo.list_graph_change_dates(Some(branch.id)).await?,
    ))
}

/// Reverse lookup (ADR-0005): which sanshain-branches reference each of this
/// producer's versions — the "which releases pin b@1.0.0?" answer.
pub async fn admin_producer_branch_memberships(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    let service_id = state
        .repo
        .find_service(&name)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("Producer '{name}' not found")))?;
    Ok(Json(
        state
            .repo
            .list_branch_memberships_for_service(service_id)
            .await?,
    ))
}

#[derive(Deserialize)]
pub struct GraphDiffQuery {
    /// `main[@instant]` or `<branch>[@instant]`.
    pub left: String,
    pub right: String,
}

pub async fn admin_graph_diff(
    State(state): State<AppState>,
    Query(query): Query<GraphDiffQuery>,
) -> Result<impl IntoResponse, AppError> {
    Ok(Json(
        services::diff_graphs(&state.repo, &query.left, &query.right).await?,
    ))
}
