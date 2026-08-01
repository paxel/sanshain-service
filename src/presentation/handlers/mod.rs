pub mod admin;
pub mod api;
pub mod auth;
pub mod pages;
pub mod roles;

/// Record an audit entry attributed to the request's caller.
///
/// Shared by the handler modules so "who acted" is resolved one way everywhere:
/// the authenticated user, or the dev-mode fallback when there is none.
pub(super) async fn record_audit_log(
    repo: &impl crate::domain::ports::SpecRepository,
    user: Option<axum::Extension<crate::domain::models::User>>,
    log: crate::domain::ports::NewAuditLog<'_>,
) -> Result<(), crate::domain::models::AppError> {
    let actor = if let Some(axum::Extension(u)) = user {
        u.username.clone()
    } else {
        "DevMode/Anonymous".to_string()
    };
    repo.insert_audit_log(&actor, log)
        .await
        .map_err(|e| crate::domain::models::AppError::Internal(e.to_string()))
}
