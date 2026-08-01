use crate::AppState;
use crate::application::services::{self, AppError};
use crate::domain::models::{ApiType, AuditLogEntry, AuthMode, LdapConfig};
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

use super::record_audit_log;

pub async fn admin_list_producers(
    State(state): State<AppState>,
    user: Option<axum::Extension<crate::domain::models::User>>,
) -> Result<impl IntoResponse, AppError> {
    let user_id = user.map(|axum::Extension(u)| u.id);
    let res = services::list_producers_detailed(&state.repo, user_id).await?;
    Ok(Json(res))
}

pub async fn admin_list_branches(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    let res = services::list_branches(&state.repo, &name).await?;
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

pub async fn admin_list_consumer_branches(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    let res = services::list_consumer_branches(&state.repo, &name).await?;
    Ok(Json(res))
}

pub async fn admin_list_consumer_endpoints(
    State(state): State<AppState>,
    Path((name, branch)): Path<(String, String)>,
) -> Result<impl IntoResponse, AppError> {
    let res = services::list_consumer_endpoints(&state.repo, &name, &branch).await?;
    Ok(Json(res))
}

pub async fn admin_list_producer_endpoints(
    State(state): State<AppState>,
    Path((name, branch)): Path<(String, String)>,
) -> Result<impl IntoResponse, AppError> {
    let res = services::list_producer_endpoints(&state.repo, &name, &branch).await?;
    Ok(Json(res))
}

#[derive(Deserialize)]
pub struct FullSpecQuery {
    pub api_type: Option<ApiType>,
}

pub async fn admin_get_full_spec(
    State(state): State<AppState>,
    Path((name, branch)): Path<(String, String)>,
    Query(query): Query<FullSpecQuery>,
) -> Result<impl IntoResponse, AppError> {
    let api_type = query.api_type.unwrap_or(ApiType::OpenApi);
    let spec = services::get_full_spec(&state.repo, &name, &branch, api_type).await?;
    // Served as a download: name the file after the service/branch it came from,
    // and use the media type matching the reassembled document.
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
        sanitize_filename_part(&branch),
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

/// Reduce a service or branch name to characters that are safe in a
/// `Content-Disposition` filename. Service and branch names are client-supplied
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

pub async fn admin_list_all_branches(
    State(state): State<AppState>,
) -> Result<impl IntoResponse, AppError> {
    let res = services::list_all_branches(&state.repo).await?;
    Ok(Json(res))
}

#[derive(Deserialize)]
pub struct AdminEndpointYamlQuery {
    pub producername: String,
    pub branch: String,
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
        &query.branch,
        query.api_type,
        &query.path,
        &query.method,
    )
    .await?;
    // Always 200 with a discriminated body: the view is told what happened
    // (published / absent / inherited / unknown) and which branch served it,
    // instead of having to infer a state from an error.
    Ok(Json(res))
}

pub async fn admin_get_endpoint_versions(
    State(state): State<AppState>,
    Query(query): Query<AdminEndpointYamlQuery>,
) -> Result<impl IntoResponse, AppError> {
    let res = services::get_endpoint_version_history(
        &state.repo,
        &query.producername,
        &query.branch,
        query.api_type,
        &query.path,
        &query.method,
    )
    .await?;
    Ok(Json(res))
}

pub async fn list_protected_branches(
    State(state): State<AppState>,
) -> Result<impl IntoResponse, AppError> {
    let res = services::list_protected_branches(&state.repo).await?;
    Ok(Json(res))
}

#[derive(Deserialize)]
pub struct AddProtectedBranchRequest {
    pub pattern: String,
}

pub async fn add_protected_branch(
    State(state): State<AppState>,
    user: Option<axum::Extension<crate::domain::models::User>>,
    Json(payload): Json<AddProtectedBranchRequest>,
) -> Result<impl IntoResponse, AppError> {
    services::add_protected_branch(&state.repo, &payload.pattern).await?;
    record_audit_log(
        &state.repo,
        user,
        NewAuditLog {
            action: "ADD_PROTECTED_BRANCH",
            details: &format!("Protected branch pattern '{}' added", payload.pattern),
            service: None,
            branch: None,
            action_type: Some("ADMIN"),
            diff: None,
        },
    )
    .await?;
    Ok(StatusCode::CREATED)
}

pub async fn delete_protected_branch(
    State(state): State<AppState>,
    user: Option<axum::Extension<crate::domain::models::User>>,
    Path(pattern): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    if services::remove_protected_branch(&state.repo, &pattern).await? {
        record_audit_log(
            &state.repo,
            user,
            NewAuditLog {
                action: "DELETE_PROTECTED_BRANCH",
                details: &format!("Protected branch pattern '{}' deleted", pattern),
                service: None,
                branch: None,
                action_type: Some("ADMIN"),
                diff: None,
            },
        )
        .await?;
        Ok(StatusCode::OK)
    } else {
        Err(AppError::NotFound(format!(
            "Protected branch pattern {} not found",
            pattern
        )))
    }
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
                branch: None,
                action_type: Some("WRITE"),
                diff: None,
            },
        )
        .await?;
        Ok(StatusCode::OK)
    } else {
        Err(AppError::NotFound(format!("Service not found: {}", name)))
    }
}

