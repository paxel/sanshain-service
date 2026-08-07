pub mod admin_service;
pub mod auth_service;

/// The one clock-to-text conversion for stored stamps: UTC, second precision,
/// `Z` suffix — the shape lexicographic timestamp comparisons in SQL rely on.
pub(crate) fn now_iso() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}
pub mod authz;
pub mod branch_service;
pub mod directory_roles;
pub mod report_service;
pub mod spec_service;

#[cfg(any(test, feature = "test-support"))]
pub mod mock_repo;

pub mod services;
