//! Administering roles and groups.
//!
//! Kept out of `admin.rs` because this is a self-contained surface: everything
//! here answers "who may do what", and nothing here touches specs.

use crate::AppState;
use crate::application::authz;
use crate::application::services::AppError;
use crate::domain::permissions::{Permission, Role};
use crate::domain::ports::NewAuditLog;
use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
};
use serde::{Deserialize, Serialize};

/// Every entry from this module is administrative and Producer-agnostic, so
/// only the action and its details vary; attribution goes through the shared
/// helper in `handlers`.
async fn record(
    state: &AppState,
    user: Option<axum::Extension<crate::domain::models::User>>,
    action: &str,
    details: &str,
) -> Result<(), AppError> {
    super::record_audit_log(
        &state.repo,
        user,
        NewAuditLog {
            action,
            details,
            action_type: Some("ADMIN"),
            ..Default::default()
        },
    )
    .await
}

#[derive(Serialize)]
pub struct RoleCatalogueEntry {
    pub role: &'static str,
    pub permissions: Vec<&'static str>,
    /// Whether this role can be granted instance-wide, as opposed to being a
    /// scope over particular Producers.
    pub globally_grantable: bool,
}

/// The role catalogue.
///
/// Roles are fixed in code, so the UI cannot compose them — it can only show
/// what each one confers, which is what an administrator needs in order to
/// understand what they are handing over.
pub async fn list_roles() -> Result<impl IntoResponse, AppError> {
    let catalogue: Vec<RoleCatalogueEntry> = Role::ALL
        .iter()
        .map(|role| RoleCatalogueEntry {
            role: role.as_str(),
            permissions: role.permissions().iter().map(|p| p.as_str()).collect(),
            globally_grantable: Role::GLOBALLY_GRANTABLE.contains(role),
        })
        .collect();
    Ok(Json(serde_json::json!({
        "roles": catalogue,
        "permissions": Permission::ALL.iter().map(|p| p.as_str()).collect::<Vec<_>>(),
    })))
}

pub async fn list_user_roles(
    State(state): State<AppState>,
    Path(user_id): Path<i64>,
) -> Result<impl IntoResponse, AppError> {
    let roles = authz::list_user_roles(&state.repo, user_id).await?;
    Ok(Json(serde_json::json!({ "roles": roles })))
}

#[derive(Deserialize)]
pub struct GrantRoleRequest {
    pub role: String,
}

pub async fn grant_user_role(
    State(state): State<AppState>,
    user: Option<axum::Extension<crate::domain::models::User>>,
    actor: Option<axum::Extension<crate::domain::permissions::Actor>>,
    Path(user_id): Path<i64>,
    Json(payload): Json<GrantRoleRequest>,
) -> Result<impl IntoResponse, AppError> {
    let actor = actor.map(|axum::Extension(a)| a);
    authz::grant_user_role(&state.repo, actor.as_ref(), user_id, &payload.role).await?;
    record(
        &state,
        user,
        "GRANT_ROLE",
        &format!("Granted role '{}' to user {}", payload.role, user_id),
    )
    .await?;
    Ok(StatusCode::CREATED)
}

pub async fn revoke_user_role(
    State(state): State<AppState>,
    user: Option<axum::Extension<crate::domain::models::User>>,
    actor: Option<axum::Extension<crate::domain::permissions::Actor>>,
    Path((user_id, role)): Path<(i64, String)>,
) -> Result<impl IntoResponse, AppError> {
    let actor = actor.map(|axum::Extension(a)| a);
    if !authz::revoke_user_role(&state.repo, actor.as_ref(), user_id, &role).await? {
        return Err(AppError::NotFound(format!(
            "User {} does not hold role '{}'",
            user_id, role
        )));
    }
    record(
        &state,
        user,
        "REVOKE_ROLE",
        &format!("Revoked role '{}' from user {}", role, user_id),
    )
    .await?;
    Ok(StatusCode::OK)
}

pub async fn list_groups(State(state): State<AppState>) -> Result<impl IntoResponse, AppError> {
    let groups = authz::list_groups(&state.repo).await?;
    Ok(Json(serde_json::json!({ "groups": groups })))
}

