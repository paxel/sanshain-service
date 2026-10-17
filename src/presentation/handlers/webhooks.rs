//! Webhooks (#47): hook management in three scopes — global under
//! `/admin/webhooks`, per Producer and per Consumer under their admin
//! paths — plus the egress allow-list setting. Thin: scope resolution,
//! JSON shapes and audit entries; the rules live in the application layer.

use crate::AppState;
use crate::application::services::AppError;
use crate::application::webhook_service as service;
use crate::domain::models::{Webhook, WebhookScope};
use crate::domain::ports::{NewAuditLog, SpecRepository};
use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::Deserialize;
use serde_json::json;

use super::record_audit_log;

type UserExt = Option<axum::Extension<crate::domain::models::User>>;
type ActorExt = Option<axum::Extension<crate::domain::permissions::Actor>>;

fn producer(name: String) -> WebhookScope {
    WebhookScope::Producer { name }
}

fn consumer(name: String) -> WebhookScope {
    WebhookScope::Consumer { name }
}

/// One audit entry per hook act; the system's name goes in `service` for
/// system hooks so the audit filter by Producer finds them.
async fn audit(
    repo: &impl SpecRepository,
    user: UserExt,
    action: &str,
    details: &str,
    scope: &WebhookScope,
) -> Result<(), AppError> {
    record_audit_log(
        repo,
        user,
        NewAuditLog {
            action,
            details,
            service: scope.name(),
            action_type: Some("ADMIN"),
            ..Default::default()
        },
    )
    .await
}

/// The audit log is readable by every `view_audit` holder, and for a Slack
/// or Teams incoming webhook the URL's path *is* the credential — so an
/// entry names the hook, its scope and the receiver's host, never the URL.
fn describe(hook: &Webhook) -> String {
    let host = url::Url::parse(&hook.url)
        .ok()
        .and_then(|u| u.host_str().map(str::to_string))
        .unwrap_or_else(|| "?".to_string());
    format!(
        "webhook {} ({} {}, {} format, host {host})",
        hook.id,
        hook.scope.kind(),
        hook.scope.name().unwrap_or("instance-wide"),
        hook.format.as_str()
    )
}

// --- The scope-independent bodies ---

async fn list_in(state: &AppState, scope: WebhookScope) -> Result<Response, AppError> {
    Ok((Json(service::list_webhooks_in(&state.repo, &scope).await?)).into_response())
}

async fn create_in(
    state: &AppState,
    actor: ActorExt,
    user: UserExt,
    scope: WebhookScope,
    request: service::CreateWebhookRequest,
) -> Result<Response, AppError> {
    let axum::Extension(actor) = actor.ok_or(AppError::Unauthorized)?;
    let created = service::create_webhook(&state.repo, &actor, scope.clone(), request).await?;
    audit(
        &state.repo,
        user,
        "WEBHOOK_CREATED",
        &format!("Registered {}", describe(&created.webhook)),
        &scope,
    )
    .await?;
    Ok((StatusCode::CREATED, Json(created)).into_response())
}

async fn get_in(state: &AppState, scope: WebhookScope, id: i64) -> Result<Response, AppError> {
    Ok((Json(service::get_webhook_in(&state.repo, &scope, id).await?)).into_response())
}

async fn patch_in(
    state: &AppState,
    user: UserExt,
    scope: WebhookScope,
    id: i64,
    request: service::PatchWebhookRequest,
) -> Result<Response, AppError> {
    let before = service::get_webhook_in(&state.repo, &scope, id).await?;
    let paused = request.paused;
    let hook = service::patch_webhook(&state.repo, &scope, id, request).await?;
    match paused {
        Some(true) if !before.paused => {
            audit(
                &state.repo,
                user,
                "WEBHOOK_PAUSED",
                &format!("Paused {}", describe(&hook)),
                &scope,
            )
            .await?;
        }
        Some(false) if before.paused => {
            audit(
                &state.repo,
                user,
                "WEBHOOK_RESUMED",
                &format!("Resumed {}", describe(&hook)),
                &scope,
            )
            .await?;
        }
        _ => {}
    }
    Ok((Json(hook)).into_response())
}

async fn delete_in(
    state: &AppState,
    user: UserExt,
    scope: WebhookScope,
    id: i64,
) -> Result<Response, AppError> {
    let hook = service::delete_webhook_in(&state.repo, &scope, id).await?;
    audit(
        &state.repo,
        user,
        "WEBHOOK_DELETED",
        &format!("Deleted {}", describe(&hook)),
        &scope,
    )
    .await?;
    Ok((StatusCode::NO_CONTENT).into_response())
}

async fn test_in(state: &AppState, scope: WebhookScope, id: i64) -> Result<Response, AppError> {
    let event_id = service::send_test_event(&state.repo, &scope, id).await?;
    Ok((StatusCode::ACCEPTED, Json(json!({ "event_id": event_id }))).into_response())
}

