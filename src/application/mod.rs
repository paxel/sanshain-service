pub mod admin_service;
pub mod auth_service;

/// The one clock-to-text conversion for stored stamps: UTC, second precision,
/// `Z` suffix — the shape lexicographic timestamp comparisons in SQL rely on.
pub(crate) fn now_iso() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

/// Run a CPU-bound computation (Argon2 hashing, YAML parse/split/merge) on
/// the blocking pool. Such work costs real CPU time by design; inline it
/// would stall the async worker thread — and every request scheduled on it —
/// for the whole computation. Request-path callers must use this; one-off
/// startup paths may compute inline.
pub(crate) async fn run_cpu_bound<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, crate::domain::models::AppError> + Send + 'static,
) -> Result<T, crate::domain::models::AppError> {
    tokio::task::spawn_blocking(work).await.map_err(|e| {
        crate::domain::models::AppError::Internal(format!("Blocking task failed: {}", e))
    })?
}
pub mod authz;
pub mod branch_service;
pub mod directory_roles;
pub mod provide_service;
pub mod report_service;
pub mod require_service;
pub mod spec_service;
pub mod version_rules;

#[cfg(any(test, feature = "test-support"))]
pub mod mock_repo;

pub mod services;
