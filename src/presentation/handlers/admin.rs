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

async fn record_audit_log(
    repo: &impl crate::domain::ports::SpecRepository,
    user: Option<axum::Extension<crate::domain::models::User>>,
    log: NewAuditLog<'_>,
) -> Result<(), AppError> {
    let actor = if let Some(axum::Extension(u)) = user {
        u.username.clone()
    } else {
        "DevMode/Anonymous".to_string()
    };
    repo.insert_audit_log(&actor, log)
        .await
        .map_err(|e| AppError::Internal(e.to_string()))
}

pub async fn admin_list_services(
    State(state): State<AppState>,
    user: Option<axum::Extension<crate::domain::models::User>>,
) -> Result<impl IntoResponse, AppError> {
    let user_id = user.map(|axum::Extension(u)| u.id);
    let res = services::list_services_detailed(&state.repo, user_id).await?;
    Ok(Json(res))
}

pub async fn admin_list_branches(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    let res = services::list_branches(&state.repo, &name).await?;
    Ok(Json(res))
}

pub async fn admin_list_clients(
    State(state): State<AppState>,
    user: Option<axum::Extension<crate::domain::models::User>>,
) -> Result<impl IntoResponse, AppError> {
    let user_id = user.map(|axum::Extension(u)| u.id);
    let res = services::list_clients(&state.repo, user_id).await?;
    Ok(Json(res))
}

pub async fn admin_list_client_branches(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    let res = services::list_client_branches(&state.repo, &name).await?;
    Ok(Json(res))
}

pub async fn admin_list_client_endpoints(
    State(state): State<AppState>,
    Path((name, branch)): Path<(String, String)>,
) -> Result<impl IntoResponse, AppError> {
    let res = services::list_client_endpoints(&state.repo, &name, &branch).await?;
    Ok(Json(res))
}

pub async fn admin_list_service_endpoints(
    State(state): State<AppState>,
    Path((name, branch)): Path<(String, String)>,
) -> Result<impl IntoResponse, AppError> {
    let res = services::list_service_endpoints(&state.repo, &name, &branch).await?;
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
    Ok((
        [(
            axum::http::header::CONTENT_TYPE,
            "text/plain; charset=utf-8",
        )],
        spec,
    ))
}

pub async fn admin_list_all_branches(
    State(state): State<AppState>,
) -> Result<impl IntoResponse, AppError> {
    let res = services::list_all_branches(&state.repo).await?;
    Ok(Json(res))
}

#[derive(Deserialize)]
pub struct AdminEndpointYamlQuery {
    pub servicename: String,
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
        &query.servicename,
        &query.branch,
        query.api_type,
        &query.path,
        &query.method,
    )
    .await?;
    Ok(res)
}

pub async fn admin_get_endpoint_versions(
    State(state): State<AppState>,
    Query(query): Query<AdminEndpointYamlQuery>,
) -> Result<impl IntoResponse, AppError> {
    let res = services::get_endpoint_version_history(
        &state.repo,
        &query.servicename,
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

pub async fn admin_delete_service(
    State(state): State<AppState>,
    user: Option<axum::Extension<crate::domain::models::User>>,
    Path(name): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    if services::delete_service(&state.repo, &name).await? {
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

pub async fn admin_delete_client(
    State(state): State<AppState>,
    user: Option<axum::Extension<crate::domain::models::User>>,
    Path(name): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    if services::delete_client(&state.repo, &name).await? {
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

pub async fn admin_list_users(
    State(state): State<AppState>,
) -> Result<impl IntoResponse, AppError> {
    let res = services::list_users(&state.repo).await?;
    Ok(Json(res))
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

pub async fn admin_nuke_services(
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

pub async fn admin_nuke_clients(
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
pub struct UpdateServiceMetadataRequest {
    pub name: String,
    pub icon: Option<String>,
    pub domain: Option<String>,
}

pub async fn admin_update_service_metadata(
    State(state): State<AppState>,
    user: Option<axum::Extension<crate::domain::models::User>>,
    Json(payload): Json<UpdateServiceMetadataRequest>,
) -> Result<impl IntoResponse, AppError> {
    services::update_service_metadata(
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
    if let Some(axum::Extension(ref u)) = user
        && !u.is_admin
    {
        return Err(AppError::Forbidden);
    }
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
    pub servicename: String,
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
    Json(payload): Json<UpdateEndpointRequest>,
) -> Result<impl IntoResponse, AppError> {
    services::update_endpoint_manual(
        &state.repo,
        services::RequireEndpointParams {
            clientname: "_admin",
            servicename: &payload.servicename,
            branch: &payload.branch,
            api_type: payload.api_type,
            path: &payload.path,
            method: &payload.method,
            timeout_secs: None,
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
                payload.method, payload.path, payload.servicename, payload.branch
            ),
            service: Some(&payload.servicename),
            branch: Some(&payload.branch),
            action_type: Some("WRITE"),
            diff: None,
        },
    )
    .await?;

    Ok(StatusCode::OK)
}