#[derive(Deserialize)]
pub struct CreateGroupRequest {
    pub name: String,
}

pub async fn create_group(
    State(state): State<AppState>,
    user: Option<axum::Extension<crate::domain::models::User>>,
    Json(payload): Json<CreateGroupRequest>,
) -> Result<impl IntoResponse, AppError> {
    let group = authz::create_native_group(&state.repo, &payload.name).await?;
    record(
        &state,
        user,
        "CREATE_GROUP",
        &format!("Created group '{}'", group.name),
    )
    .await?;
    Ok((StatusCode::CREATED, Json(group)))
}

#[derive(Deserialize)]
pub struct UpdateGroupRequest {
    /// Renames the group when present. Only Sanshain's own groups may be
    /// renamed; a directory group's name belongs to the directory.
    #[serde(default)]
    pub name: Option<String>,
    /// Replaces the group's role grants when present.
    #[serde(default)]
    pub roles: Option<Vec<String>>,
}

pub async fn update_group(
    State(state): State<AppState>,
    user: Option<axum::Extension<crate::domain::models::User>>,
    actor: Option<axum::Extension<crate::domain::permissions::Actor>>,
    Path(group_id): Path<i64>,
    Json(payload): Json<UpdateGroupRequest>,
) -> Result<impl IntoResponse, AppError> {
    let actor = actor.map(|axum::Extension(a)| a);
    let mut changes = Vec::new();
    if let Some(name) = &payload.name {
        authz::rename_group(&state.repo, group_id, name).await?;
        changes.push(format!("renamed to '{}'", name));
    }
    if let Some(roles) = &payload.roles {
        authz::set_group_roles(&state.repo, actor.as_ref(), group_id, roles).await?;
        changes.push(format!("roles set to [{}]", roles.join(", ")));
    }
    if changes.is_empty() {
        return Err(AppError::BadRequest(
            "Nothing to update: supply a name, roles, or both.".to_string(),
        ));
    }
    record(
        &state,
        user,
        "UPDATE_GROUP",
        &format!("Group {}: {}", group_id, changes.join("; ")),
    )
    .await?;
    Ok(StatusCode::OK)
}

pub async fn delete_group(
    State(state): State<AppState>,
    user: Option<axum::Extension<crate::domain::models::User>>,
    actor: Option<axum::Extension<crate::domain::permissions::Actor>>,
    Path(group_id): Path<i64>,
) -> Result<impl IntoResponse, AppError> {
    let actor = actor.map(|axum::Extension(a)| a);
    if !authz::delete_group(&state.repo, actor.as_ref(), group_id).await? {
        return Err(AppError::NotFound(format!("Group {} not found", group_id)));
    }
    record(
        &state,
        user,
        "DELETE_GROUP",
        &format!("Deleted group {}", group_id),
    )
    .await?;
    Ok(StatusCode::OK)
}

#[derive(Deserialize)]
pub struct AddMemberRequest {
    pub user_id: i64,
}

pub async fn add_group_member(
    State(state): State<AppState>,
    user: Option<axum::Extension<crate::domain::models::User>>,
    actor: Option<axum::Extension<crate::domain::permissions::Actor>>,
    Path(group_id): Path<i64>,
    Json(payload): Json<AddMemberRequest>,
) -> Result<impl IntoResponse, AppError> {
    let actor = actor.map(|axum::Extension(a)| a);
    authz::add_group_member(&state.repo, actor.as_ref(), group_id, payload.user_id).await?;
    record(
        &state,
        user,
        "ADD_GROUP_MEMBER",
        &format!("Added user {} to group {}", payload.user_id, group_id),
    )
    .await?;
    Ok(StatusCode::CREATED)
}