pub async fn admin_delete_branch(
    State(state): State<AppState>,
    user: Option<axum::Extension<crate::domain::models::User>>,
    Path((name, branch)): Path<(String, String)>,
) -> Result<impl IntoResponse, AppError> {
    if services::delete_branch(&state.repo, &name, &branch).await? {
        let _ = state.spec_updated_tx.send(());
        record_audit_log(
            &state.repo,
            user,
            NewAuditLog {
                action: "DELETE_BRANCH",
                details: &format!("Deleted branch '{}' of service '{}'", branch, name),
                service: Some(&name),
                branch: Some(&branch),
                action_type: Some("WRITE"),
                diff: None,
            },
        )
        .await?;
        Ok(StatusCode::OK)
    } else {
        Err(AppError::NotFound(format!(
            "Branch {} for service {} not found",
            branch, name
        )))
    }
}

pub async fn admin_reset_branch_history(
    State(state): State<AppState>,
    user: Option<axum::Extension<crate::domain::models::User>>,
    Path((name, branch)): Path<(String, String)>,
) -> Result<impl IntoResponse, AppError> {
    if services::reset_branch_history(&state.repo, &name, &branch).await? {
        record_audit_log(
            &state.repo,
            user,
            NewAuditLog {
                action: "RESET_BRANCH_HISTORY",
                details: &format!(
                    "Reset branch history of branch '{}' of service '{}'",
                    branch, name
                ),
                service: Some(&name),
                branch: Some(&branch),
                action_type: Some("WRITE"),
                diff: None,
            },
        )
        .await?;
        Ok(StatusCode::OK)
    } else {
        Err(AppError::NotFound(format!(
            "Branch {} for service {} not found",
            branch, name
        )))
    }
}

#[derive(serde::Serialize)]
pub struct SourceProtectedBranchResponse {
    pub source_protected_branch: Option<String>,
}

/// Item #17: view a branch's `source_protected_branch` — the protected branch
/// it defers to when it has no data of its own (either caller-supplied on a
/// first `/provide`/`/require`, or admin-corrected).
pub async fn admin_get_source_protected_branch(
    State(state): State<AppState>,
    Path((name, branch)): Path<(String, String)>,
) -> Result<impl IntoResponse, AppError> {
    let value = services::get_source_protected_branch(&state.repo, &name, &branch).await?;
    Ok(Json(SourceProtectedBranchResponse {
        source_protected_branch: value,
    }))
}

#[derive(Deserialize)]
pub struct SetSourceProtectedBranchRequest {
    pub source_protected_branch: Option<String>,
}

