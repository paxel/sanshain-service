//! Products (#76, ADR-0011): the `/admin/products*` routes. Thin: every rule
//! lives in `product_service`; each state change is audited with the Actor.

use crate::AppState;
use crate::application::services::{self, AppError};
use crate::domain::models::User;
use crate::domain::ports::NewAuditLog;
use axum::{
    Extension, Json,
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
};
use serde::Deserialize;
use serde_json::json;

async fn record(
    state: &AppState,
    user: Option<Extension<User>>,
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

/// `GET /admin/products`: every Product with its member count.
pub async fn list_products(State(state): State<AppState>) -> Result<impl IntoResponse, AppError> {
    let products = services::list_products(&state.repo).await?;
    Ok(Json(json!({ "products": products })))
}

#[derive(Deserialize)]
pub struct CreateProductRequest {
    pub name: String,
}

/// `POST /admin/products`: create an empty Product.
pub async fn create_product(
    State(state): State<AppState>,
    user: Option<Extension<User>>,
    Json(payload): Json<CreateProductRequest>,
) -> Result<impl IntoResponse, AppError> {
    let product = services::create_product(&state.repo, &payload.name).await?;
    record(
        &state,
        user,
        "CREATE_PRODUCT",
        &format!("Created Product '{}'", product.name),
    )
    .await?;
    Ok((
        StatusCode::CREATED,
        Json(json!({ "name": product.name, "created_at": product.created_at })),
    ))
}

/// `GET /admin/products/{name}`: members with their roles, and maintainers.
pub async fn get_product(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    Ok(Json(services::describe_product(&state.repo, &name).await?))
}

/// `DELETE /admin/products/{name}`: the envelope goes, its systems stay.
pub async fn delete_product(
    State(state): State<AppState>,
    user: Option<Extension<User>>,
    Path(name): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    services::delete_product(&state.repo, &name).await?;
    record(
        &state,
        user,
        "DELETE_PRODUCT",
        &format!("Deleted Product '{name}'"),
    )
    .await?;
    Ok(Json(json!({ "deleted": name })))
}

#[derive(Deserialize)]
pub struct AddMemberRequest {
    pub system: String,
}

/// `POST /admin/products/{name}/members`: add a known system by hand.
pub async fn add_member(
    State(state): State<AppState>,
    user: Option<Extension<User>>,
    Path(name): Path<String>,
    Json(payload): Json<AddMemberRequest>,
) -> Result<impl IntoResponse, AppError> {
    let system = payload.system.trim().to_string();
    let added = services::add_product_member(&state.repo, &name, &system).await?;
    if added {
        record(
            &state,
            user,
            "ADD_PRODUCT_MEMBER",
            &format!("Added '{system}' to Product '{name}' by hand"),
        )
        .await?;
    }
    let status = if added {
        StatusCode::CREATED
    } else {
        StatusCode::OK
    };
    Ok((status, Json(json!({ "system": system, "added": added }))))
}

/// `DELETE /admin/products/{name}/members/{system}`.
pub async fn remove_member(
    State(state): State<AppState>,
    user: Option<Extension<User>>,
    Path((name, system)): Path<(String, String)>,
) -> Result<impl IntoResponse, AppError> {
    services::remove_product_member(&state.repo, &name, &system).await?;
    record(
        &state,
        user,
        "REMOVE_PRODUCT_MEMBER",
        &format!("Removed '{system}' from Product '{name}'"),
    )
    .await?;
    Ok(Json(json!({ "removed": system })))
}

#[derive(Deserialize)]
pub struct ImportChartRequest {
    pub chart_yaml: String,
}

fn listed(names: &[String]) -> String {
    if names.is_empty() {
        "none".to_string()
    } else {
        names.join(", ")
    }
}

/// `POST /admin/products/{name}/chart`: apply a pasted umbrella
/// `Chart.yaml` directly and answer what it added, removed and skipped.
pub async fn import_chart(
    State(state): State<AppState>,
    user: Option<Extension<User>>,
    Path(name): Path<String>,
    Json(payload): Json<ImportChartRequest>,
) -> Result<impl IntoResponse, AppError> {
    let report = services::import_product_chart(&state.repo, &name, &payload.chart_yaml).await?;
    record(
        &state,
        user,
        "IMPORT_PRODUCT_CHART",
        &format!(
            "Imported a Chart.yaml into Product '{name}': added {}; removed {}; skipped {}",
            listed(&report.added),
            listed(&report.removed),
            listed(&report.skipped)
        ),
    )
    .await?;
    Ok(Json(report))
}

#[derive(Deserialize)]
pub struct AssignProductMaintainerRequest {
    #[serde(default)]
    pub user_id: Option<i64>,
    #[serde(default)]
    pub group_id: Option<i64>,
}

/// `POST /admin/products/{name}/maintainers`: exactly one of a user or a
/// group, like the Producer maintainer assignment.
pub async fn assign_maintainer(
    State(state): State<AppState>,
    user: Option<Extension<User>>,
    Path(name): Path<String>,
    Json(payload): Json<AssignProductMaintainerRequest>,
) -> Result<impl IntoResponse, AppError> {
    let details = match (payload.user_id, payload.group_id) {
        (Some(user_id), None) => {
            services::assign_product_user_maintainer(&state.repo, &name, user_id).await?;
            format!("User {user_id} now maintains Product '{name}'")
        }
        (None, Some(group_id)) => {
            services::assign_product_group_maintainer(&state.repo, &name, group_id).await?;
            format!("Group {group_id} now maintains Product '{name}'")
        }
        _ => {
            return Err(AppError::BadRequest(
                "Supply exactly one of user_id or group_id.".to_string(),
            ));
        }
    };
    record(&state, user, "ASSIGN_PRODUCT_MAINTAINER", &details).await?;
    Ok(StatusCode::CREATED)
}

/// `DELETE /admin/products/{name}/maintainers/users/{user_id}`.
pub async fn unassign_user_maintainer(
    State(state): State<AppState>,
    user: Option<Extension<User>>,
    Path((name, user_id)): Path<(String, i64)>,
) -> Result<impl IntoResponse, AppError> {
    if !services::unassign_product_user_maintainer(&state.repo, &name, user_id).await? {
        return Err(AppError::NotFound(format!(
            "User {user_id} does not maintain Product '{name}'"
        )));
    }
    record(
        &state,
        user,
        "UNASSIGN_PRODUCT_MAINTAINER",
        &format!("User {user_id} no longer maintains Product '{name}'"),
    )
    .await?;
    Ok(StatusCode::OK)
}

/// `DELETE /admin/products/{name}/maintainers/groups/{group_id}`.
pub async fn unassign_group_maintainer(
    State(state): State<AppState>,
    user: Option<Extension<User>>,
    Path((name, group_id)): Path<(String, i64)>,
) -> Result<impl IntoResponse, AppError> {
    if !services::unassign_product_group_maintainer(&state.repo, &name, group_id).await? {
        return Err(AppError::NotFound(format!(
            "Group {group_id} does not maintain Product '{name}'"
        )));
    }
    record(
        &state,
        user,
        "UNASSIGN_PRODUCT_MAINTAINER",
        &format!("Group {group_id} no longer maintains Product '{name}'"),
    )
    .await?;
    Ok(StatusCode::OK)
}