pub async fn remove_group_member(
    State(state): State<AppState>,
    user: Option<axum::Extension<crate::domain::models::User>>,
    actor: Option<axum::Extension<crate::domain::permissions::Actor>>,
    Path((group_id, user_id)): Path<(i64, i64)>,
) -> Result<impl IntoResponse, AppError> {
    let actor = actor.map(|axum::Extension(a)| a);
    if !authz::remove_group_member(&state.repo, actor.as_ref(), group_id, user_id).await? {
        return Err(AppError::NotFound(format!(
            "User {} is not a member of group {}",
            user_id, group_id
        )));
    }
    record(
        &state,
        user,
        "REMOVE_GROUP_MEMBER",
        &format!("Removed user {} from group {}", user_id, group_id),
    )
    .await?;
    Ok(StatusCode::OK)
}

// --- Maintainer scope ---

/// Every Producer with its maintainer sets, in one response.
pub async fn list_all_maintainers(
    State(state): State<AppState>,
) -> Result<impl IntoResponse, AppError> {
    let producers = authz::all_maintainers(&state.repo).await?;
    Ok(Json(serde_json::json!({ "producers": producers })))
}

pub async fn list_maintainers(
    State(state): State<AppState>,
    Path(producer): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    let maintainers = authz::list_maintainers(&state.repo, &producer).await?;
    Ok(Json(maintainers))
}

#[derive(Deserialize)]
pub struct AssignMaintainerRequest {
    #[serde(default)]
    pub user_id: Option<i64>,
    #[serde(default)]
    pub group_id: Option<i64>,
}

pub async fn assign_maintainer(
    State(state): State<AppState>,
    user: Option<axum::Extension<crate::domain::models::User>>,
    Path(producer): Path<String>,
    Json(payload): Json<AssignMaintainerRequest>,
) -> Result<impl IntoResponse, AppError> {
    // Exactly one, because a row assigns either a user or a group and accepting
    // both would silently make two assignments from one request.
    let details = match (payload.user_id, payload.group_id) {
        (Some(user_id), None) => {
            authz::assign_user_maintainer(&state.repo, &producer, user_id).await?;
            format!("User {} now maintains '{}'", user_id, producer)
        }
        (None, Some(group_id)) => {
            authz::assign_group_maintainer(&state.repo, &producer, group_id).await?;
            format!("Group {} now maintains '{}'", group_id, producer)
        }
        _ => {
            return Err(AppError::BadRequest(
                "Supply exactly one of user_id or group_id.".to_string(),
            ));
        }
    };
    record(&state, user, "ASSIGN_MAINTAINER", &details).await?;
    Ok(StatusCode::CREATED)
}

pub async fn unassign_user_maintainer(
    State(state): State<AppState>,
    user: Option<axum::Extension<crate::domain::models::User>>,
    Path((producer, user_id)): Path<(String, i64)>,
) -> Result<impl IntoResponse, AppError> {
    if !authz::unassign_user_maintainer(&state.repo, &producer, user_id).await? {
        return Err(AppError::NotFound(format!(
            "User {} does not maintain '{}'",
            user_id, producer
        )));
    }
    record(
        &state,
        user,
        "UNASSIGN_MAINTAINER",
        &format!("User {} no longer maintains '{}'", user_id, producer),
    )
    .await?;
    Ok(StatusCode::OK)
}

pub async fn unassign_group_maintainer(
    State(state): State<AppState>,
    user: Option<axum::Extension<crate::domain::models::User>>,
    Path((producer, group_id)): Path<(String, i64)>,
) -> Result<impl IntoResponse, AppError> {
    if !authz::unassign_group_maintainer(&state.repo, &producer, group_id).await? {
        return Err(AppError::NotFound(format!(
            "Group {} does not maintain '{}'",
            group_id, producer
        )));
    }
    record(
        &state,
        user,
        "UNASSIGN_MAINTAINER",
        &format!("Group {} no longer maintains '{}'", group_id, producer),
    )
    .await?;
    Ok(StatusCode::OK)
}

/// The Producers a user is responsible for.
pub async fn list_maintained_producers(
    State(state): State<AppState>,
    Path(user_id): Path<i64>,
) -> Result<impl IntoResponse, AppError> {
    let producers = authz::maintained_producers(&state.repo, user_id).await?;
    Ok(Json(serde_json::json!({ "producers": producers })))
}
