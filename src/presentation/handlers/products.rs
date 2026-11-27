//! Products (#76, ADR-0011): the `/admin/products*` routes. Thin: every rule
//! lives in `product_service`; each state change is audited with the Actor.

use crate::AppState;
use crate::application::services::{self, AppError};
use crate::domain::models::User;
use crate::domain::ports::NewAuditLog;
use axum::{
    Extension, Json,
    extract::{Path, Query, State},
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
/// `Chart.yaml`. With a chart `version` it creates or replaces that
/// snapshot (san-1033) and, as the highest version, sets the members; without
/// one it sets the members only, as before.
pub async fn import_chart(
    State(state): State<AppState>,
    user: Option<Extension<User>>,
    Path(name): Path<String>,
    Json(payload): Json<ImportChartRequest>,
) -> Result<impl IntoResponse, AppError> {
    let actor = user.as_ref().map(|u| u.username.clone());
    let report = services::push_product_chart(
        &state.repo,
        &name,
        &payload.chart_yaml,
        None,
        actor.as_deref(),
        false,
    )
    .await?;
    audit_chart(&state, user, &name, &report).await?;
    Ok(Json(report))
}

async fn audit_chart(
    state: &AppState,
    user: Option<Extension<User>>,
    name: &str,
    report: &services::ChartPushReport,
) -> Result<(), AppError> {
    let members = match &report.members {
        Some(m) => format!(
            "added {}; removed {}; skipped {}",
            listed(&m.added),
            listed(&m.removed),
            listed(&m.skipped)
        ),
        None => "members unchanged".to_string(),
    };
    let snapshot = match (&report.chart_version, &report.outcome) {
        (Some(v), Some(o)) => format!("snapshot {v} {o:?}"),
        _ => "no snapshot (the chart has no version)".to_string(),
    };
    record(
        state,
        user,
        "IMPORT_PRODUCT_CHART",
        &format!("Imported a Chart.yaml into Product '{name}': {snapshot}; {members}"),
    )
    .await
}

#[derive(Deserialize)]
pub struct PushChartQuery {
    pub scope: Option<String>,
}

/// `PUT /products/{name}/chart` (san-1033): a Helm pipeline pushes the
/// umbrella `Chart.yaml` as the body, with a full API token of a Product
/// Maintainer or a holder of `manage_producers`.
pub async fn push_chart(
    State(state): State<AppState>,
    user: Option<Extension<User>>,
    Path(name): Path<String>,
    Query(query): Query<PushChartQuery>,
    body: String,
) -> Result<impl IntoResponse, AppError> {
    let actor = user.as_ref().map(|u| u.username.clone());
    let report = services::push_product_chart(
        &state.repo,
        &name,
        &body,
        query.scope.as_deref(),
        actor.as_deref(),
        true,
    )
    .await?;
    audit_chart(&state, user, &name, &report).await?;
    Ok(Json(report))
}

/// The snapshots of a Product, the highest chart version first.
pub async fn list_snapshots(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    Ok(Json(
        services::list_product_snapshots(&state.repo, &name).await?,
    ))
}

/// The release sheet of one snapshot.
pub async fn snapshot_sheet(
    State(state): State<AppState>,
    Path((name, version)): Path<(String, String)>,
) -> Result<impl IntoResponse, AppError> {
    Ok(Json(
        services::product_release_sheet(&state.repo, &name, &version).await?,
    ))
}

#[derive(Deserialize)]
pub struct EditSnapshotRequest {
    /// Present: set it (blank clears); absent: leave it.
    #[serde(default, deserialize_with = "present_option")]
    pub sales_name: Option<Option<String>>,
    #[serde(default)]
    pub scope: Option<String>,
}

/// A field that is present, `null` included, is `Some`.
fn present_option<'de, D>(deserializer: D) -> Result<Option<Option<String>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Ok(Some(Option::<String>::deserialize(deserializer)?))
}

/// Set a snapshot's sales name or scope; a later push keeps them.
pub async fn edit_snapshot(
    State(state): State<AppState>,
    user: Option<Extension<User>>,
    Path((name, version)): Path<(String, String)>,
    Json(payload): Json<EditSnapshotRequest>,
) -> Result<impl IntoResponse, AppError> {
    let snapshot = services::edit_product_snapshot(
        &state.repo,
        &name,
        &version,
        payload.sales_name.as_ref().map(|n| n.as_deref()),
        payload.scope.as_deref(),
    )
    .await?;
    record(
        &state,
        user,
        "EDIT_PRODUCT_SNAPSHOT",
        &format!(
            "Set snapshot {version} of Product '{name}': sales name {}, scope {}",
            snapshot.sales_name.as_deref().unwrap_or("none"),
            snapshot.scope
        ),
    )
    .await?;
    Ok(Json(snapshot))
}

#[derive(Deserialize)]
pub struct CompareSnapshotsQuery {
    pub from: String,
    pub to: String,
}

/// Two snapshots compared: per member the API versions that moved.
pub async fn compare_snapshots(
    State(state): State<AppState>,
    Path(name): Path<String>,
    Query(query): Query<CompareSnapshotsQuery>,
) -> Result<impl IntoResponse, AppError> {
    Ok(Json(
        services::compare_product_snapshots(&state.repo, &name, &query.from, &query.to).await?,
    ))
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