/// Item #17, admin-only: unconditionally set (or clear, with `null`) a
/// branch's `source_protected_branch`, overwriting whatever is currently
/// stored — the only way to correct a wrong or missing caller-supplied value.
pub async fn admin_set_source_protected_branch(
    State(state): State<AppState>,
    user: Option<axum::Extension<crate::domain::models::User>>,
    Path((name, branch)): Path<(String, String)>,
    Json(payload): Json<SetSourceProtectedBranchRequest>,
) -> Result<impl IntoResponse, AppError> {
    services::admin_set_source_protected_branch(
        &state.repo,
        &name,
        &branch,
        payload.source_protected_branch.as_deref(),
    )
    .await?;
    record_audit_log(
        &state.repo,
        user,
        NewAuditLog {
            action: "SET_SOURCE_PROTECTED_BRANCH",
            details: &match &payload.source_protected_branch {
                Some(v) => format!(
                    "Set source_protected_branch of branch '{}' of service '{}' to '{}'",
                    branch, name, v
                ),
                None => format!(
                    "Cleared source_protected_branch of branch '{}' of service '{}'",
                    branch, name
                ),
            },
            service: Some(&name),
            branch: Some(&branch),
            action_type: Some("ADMIN"),
            diff: None,
        },
    )
    .await?;
    Ok(StatusCode::OK)
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
                service: None,
                branch: None,
                action_type: Some("WRITE"),
                diff: None,
            },
        )
        .await?;
        Ok(StatusCode::OK)
    } else {
        Err(AppError::NotFound(format!("Client not found: {}", name)))
    }
}

pub async fn get_dev_mode(State(state): State<AppState>) -> Result<impl IntoResponse, AppError> {
    // Report the persisted *setting* (what the admin toggled), not the effective
    // gated value. Whether dev mode actually bypasses auth additionally depends on
    // the `ALLOW_INSECURE_DEV_MODE` safety gate, which is enforced in middleware.
    let res = services::is_dev_mode_requested(&state.repo).await?;
    Ok(Json(json!({ "dev_mode": res })))
}

#[derive(Deserialize)]
pub struct EnabledRequest {
    pub enabled: bool,
}

