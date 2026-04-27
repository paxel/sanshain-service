use crate::AppState;
use crate::application::services::{self, AppError};
use crate::domain::models::{ApiType, AuthMode, LdapConfig};
use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
};
use serde::Deserialize;
use serde_json::json;
use std::str::FromStr;

pub async fn admin_list_services(
    State(state): State<AppState>,
) -> Result<impl IntoResponse, AppError> {
    let res = services::list_services_detailed(&state.repo).await?;
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
) -> Result<impl IntoResponse, AppError> {
    let res = services::list_clients(&state.repo).await?;
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
    Json(payload): Json<AddProtectedBranchRequest>,
) -> Result<impl IntoResponse, AppError> {
    services::add_protected_branch(&state.repo, &payload.pattern).await?;
    Ok(StatusCode::CREATED)
}

pub async fn delete_protected_branch(
    State(state): State<AppState>,
    Path(pattern): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    if services::remove_protected_branch(&state.repo, &pattern).await? {
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
    Path(name): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    if services::delete_service(&state.repo, &name).await? {
        Ok(StatusCode::OK)
    } else {
        Err(AppError::NotFound(format!("Service not found: {}", name)))
    }
}

pub async fn admin_delete_branch(
    State(state): State<AppState>,
    Path((name, branch)): Path<(String, String)>,
) -> Result<impl IntoResponse, AppError> {
    if services::delete_branch(&state.repo, &name, &branch).await? {
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
    Path(name): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    if services::delete_client(&state.repo, &name).await? {
        Ok(StatusCode::OK)
    } else {
        Err(AppError::NotFound(format!("Client not found: {}", name)))
    }
}

pub async fn get_dev_mode(State(state): State<AppState>) -> Result<impl IntoResponse, AppError> {
    let res = services::get_dev_mode(&state.repo).await?;
    Ok(Json(res))
}

#[derive(Deserialize)]
pub struct EnabledRequest {
    pub enabled: bool,
}

pub async fn set_dev_mode(
    State(state): State<AppState>,
    Json(payload): Json<EnabledRequest>,
) -> Result<impl IntoResponse, AppError> {
    services::set_dev_mode(&state.repo, payload.enabled).await?;
    Ok(StatusCode::OK)
}

pub async fn get_local_users(State(state): State<AppState>) -> Result<impl IntoResponse, AppError> {
    let res = services::get_local_users_enabled(&state.repo).await?;
    Ok(Json(res))
}

pub async fn set_local_users(
    State(state): State<AppState>,
    Json(payload): Json<EnabledRequest>,
) -> Result<impl IntoResponse, AppError> {
    services::set_local_users_enabled(&state.repo, payload.enabled).await?;
    Ok(StatusCode::OK)
}

pub async fn get_auto_approve_users(
    State(state): State<AppState>,
) -> Result<impl IntoResponse, AppError> {
    let res = services::get_auto_approve_users(&state.repo).await?;
    Ok(Json(res))
}

pub async fn set_auto_approve_users(
    State(state): State<AppState>,
    Json(payload): Json<EnabledRequest>,
) -> Result<impl IntoResponse, AppError> {
    services::set_auto_approve_users(&state.repo, payload.enabled).await?;
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
    if let Some(ldap) = payload.ldap_config {
        services::set_ldap_config(&state.repo, &ldap).await?;
    }
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
    Json(payload): Json<MaxAgeDaysPayload>,
) -> Result<impl IntoResponse, AppError> {
    if payload.days == 0 {
        return Err(AppError::BadRequest(
            "days must be greater than 0".to_string(),
        ));
    }
    services::set_branch_max_age_days(&state.repo, payload.days).await?;
    Ok(StatusCode::OK)
}

pub async fn trigger_branch_cleanup(
    State(state): State<AppState>,
) -> Result<impl IntoResponse, AppError> {
    let res = services::cleanup_stale_branches(&state.repo).await?;
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
    Json(payload): Json<MaxAgeDaysPayload>,
) -> Result<impl IntoResponse, AppError> {
    if payload.days == 0 {
        return Err(AppError::BadRequest(
            "days must be greater than 0".to_string(),
        ));
    }
    services::set_dependency_max_age_days(&state.repo, payload.days).await?;
    Ok(StatusCode::OK)
}

pub async fn trigger_dependency_cleanup(
    State(state): State<AppState>,
) -> Result<impl IntoResponse, AppError> {
    let res = services::cleanup_stale_dependencies(&state.repo).await?;
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
    Path(id): Path<i64>,
) -> Result<impl IntoResponse, AppError> {
    if services::approve_user(&state.repo, id).await? {
        Ok(StatusCode::OK)
    } else {
        Err(AppError::NotFound(format!("User with ID {} not found", id)))
    }
}

pub async fn admin_delete_user_handler(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Result<impl IntoResponse, AppError> {
    if services::admin_delete_user(&state.repo, id).await? {
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
    Json(payload): Json<NukeConfirmPayload>,
) -> Result<impl IntoResponse, AppError> {
    if payload.confirmation != "DELETE ALL SERVICES" {
        return Err(AppError::BadRequest("Invalid confirmation".to_string()));
    }
    let res = services::delete_all_services(&state.repo).await?;
    Ok(Json(json!({ "deleted": res })))
}

pub async fn admin_nuke_clients(
    State(state): State<AppState>,
    Json(payload): Json<NukeConfirmPayload>,
) -> Result<impl IntoResponse, AppError> {
    if payload.confirmation != "DELETE ALL CLIENTS" {
        return Err(AppError::BadRequest("Invalid confirmation".to_string()));
    }
    let res = services::delete_all_clients(&state.repo).await?;
    Ok(Json(json!({ "deleted": res })))
}

pub async fn admin_nuke_users(
    State(state): State<AppState>,
    Json(payload): Json<NukeConfirmPayload>,
) -> Result<impl IntoResponse, AppError> {
    if payload.confirmation != "DELETE ALL USERS" {
        return Err(AppError::BadRequest("Invalid confirmation".to_string()));
    }
    let res = services::delete_all_non_admin_users(&state.repo).await?;
    Ok(Json(json!({ "deleted": res })))
}

pub async fn admin_nuke_database(
    State(state): State<AppState>,
    axum::Extension(user): axum::Extension<crate::domain::models::User>,
    Json(payload): Json<NukeConfirmPayload>,
) -> Result<impl IntoResponse, AppError> {
    if payload.confirmation != "NUKE DATABASE" {
        return Err(AppError::BadRequest("Invalid confirmation".to_string()));
    }
    services::nuke_database(&state.repo, Some(user.id)).await?;
    Ok(StatusCode::OK)
}

pub async fn admin_nuke_branch(
    State(state): State<AppState>,
    Path(branch): Path<String>,
    Json(payload): Json<NukeConfirmPayload>,
) -> Result<impl IntoResponse, AppError> {
    if payload.confirmation != format!("DELETE BRANCH {}", branch) {
        return Err(AppError::BadRequest("Invalid confirmation".to_string()));
    }
    let res = services::delete_branch_all_services(&state.repo, &branch).await?;
    Ok(Json(res))
}

pub async fn get_observability_stats(
    State(state): State<AppState>,
) -> Result<impl IntoResponse, AppError> {
    use crate::domain::models::SystemStats;
    let stats = SystemStats {
        requests_total: state
            .requests_total
            .load(std::sync::atomic::Ordering::Relaxed),
        failures_total: state
            .failures_total
            .load(std::sync::atomic::Ordering::Relaxed),
        process_uptime: (chrono::Utc::now() - state.process_start_time)
            .num_seconds()
            .max(0) as u64,
        ..Default::default()
    };
    Ok(Json(stats))
}

pub async fn get_observability_logs(
    State(state): State<AppState>,
) -> Result<impl IntoResponse, AppError> {
    use crate::domain::models::LogResponse;
    let errors = state.error_buffer.lock().unwrap().iter().cloned().collect();
    let warnings = state.warn_buffer.lock().unwrap().iter().cloned().collect();
    let infos = state.info_buffer.lock().unwrap().iter().cloned().collect();
    let debugs = state.debug_buffer.lock().unwrap().iter().cloned().collect();
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
    axum::Extension(user): axum::Extension<crate::domain::models::User>,
    Json(config): Json<crate::domain::models::DebugConfig>,
) -> Result<impl IntoResponse, AppError> {
    if !user.is_admin {
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
    Ok(StatusCode::OK)
}
