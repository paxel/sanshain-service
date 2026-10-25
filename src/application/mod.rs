pub mod admin_service;
pub mod auth_service;
pub mod authz;
pub mod background;
pub mod branch_service;
pub mod catalog_service;
pub mod dashboard_service;
pub mod directory_roles;
pub mod mcp_service;
pub mod provide_service;
pub mod report_service;
pub mod require_service;
pub mod service_account_service;
pub mod sign_in_service;
pub mod spec_service;
pub mod structure_service;
pub mod text_diff;
pub mod version_rules;
pub mod webhook_delivery;
pub mod webhook_service;

#[cfg(any(test, feature = "test-support"))]
pub mod mock_repo;

pub mod services;

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

/// `count` days as a span, saturating at chrono's limit: `TimeDelta::days`
/// panics past it.
pub(crate) fn days(count: u64) -> chrono::TimeDelta {
    i64::try_from(count)
        .ok()
        .and_then(chrono::TimeDelta::try_days)
        .unwrap_or(chrono::TimeDelta::MAX)
}

/// `count` hours as a span, saturating like [`days`].
pub(crate) fn hours(count: u64) -> chrono::TimeDelta {
    i64::try_from(count)
        .ok()
        .and_then(chrono::TimeDelta::try_hours)
        .unwrap_or(chrono::TimeDelta::MAX)
}

/// `count` seconds as a span, saturating like [`days`] (towards the past for
/// a negative count).
pub(crate) fn seconds(count: i64) -> chrono::TimeDelta {
    chrono::TimeDelta::try_seconds(count).unwrap_or(if count < 0 {
        chrono::TimeDelta::MIN
    } else {
        chrono::TimeDelta::MAX
    })
}

/// `span` after `at`, clamped to the latest time chrono can hold (where `+`
/// would panic).
pub(crate) fn after(
    at: chrono::DateTime<chrono::Utc>,
    span: chrono::TimeDelta,
) -> chrono::DateTime<chrono::Utc> {
    at.checked_add_signed(span)
        .unwrap_or(chrono::DateTime::<chrono::Utc>::MAX_UTC)
}

/// `span` before `at`, clamped to the earliest time chrono can hold (where
/// `-` would panic).
pub(crate) fn before(
    at: chrono::DateTime<chrono::Utc>,
    span: chrono::TimeDelta,
) -> chrono::DateTime<chrono::Utc> {
    at.checked_sub_signed(span)
        .unwrap_or(chrono::DateTime::<chrono::Utc>::MIN_UTC)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{DateTime, TimeDelta, TimeZone, Utc};

    fn noon() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 10, 25, 12, 0, 0).unwrap()
    }

    #[test]
    fn spans_count_days_and_hours() {
        assert_eq!(days(7), TimeDelta::try_days(7).unwrap());
        assert_eq!(hours(36), TimeDelta::try_hours(36).unwrap());
        assert_eq!(seconds(-90), TimeDelta::try_seconds(-90).unwrap());
    }

    #[test]
    fn spans_past_chronos_range_saturate_instead_of_panicking() {
        assert_eq!(days(u64::MAX), TimeDelta::MAX);
        assert_eq!(hours(u64::MAX), TimeDelta::MAX);
        assert_eq!(days(i64::MAX as u64 / 86_400 + 1), TimeDelta::MAX);
        assert_eq!(seconds(i64::MAX), TimeDelta::MAX);
        assert_eq!(seconds(i64::MIN), TimeDelta::MIN);
    }

    #[test]
    fn after_and_before_move_the_time() {
        assert_eq!(
            after(noon(), days(7)),
            Utc.with_ymd_and_hms(2026, 11, 1, 12, 0, 0).unwrap()
        );
        assert_eq!(
            before(noon(), hours(12)),
            Utc.with_ymd_and_hms(2026, 10, 25, 0, 0, 0).unwrap()
        );
    }

    #[test]
    fn after_and_before_clamp_at_chronos_ends() {
        assert_eq!(after(noon(), days(u64::MAX)), DateTime::<Utc>::MAX_UTC);
        assert_eq!(before(noon(), days(u64::MAX)), DateTime::<Utc>::MIN_UTC);
    }
}