async fn rotate_in(
    state: &AppState,
    user: UserExt,
    scope: WebhookScope,
    id: i64,
) -> Result<Response, AppError> {
    let rotated = service::rotate_webhook_secret(&state.repo, &scope, id).await?;
    audit(
        &state.repo,
        user,
        "WEBHOOK_SECRET_ROTATED",
        &format!("Rotated the secret of {}", describe(&rotated.webhook)),
        &scope,
    )
    .await?;
    Ok((Json(rotated)).into_response())
}

#[derive(Deserialize)]
pub struct DeliveriesQuery {
    pub limit: Option<u32>,
}

async fn deliveries_in(
    state: &AppState,
    scope: WebhookScope,
    id: i64,
    limit: Option<u32>,
) -> Result<Response, AppError> {
    Ok((Json(service::list_deliveries_in(&state.repo, &scope, id, limit).await?)).into_response())
}

async fn redeliver_in(
    state: &AppState,
    user: UserExt,
    scope: WebhookScope,
    id: i64,
    delivery_id: i64,
) -> Result<Response, AppError> {
    let delivery = service::redeliver(&state.repo, &scope, id, delivery_id).await?;
    audit(
        &state.repo,
        user,
        "WEBHOOK_REDELIVERED",
        &format!(
            "Redelivered delivery {} ({}) of webhook {}",
            delivery.id,
            delivery.event_type.as_str(),
            id
        ),
        &scope,
    )
    .await?;
    Ok((StatusCode::ACCEPTED, Json(delivery)).into_response())
}

// --- Global hooks: /admin/webhooks ---

pub async fn list_global(State(state): State<AppState>) -> Result<impl IntoResponse, AppError> {
    list_in(&state, WebhookScope::Global).await
}

pub async fn create_global(
    State(state): State<AppState>,
    actor: ActorExt,
    user: UserExt,
    Json(request): Json<service::CreateWebhookRequest>,
) -> Result<impl IntoResponse, AppError> {
    create_in(&state, actor, user, WebhookScope::Global, request).await
}