pub async fn set_dev_mode(
    State(state): State<AppState>,
    user: Option<axum::Extension<crate::domain::models::User>>,
    Json(payload): Json<EnabledRequest>,
) -> Result<impl IntoResponse, AppError> {
    services::set_dev_mode(&state.repo, payload.enabled).await?;
    record_audit_log(
        &state.repo,
        user,
        NewAuditLog {
            action: "SET_DEV_MODE",
            details: &format!("Set dev-mode to {}", payload.enabled),
            service: None,
            branch: None,
            action_type: Some("ADMIN"),
            diff: None,
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
            service: None,
            branch: None,
            action_type: Some("ADMIN"),
            diff: None,
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

    Ok(Json(serde_json::json!({
        "auth_mode": mode,
        "ldap_config": ldap_val,
    })))
}

#[derive(Deserialize)]
pub struct AuthConfigRequest {
    pub auth_mode: String,
    pub ldap_config: Option<LdapConfig>,
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

    services::set_auth_mode(&state.repo, &mode).await?;
    if let Some(mut ldap) = payload.ldap_config {
        if let Some(ref pass) = ldap.bind_password
            && pass == "****"
            && let Ok(Some(old_config)) = services::get_ldap_config(&state.repo).await
        {
            ldap.bind_password = old_config.bind_password;
        }
        services::set_ldap_config(&state.repo, &ldap).await?;
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
            service: None,
            branch: None,
            action_type: Some("ADMIN"),
            diff: None,
        },
    )
    .await?;
    Ok(StatusCode::OK)
}

pub async fn test_auth_config(
    State(_state): State<AppState>,
    Json(config): Json<LdapConfig>,
) -> Result<impl IntoResponse, AppError> {
    let provider = crate::infrastructure::ldap_provider::LdapAuthProvider::new(config);
    services::test_ldap_connection(&provider).await?;
    Ok(StatusCode::OK)
}

pub async fn get_branch_max_age(
    State(state): State<AppState>,
) -> Result<impl IntoResponse, AppError> {
    let res = services::get_branch_max_age_days(&state.repo).await?;
    Ok(Json(json!({ "days": res })))
}

#[derive(Deserialize)]
pub struct MaxAgeDaysPayload {
    pub days: u64,
}

pub async fn set_branch_max_age(
    State(state): State<AppState>,
    user: Option<axum::Extension<crate::domain::models::User>>,
    Json(payload): Json<MaxAgeDaysPayload>,
) -> Result<impl IntoResponse, AppError> {
    if payload.days == 0 {
        return Err(AppError::BadRequest(
            "days must be greater than 0".to_string(),
        ));
    }
    services::set_branch_max_age_days(&state.repo, payload.days).await?;
    record_audit_log(
        &state.repo,
        user,
        NewAuditLog {
            action: "UPDATE_SETTINGS",
            details: &format!("Set branch max age to {} days", payload.days),
            service: None,
            branch: None,
            action_type: Some("ADMIN"),
            diff: None,
        },
    )
    .await?;
    Ok(StatusCode::OK)
}

pub async fn trigger_branch_cleanup(
    State(state): State<AppState>,
    user: Option<axum::Extension<crate::domain::models::User>>,
) -> Result<impl IntoResponse, AppError> {
    let res = services::cleanup_stale_branches(&state.repo).await?;
    record_audit_log(
        &state.repo,
        user,
        NewAuditLog {
            action: "BRANCH_CLEANUP",
            details: &format!("Triggered branch cleanup, deleted {} stale branches", res),
            service: None,
            branch: None,
            action_type: Some("ADMIN"),
            diff: None,
        },
    )
    .await?;
    Ok(Json(json!({ "deleted": res })))
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
            service: None,
            branch: None,
            action_type: Some("ADMIN"),
            diff: None,
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
            service: None,
            branch: None,
            action_type: Some("ADMIN"),
            diff: None,
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
    let mut out = Vec::with_capacity(users.len());
    for user in users {
        let roles = crate::application::authz::list_user_roles(&state.repo, user.id).await?;
        out.push(json!({
            "id": user.id,
            "username": user.username,
            "approved": user.approved,
            "roles": roles,
        }));
    }
    Ok(Json(out))
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
                service: None,
                branch: None,
                action_type: Some("ADMIN"),
                diff: None,
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

    if services::admin_delete_user(&state.repo, id).await? {
        record_audit_log(
            &state.repo,
            user,
            NewAuditLog {
                action: "DELETE_USER",
                details: &format!("Deleted user '{}'", target_username),
                service: None,
                branch: None,
                action_type: Some("ADMIN"),
                diff: None,
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
            service: None,
            branch: None,
            action_type: Some("ADMIN"),
            diff: None,
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
            service: None,
            branch: None,
            action_type: Some("ADMIN"),
            diff: None,
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
    let res = services::delete_all_non_admin_users(&state.repo).await?;
    let _ = state.spec_updated_tx.send(());
    record_audit_log(
        &state.repo,
        user,
        NewAuditLog {
            action: "NUKE_DATABASE",
            details: &format!("Nuked all non-admin users, deleted {} users", res),
            service: None,
            branch: None,
            action_type: Some("ADMIN"),
            diff: None,
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
            service: None,
            branch: None,
            action_type: Some("ADMIN"),
            diff: None,
        },
    )
    .await?;
    Ok(StatusCode::OK)
}

pub async fn export_audit_logs_csv(
    State(state): State<AppState>,
) -> Result<impl IntoResponse, AppError> {
    let logs: Vec<AuditLogEntry> = state.repo.get_recent_audit_logs(1000).await?;
    let mut csv = String::from("id,timestamp,username,action,details,service,branch,action_type\n");
    for log in logs {
        let esc_user = log.username.replace('"', "\"\"");
        let esc_action = log.action.replace('"', "\"\"");
        let esc_details = log.details.replace('"', "\"\"");
        csv.push_str(&format!(
            "{},\"{}\",\"{}\",\"{}\",\"{}\",\"{}\",\"{}\",\"{}\"\n",
            log.id,
            log.timestamp,
            esc_user,
            esc_action,
            esc_details,
            log.service.as_deref().unwrap_or(""),
            log.branch.as_deref().unwrap_or(""),
            log.action_type.as_deref().unwrap_or("")
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

pub async fn admin_nuke_branch(
    State(state): State<AppState>,
    Path(branch): Path<String>,
    user: Option<axum::Extension<crate::domain::models::User>>,
    Json(payload): Json<NukeConfirmPayload>,
) -> Result<impl IntoResponse, AppError> {
    if payload.confirmation != format!("DELETE BRANCH {}", branch) {
        return Err(AppError::BadRequest("Invalid confirmation".to_string()));
    }
    let res = services::delete_branch_all_services(&state.repo, &branch).await?;
    let _ = state.spec_updated_tx.send(());
    record_audit_log(
        &state.repo,
        user,
        NewAuditLog {
            action: "NUKE_DATABASE",
            details: &format!(
                "Nuked branch '{}', deleted {} services on branch",
                branch, res
            ),
            service: None,
            branch: Some(&branch),
            action_type: Some("ADMIN"),
            diff: None,
        },
    )
    .await?;
    Ok(Json(json!({ "deleted": res })))
}

#[derive(Deserialize)]
pub struct UpdateProducerMetadataRequest {
    pub name: String,
    pub icon: Option<String>,
    pub domain: Option<String>,
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
            branch: None,
            action_type: Some("WRITE"),
            diff: None,
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
            service: None,
            branch: None,
            action_type: Some("ADMIN"),
            diff: None,
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
            service: None,
            branch: None,
            action_type: Some("ADMIN"),
            diff: None,
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
            details: "Cleared service and branch caches",
            service: None,
            branch: None,
            action_type: Some("ADMIN"),
            diff: None,
        },
    )
    .await?;
    Ok(Json(json!({ "cleared": true })))
}

pub async fn get_observability_audit_logs(
    State(state): State<AppState>,
) -> Result<impl IntoResponse, AppError> {
    let logs: Vec<AuditLogEntry> = state.repo.get_recent_audit_logs(30).await?;
    Ok(Json(logs))
}

#[derive(Deserialize)]
pub struct UpdateEndpointRequest {
    pub producername: String,
    pub branch: String,
    pub api_type: ApiType,
    pub path: String,
    pub method: String,
    pub yaml: String,
    pub deprecated: bool,
    pub external: bool,
}

pub async fn admin_update_endpoint(
    State(state): State<AppState>,
    user: Option<axum::Extension<crate::domain::models::User>>,
    actor: Option<axum::Extension<crate::domain::permissions::Actor>>,
    Json(payload): Json<UpdateEndpointRequest>,
) -> Result<impl IntoResponse, AppError> {
    // Editing a Producer's spec is the maintainer's own work, so their scoped
    // `manage_producers` counts here — checked against the Producer the payload
    // names, before anything is touched.
    let acting = actor_of(actor)?;
    crate::application::authz::require_producer_permission(
        &state.repo,
        &acting,
        crate::domain::permissions::Permission::ManageProducers,
        &payload.producername,
    )
    .await?;

    services::update_endpoint_manual(
        &state.repo,
        services::RequireEndpointParams {
            consumername: "_admin",
            producername: &payload.producername,
            branch: &payload.branch,
            api_type: payload.api_type,
            path: &payload.path,
            method: &payload.method,
            timeout_secs: None,
            source_protected_branch: None,
            pull_from_branch: None,
        },
        payload.api_type,
        payload.yaml,
        payload.deprecated,
        payload.external,
    )
    .await?;

    let _ = state.spec_updated_tx.send(());

    record_audit_log(
        &state.repo,
        user,
        NewAuditLog {
            action: "MANUAL_UPDATE_ENDPOINT",
            details: &format!(
                "Manually updated endpoint {} {} in {} ({})",
                payload.method, payload.path, payload.producername, payload.branch
            ),
            service: Some(&payload.producername),
            branch: Some(&payload.branch),
            action_type: Some("WRITE"),
            diff: None,
        },
    )
    .await?;

    Ok(StatusCode::OK)
}

#[cfg(test)]
mod tests {
    use super::redact_database_url;

    #[test]
    fn strips_the_password_from_a_postgres_url() {
        assert_eq!(
            redact_database_url("postgres://user:secret@db.example.com:5432/sanshain"),
            "postgres://user@db.example.com:5432/sanshain"
        );
        assert_eq!(
            redact_database_url("postgresql://admin:hunter2@localhost/app"),
            "postgresql://admin@localhost/app"
        );
    }

    #[test]
    fn keeps_urls_that_carry_no_credentials_intact() {
        assert_eq!(
            redact_database_url("postgres://db.example.com:5432/sanshain"),
            "postgres://db.example.com:5432/sanshain"
        );
        assert_eq!(
            redact_database_url("postgres://user@localhost/app"),
            "postgres://user@localhost/app"
        );
        assert_eq!(
            redact_database_url("sqlite:sanshain.db?mode=rwc"),
            "sqlite:sanshain.db?mode=rwc"
        );
        assert_eq!(redact_database_url("sqlite::memory:"), "sqlite::memory:");
    }

    #[test]
    fn a_password_containing_an_at_sign_cannot_leak_into_the_host() {
        // Userinfo must split at the LAST '@' of the authority; splitting at the
        // first would leave "ss@host" as the host and emit the rest of the
        // password verbatim.
        let redacted = redact_database_url("postgres://user:p@ss@db.example.com/app");
        assert_eq!(redacted, "postgres://user@db.example.com/app");
        assert!(
            !redacted.contains("ss"),
            "password fragment leaked: {redacted}"
        );
    }

    #[test]
    fn redacts_password_query_parameters() {
        assert_eq!(
            redact_database_url("postgres://user:secret@host/app?sslmode=require&password=p1"),
            "postgres://user@host/app?sslmode=require&password=***"
        );
        // Also on schemes with no authority section.
        assert_eq!(
            redact_database_url("sqlite:app.db?password=p1&mode=rwc"),
            "sqlite:app.db?password=***&mode=rwc"
        );
        // Case-insensitive, and matches embedded names like `sslpassword`.
        assert_eq!(
            redact_database_url("postgres://host/app?SSLPassword=p1"),
            "postgres://host/app?SSLPassword=***"
        );
    }

    #[test]
    fn never_emits_a_known_secret_for_any_shape() {
        for url in [
            "postgres://user:sup3rsecret@host:5432/db",
            "postgres://user:sup3rsecret@host/db?password=sup3rsecret",
            "postgres://:sup3rsecret@host/db",
            "postgresql://u:sup3rsecret@h/d?sslpassword=sup3rsecret&x=1",
        ] {
            let redacted = redact_database_url(url);
            assert!(
                !redacted.contains("sup3rsecret"),
                "secret survived redaction of {url}: {redacted}"
            );
        }
    }
}

// --- Producer Onboarding ---

#[derive(Deserialize)]
pub struct SetOnboardingRequest {
    pub onboarding: bool,
}

pub async fn admin_get_producer_onboarding(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    let onboarding = services::is_producer_onboarding(&state.repo, &name).await?;
    Ok(Json(json!({ "producer": name, "onboarding": onboarding })))
}

pub async fn admin_set_producer_onboarding(
    State(state): State<AppState>,
    user: Option<axum::Extension<crate::domain::models::User>>,
    Path(name): Path<String>,
    Json(payload): Json<SetOnboardingRequest>,
) -> Result<impl IntoResponse, AppError> {
    services::set_producer_onboarding(&state.repo, &name, payload.onboarding).await?;
    record_audit_log(
        &state.repo,
        user,
        NewAuditLog {
            action: if payload.onboarding {
                "START_ONBOARDING"
            } else {
                "END_ONBOARDING"
            },
            details: &format!(
                "Producer '{}' {} onboarding",
                name,
                if payload.onboarding {
                    "entered"
                } else {
                    "left"
                }
            ),
            service: Some(&name),
            branch: None,
            action_type: Some("ADMIN"),
            diff: None,
        },
    )
    .await?;
    Ok(StatusCode::OK)
}

/// Which Producers are currently in onboarding.
///
/// Onboarding never expires, so without this an operator has no way of noticing
/// that a Producer stopped being gatekept months ago.
pub async fn admin_list_onboarding_producers(
    State(state): State<AppState>,
) -> Result<impl IntoResponse, AppError> {
    let producers = services::list_onboarding_producers(&state.repo).await?;
    Ok(Json(json!({ "producers": producers })))
}

// --- Reviewing held Provides ---

/// The Actor behind the request, for the Producer-scoped checks below.
///
/// These routes are not gated on a Producer by the router: which Producer a held
/// spec belongs to is only known once the entry is loaded, so the check lives
/// here instead.
fn actor_of(
    actor: Option<axum::Extension<crate::domain::permissions::Actor>>,
) -> Result<crate::domain::permissions::Actor, AppError> {
    actor
        .map(|axum::Extension(a)| a)
        .ok_or(AppError::Unauthorized)
}

/// Held Provides the caller may act on.
///
/// Filtered rather than refused: an administrator sees everything, a maintainer
/// sees the Producers they are responsible for, and anyone else sees an empty
/// inbox rather than a 403 on a page they are allowed to open.
pub async fn admin_list_pending_specs(
    State(state): State<AppState>,
    actor: Option<axum::Extension<crate::domain::permissions::Actor>>,
) -> Result<impl IntoResponse, AppError> {
    let actor = actor_of(actor)?;
    let all = services::list_pending_specs(&state.repo).await?;

    let mut visible = Vec::new();
    for pending in all {
        if crate::application::authz::require_producer_permission(
            &state.repo,
            &actor,
            crate::domain::permissions::Permission::ReviewPendingSpecs,
            &pending.producer,
        )
        .await
        .is_ok()
        {
            visible.push(pending);
        }
    }

    Ok(Json(json!({ "count": visible.len(), "pending": visible })))
}

pub async fn admin_get_pending_spec(
    State(state): State<AppState>,
    actor: Option<axum::Extension<crate::domain::permissions::Actor>>,
    Path(id): Path<i64>,
) -> Result<impl IntoResponse, AppError> {
    let actor = actor_of(actor)?;
    let pending = services::get_pending_spec(&state.repo, id).await?;
    crate::application::authz::require_producer_permission(
        &state.repo,
        &actor,
        crate::domain::permissions::Permission::ReviewPendingSpecs,
        &pending.producer,
    )
    .await?;
    Ok(Json(pending))
}

pub async fn admin_accept_pending_spec(
    State(state): State<AppState>,
    actor: Option<axum::Extension<crate::domain::permissions::Actor>>,
    Path(id): Path<i64>,
) -> Result<impl IntoResponse, AppError> {
    let actor = actor_of(actor)?;
    let pending = services::get_pending_spec(&state.repo, id).await?;
    crate::application::authz::require_producer_permission(
        &state.repo,
        &actor,
        crate::domain::permissions::Permission::ReviewPendingSpecs,
        &pending.producer,
    )
    .await?;

    let response = services::apply_pending_spec(&state.repo, id, Some(&actor.username)).await?;
    let _ = state.spec_updated_tx.send(());
    Ok(Json(response))
}

pub async fn admin_reject_pending_spec(
    State(state): State<AppState>,
    actor: Option<axum::Extension<crate::domain::permissions::Actor>>,
    Path(id): Path<i64>,
) -> Result<impl IntoResponse, AppError> {
    let actor = actor_of(actor)?;
    let pending = services::get_pending_spec(&state.repo, id).await?;
    crate::application::authz::require_producer_permission(
        &state.repo,
        &actor,
        crate::domain::permissions::Permission::ReviewPendingSpecs,
        &pending.producer,
    )
    .await?;

    services::reject_pending_spec(&state.repo, id, Some(&actor.username)).await?;
    Ok(StatusCode::OK)
}
