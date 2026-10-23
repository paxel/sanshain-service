//! The landing page's dashboard: how big the instance is and how busy it has
//! been. The numbers are public; the names of what was provided are for
//! signed-in users only, so an anonymous visitor learns nothing about which
//! services exist.

use crate::domain::models::{AppError, AuditLogEntry, AuditLogFilter};
use crate::domain::ports::SpecRepository;
use chrono::{DateTime, Duration, NaiveDate, Utc};
use serde::Serialize;

/// How many days the activity chart covers, today included.
pub const ACTIVITY_DAYS: usize = 30;
/// How many of the latest provides a signed-in user sees.
pub const RECENT_PROVIDES: usize = 10;
/// The audit action a provide that changed a spec writes.
const PROVIDE_ACTION: &str = "PROVIDE_SPEC";
/// Upper bound on the audit entries read for the chart. The endpoint is
/// public, so one request must stay cheap; an instance writing more audit
/// entries than this in a month undercounts its oldest days.
const ACTIVITY_SCAN_LIMIT: u32 = 20_000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DayCount {
    /// `YYYY-MM-DD`, UTC.
    pub day: String,
    pub provides: u32,
}

/// What anyone may see: counts only, no names.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DashboardStats {
    pub producers: usize,
    pub consumers: usize,
    pub versions: usize,
    pub provides_last_7_days: u32,
    /// Oldest first, one entry per day, days without a provide included.
    pub provides_per_day: Vec<DayCount>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RecentProvide {
    pub timestamp: String,
    pub producer: Option<String>,
    pub version: Option<String>,
    pub username: String,
}

/// The public numbers of the dashboard, counted up to `now`.
pub async fn dashboard_stats(
    repo: &impl SpecRepository,
    now: DateTime<Utc>,
) -> Result<DashboardStats, AppError> {
    let producers = repo.list_producers().await?.len();
    let consumers = repo.list_consumers().await?.len();
    let versions = repo.list_all_spec_versions().await?.len();

    let today = now.date_naive();
    let first = today - Duration::days(ACTIVITY_DAYS as i64 - 1);
    let entries = repo
        .get_audit_logs(AuditLogFilter {
            from_date: Some(first.format("%Y-%m-%d").to_string()),
            to_date: None,
            action_type: None,
            service_wildcard: None,
            version_wildcard: None,
            stream: None,
            branch_id: None,
            limit: ACTIVITY_SCAN_LIMIT,
        })
        .await?;
    let provides_per_day = per_day(&entries, first, ACTIVITY_DAYS);
    let provides_last_7_days = provides_per_day
        .iter()
        .rev()
        .take(7)
        .map(|d| d.provides)
        .sum();

    Ok(DashboardStats {
        producers,
        consumers,
        versions,
        provides_last_7_days,
        provides_per_day,
    })
}

/// The latest provides that changed a spec, newest first.
pub async fn recent_provides(repo: &impl SpecRepository) -> Result<Vec<RecentProvide>, AppError> {
    // Other actions are interleaved; read enough to find ten provides.
    let entries = repo
        .get_audit_logs(AuditLogFilter {
            from_date: None,
            to_date: None,
            action_type: Some("WRITE".to_string()),
            service_wildcard: None,
            version_wildcard: None,
            stream: None,
            branch_id: None,
            limit: 500,
        })
        .await?;
    Ok(entries
        .into_iter()
        .filter(|e| e.action == PROVIDE_ACTION)
        .take(RECENT_PROVIDES)
        .map(|e| RecentProvide {
            timestamp: e.timestamp,
            producer: e.service,
            version: e.version,
            username: e.username,
        })
        .collect())
}

/// Count the provides among `entries` per day, for `days` days from `first`.
fn per_day(entries: &[AuditLogEntry], first: NaiveDate, days: usize) -> Vec<DayCount> {
    let mut counts = vec![0u32; days];
    for entry in entries.iter().filter(|e| e.action == PROVIDE_ACTION) {
        let Ok(stamp) = DateTime::parse_from_rfc3339(&entry.timestamp) else {
            continue;
        };
        let offset = (stamp.with_timezone(&Utc).date_naive() - first).num_days();
        if let Ok(index) = usize::try_from(offset)
            && let Some(count) = counts.get_mut(index)
        {
            *count += 1;
        }
    }
    counts
        .into_iter()
        .enumerate()
        .map(|(i, provides)| DayCount {
            day: (first + Duration::days(i as i64))
                .format("%Y-%m-%d")
                .to_string(),
            provides,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(action: &str, timestamp: &str) -> AuditLogEntry {
        AuditLogEntry {
            id: 0,
            timestamp: timestamp.to_string(),
            username: "ci".to_string(),
            action: action.to_string(),
            details: String::new(),
            service: Some("penguin".to_string()),
            version: Some("1.0.0".to_string()),
            action_type: Some("WRITE".to_string()),
            diff: None,
            stream: None,
        }
    }

    #[test]
    fn per_day_counts_provides_only_and_keeps_empty_days() {
        let first = NaiveDate::from_ymd_opt(2026, 10, 1).unwrap();
        let entries = [
            entry("PROVIDE_SPEC", "2026-10-01T08:00:00+00:00"),
            entry("PROVIDE_SPEC", "2026-10-01T23:59:59Z"),
            entry("DELETE_SPEC", "2026-10-02T08:00:00Z"),
            entry("PROVIDE_SPEC", "2026-10-03T00:30:00+02:00"),
            entry("PROVIDE_SPEC", "2026-09-30T23:00:00Z"),
            entry("PROVIDE_SPEC", "not a time"),
        ];
        let days = per_day(&entries, first, 3);
        assert_eq!(
            days,
            vec![
                DayCount {
                    day: "2026-10-01".into(),
                    provides: 2
                },
                DayCount {
                    day: "2026-10-02".into(),
                    provides: 1
                },
                DayCount {
                    day: "2026-10-03".into(),
                    provides: 0
                },
            ],
            "+02:00 on the 3rd is the 2nd in UTC; the day before the window is dropped"
        );
    }
}