pub async fn get_global(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Result<impl IntoResponse, AppError> {
    get_in(&state, WebhookScope::Global, id).await
}

pub async fn patch_global(
    State(state): State<AppState>,
    user: UserExt,
    Path(id): Path<i64>,
    Json(request): Json<service::PatchWebhookRequest>,
) -> Result<impl IntoResponse, AppError> {
    patch_in(&state, user, WebhookScope::Global, id, request).await
}

pub async fn delete_global(
    State(state): State<AppState>,
    user: UserExt,
    Path(id): Path<i64>,
) -> Result<impl IntoResponse, AppError> {
    delete_in(&state, user, WebhookScope::Global, id).await
}

pub async fn test_global(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Result<impl IntoResponse, AppError> {
    test_in(&state, WebhookScope::Global, id).await
}

pub async fn rotate_global(
    State(state): State<AppState>,
    user: UserExt,
    Path(id): Path<i64>,
) -> Result<impl IntoResponse, AppError> {
    rotate_in(&state, user, WebhookScope::Global, id).await
}

pub async fn deliveries_global(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Query(query): Query<DeliveriesQuery>,
) -> Result<impl IntoResponse, AppError> {
    deliveries_in(&state, WebhookScope::Global, id, query.limit).await
}

pub async fn redeliver_global(
    State(state): State<AppState>,
    user: UserExt,
    Path((id, delivery_id)): Path<(i64, i64)>,
) -> Result<impl IntoResponse, AppError> {
    redeliver_in(&state, user, WebhookScope::Global, id, delivery_id).await
}

// --- Producer hooks: /admin/producers/{name}/webhooks ---

pub async fn list_producer(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    list_in(&state, producer(name)).await
}

pub async fn create_producer(
    State(state): State<AppState>,
    actor: ActorExt,
    user: UserExt,
    Path(name): Path<String>,
    Json(request): Json<service::CreateWebhookRequest>,
) -> Result<impl IntoResponse, AppError> {
    create_in(&state, actor, user, producer(name), request).await
}

pub async fn get_producer(
    State(state): State<AppState>,
    Path((name, id)): Path<(String, i64)>,
) -> Result<impl IntoResponse, AppError> {
    get_in(&state, producer(name), id).await
}

pub async fn patch_producer(
    State(state): State<AppState>,
    user: UserExt,
    Path((name, id)): Path<(String, i64)>,
    Json(request): Json<service::PatchWebhookRequest>,
) -> Result<impl IntoResponse, AppError> {
    patch_in(&state, user, producer(name), id, request).await
}

pub async fn delete_producer(
    State(state): State<AppState>,
    user: UserExt,
    Path((name, id)): Path<(String, i64)>,
) -> Result<impl IntoResponse, AppError> {
    delete_in(&state, user, producer(name), id).await
}

pub async fn test_producer(
    State(state): State<AppState>,
    Path((name, id)): Path<(String, i64)>,
) -> Result<impl IntoResponse, AppError> {
    test_in(&state, producer(name), id).await
}

pub async fn rotate_producer(
    State(state): State<AppState>,
    user: UserExt,
    Path((name, id)): Path<(String, i64)>,
) -> Result<impl IntoResponse, AppError> {
    rotate_in(&state, user, producer(name), id).await
}

pub async fn deliveries_producer(
    State(state): State<AppState>,
    Path((name, id)): Path<(String, i64)>,
    Query(query): Query<DeliveriesQuery>,
) -> Result<impl IntoResponse, AppError> {
    deliveries_in(&state, producer(name), id, query.limit).await
}

pub async fn redeliver_producer(
    State(state): State<AppState>,
    user: UserExt,
    Path((name, id, delivery_id)): Path<(String, i64, i64)>,
) -> Result<impl IntoResponse, AppError> {
    redeliver_in(&state, user, producer(name), id, delivery_id).await
}

// --- Consumer hooks: /admin/consumers/{name}/webhooks ---

pub async fn list_consumer(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    list_in(&state, consumer(name)).await
}

pub async fn create_consumer(
    State(state): State<AppState>,
    actor: ActorExt,
    user: UserExt,
    Path(name): Path<String>,
    Json(request): Json<service::CreateWebhookRequest>,
) -> Result<impl IntoResponse, AppError> {
    create_in(&state, actor, user, consumer(name), request).await
}

pub async fn get_consumer(
    State(state): State<AppState>,
    Path((name, id)): Path<(String, i64)>,
) -> Result<impl IntoResponse, AppError> {
    get_in(&state, consumer(name), id).await
}

pub async fn patch_consumer(
    State(state): State<AppState>,
    user: UserExt,
    Path((name, id)): Path<(String, i64)>,
    Json(request): Json<service::PatchWebhookRequest>,
) -> Result<impl IntoResponse, AppError> {
    patch_in(&state, user, consumer(name), id, request).await
}

pub async fn delete_consumer(
    State(state): State<AppState>,
    user: UserExt,
    Path((name, id)): Path<(String, i64)>,
) -> Result<impl IntoResponse, AppError> {
    delete_in(&state, user, consumer(name), id).await
}

pub async fn test_consumer(
    State(state): State<AppState>,
    Path((name, id)): Path<(String, i64)>,
) -> Result<impl IntoResponse, AppError> {
    test_in(&state, consumer(name), id).await
}

pub async fn rotate_consumer(
    State(state): State<AppState>,
    user: UserExt,
    Path((name, id)): Path<(String, i64)>,
) -> Result<impl IntoResponse, AppError> {
    rotate_in(&state, user, consumer(name), id).await
}

pub async fn deliveries_consumer(
    State(state): State<AppState>,
    Path((name, id)): Path<(String, i64)>,
    Query(query): Query<DeliveriesQuery>,
) -> Result<impl IntoResponse, AppError> {
    deliveries_in(&state, consumer(name), id, query.limit).await
}

pub async fn redeliver_consumer(
    State(state): State<AppState>,
    user: UserExt,
    Path((name, id, delivery_id)): Path<(String, i64, i64)>,
) -> Result<impl IntoResponse, AppError> {
    redeliver_in(&state, user, consumer(name), id, delivery_id).await
}

// --- The allow-list: /admin/settings/webhook-allowed-hosts ---

pub async fn get_allowed_hosts(
    State(state): State<AppState>,
) -> Result<impl IntoResponse, AppError> {
    let hosts = service::webhook_allowed_hosts(&state.repo).await?;
    Ok(Json(json!({ "hosts": hosts })))
}

#[derive(Deserialize)]
pub struct AllowedHostsPayload {
    pub hosts: Vec<String>,
}

pub async fn set_allowed_hosts(
    State(state): State<AppState>,
    user: UserExt,
    Json(payload): Json<AllowedHostsPayload>,
) -> Result<impl IntoResponse, AppError> {
    let hosts = service::set_webhook_allowed_hosts(&state.repo, &payload.hosts).await?;
    record_audit_log(
        &state.repo,
        user,
        NewAuditLog {
            action: "SET_WEBHOOK_ALLOWED_HOSTS",
            details: &format!(
                "Set the webhook allow-list to: {}",
                if hosts.is_empty() {
                    "(empty)".to_string()
                } else {
                    hosts.join(", ")
                }
            ),
            action_type: Some("ADMIN"),
            ..Default::default()
        },
    )
    .await?;
    Ok(Json(json!({ "hosts": hosts })))
}
