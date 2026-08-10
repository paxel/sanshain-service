pub mod admin;
pub mod api;
pub mod auth;
pub mod pages;
pub mod roles;

/// Stand-in actor when no authenticated user is attached (dev mode). Provenance
/// must never read as blank — every audited path uses this same sentinel.
pub(super) const DEV_MODE_ACTOR: &str = "DevMode/Anonymous";

/// The acting username for a write that records its own provenance, rather
/// than going through [`record_audit_log`]. Same resolution, same sentinel —
/// provenance must never read as blank, and it must not depend on which
/// extension a given handler happens to take.
pub(super) fn actor_or_dev_mode(
    user: Option<&axum::Extension<crate::domain::models::User>>,
) -> String {
    user.map(|axum::Extension(u)| u.username.clone())
        .unwrap_or_else(|| DEV_MODE_ACTOR.to_string())
}

/// Record an audit entry attributed to the request's caller.
///
/// Shared by the handler modules so "who acted" is resolved one way everywhere:
/// the authenticated user, or the dev-mode fallback when there is none.
pub(super) async fn record_audit_log(
    repo: &impl crate::domain::ports::SpecRepository,
    user: Option<axum::Extension<crate::domain::models::User>>,
    log: crate::domain::ports::NewAuditLog<'_>,
) -> Result<(), crate::domain::models::AppError> {
    let actor = actor_or_dev_mode(user.as_ref());
    repo.insert_audit_log(&actor, log)
        .await
        .map_err(|e| crate::domain::models::AppError::Internal(e.to_string()))
}
