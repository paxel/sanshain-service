pub mod admin;
pub mod api;
pub mod auth;
pub mod pages;
pub mod roles;

pub(super) use crate::domain::models::DEV_MODE_ACTOR;

/// The acting username for a write that records its own provenance. One
/// resolution for every handler module — the local copies this replaces each
/// hardcoded the sentinel string themselves.
pub(super) fn actor_or_dev_mode(user: Option<&crate::domain::models::User>) -> String {
    user.map(|u| u.username.clone())
        .unwrap_or_else(|| DEV_MODE_ACTOR.to_string())
}

/// Record an audit entry attributed to the request's caller, given the plain
/// user the handler already holds.
pub(super) async fn record_audit_log_for(
    repo: &impl crate::domain::ports::SpecRepository,
    user: Option<&crate::domain::models::User>,
    log: crate::domain::ports::NewAuditLog<'_>,
) -> Result<(), crate::domain::models::AppError> {
    repo.insert_audit_log(&actor_or_dev_mode(user), log)
        .await
        .map_err(|e| crate::domain::models::AppError::Internal(e.to_string()))
}

/// The same, for handlers that take the user as an axum extension.
pub(super) async fn record_audit_log(
    repo: &impl crate::domain::ports::SpecRepository,
    user: Option<axum::Extension<crate::domain::models::User>>,
    log: crate::domain::ports::NewAuditLog<'_>,
) -> Result<(), crate::domain::models::AppError> {
    record_audit_log_for(repo, user.as_ref().map(|axum::Extension(u)| u), log).await
}
