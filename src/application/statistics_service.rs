//! The statistics page (san-1021): counts of what Sanshain holds, for
//! architects, management and maintainers, one tab per topic.
//!
//! Everything is aggregated here, on the server: a `report_viewer` holds
//! `view_statistics` and nothing else, so no answer may carry a person's name
//! or a single audit entry, not even one the page would hide. Activity is
//! counted per system and rolled up to the system's Backstage owner.

use crate::domain::models::{
    ApiType, AppError, AuditLogFilter, MemberTrack, PeripheryTrack, SystemContext,
};
use crate::domain::ports::SpecRepository;
use chrono::{Datelike, Days, Months, NaiveDate};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

/// The range the page shows when it names none: the last 90 days, today
/// included.
pub const DEFAULT_RANGE_DAYS: u64 = 90;

/// The days a statistics answer covers, both ends included, in UTC.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct StatisticsRange {
    pub from: NaiveDate,
    pub to: NaiveDate,
}

impl StatisticsRange {
    /// The range of `from`/`to` (`YYYY-MM-DD`): a missing `to` is today, a
    /// missing `from` is [`DEFAULT_RANGE_DAYS`] before `to`. A range that ends
    /// before it starts, or a date that is not one, is a 400.
    pub fn parse(
        from: Option<&str>,
        to: Option<&str>,
        today: NaiveDate,
    ) -> Result<StatisticsRange, AppError> {
        let day = |name: &str, raw: &str| {
            NaiveDate::parse_from_str(raw, "%Y-%m-%d").map_err(|_| {
                AppError::BadRequest(format!("{name} must be a date (YYYY-MM-DD), got '{raw}'"))
            })
        };
        let to = match to.filter(|s| !s.is_empty()) {
            Some(raw) => day("to", raw)?,
            None => today,
        };
        let from = match from.filter(|s| !s.is_empty()) {
            Some(raw) => day("from", raw)?,
            None => to
                .checked_sub_days(Days::new(DEFAULT_RANGE_DAYS.saturating_sub(1)))
                .unwrap_or(NaiveDate::MIN),
        };
        if from > to {
            return Err(AppError::BadRequest(format!(
                "the range ends ({to}) before it starts ({from})"
            )));
        }
        Ok(StatisticsRange { from, to })
    }

    /// Every day of the range, oldest first.
    pub fn days(&self) -> Vec<NaiveDate> {
        self.from
            .iter_days()
            .take_while(|d| *d <= self.to)
            .collect()
    }
}

/// The tabs of the statistics page that have data of their own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatisticsTab {
    Landscape,
}

impl StatisticsTab {
    /// The tab a route segment names, or `None` for one that does not exist.
    pub fn parse(raw: &str) -> Option<StatisticsTab> {
        match raw {
            "landscape" => Some(StatisticsTab::Landscape),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            StatisticsTab::Landscape => "landscape",
        }
    }
}

/// The owner a system's activity is counted under when it has none, or when
/// its owner is a person (a Backstage `user:` ref): no name of a person is
/// ever shown.
pub const NO_OWNER: &str = "no owner";

/// The team a system belongs to: its Backstage owner, unless that is a user.
pub fn owner_of(contexts: &BTreeMap<String, SystemContext>, system: &str) -> String {
    contexts
        .get(system)
        .and_then(|c| c.owner.as_ref())
        .filter(|o| !o.kind.eq_ignore_ascii_case("user"))
        .map(|o| o.name.clone())
        .unwrap_or_else(|| NO_OWNER.to_string())
}

/// Upper bound of audit entries read per month of the range. A month that
/// reaches it is reported in `capped_months`, so an undercount is never
/// silent.
pub const AUDIT_MONTH_CAP: u32 = 100_000;

/// One audit entry as the statistics see it: no actor, no details.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScannedEntry {
    pub day: NaiveDate,
    pub action: String,
    /// The system the entry is about: the Producer of a provide, the
    /// Consumer of a require, the system of a catalog push.
    pub system: Option<String>,
    pub stream: Option<String>,
}

/// The audit entries of a range, read month by month.
#[derive(Debug, Clone, Default)]
pub struct AuditScan {
    pub entries: Vec<ScannedEntry>,
    /// `YYYY-MM` of every month that reached [`AUDIT_MONTH_CAP`].
    pub capped_months: Vec<String>,
}

/// The Consumer of a `REQUIRE_SPEC` entry, read from its details, which
/// always start `Consumer '<name>' required` (single and bundle requires).
fn require_consumer(details: &str) -> Option<String> {
    details
        .strip_prefix("Consumer '")
        .and_then(|rest| rest.split_once("' required"))
        .map(|(name, _)| name.to_string())
}

/// Reads the audit entries of `range`, one month at a time.
pub async fn scan_audit(
    repo: &impl SpecRepository,
    range: StatisticsRange,
) -> Result<AuditScan, AppError> {
    let mut scan = AuditScan::default();
    let mut start = range.from;
    while start <= range.to {
        let month_end = start
            .with_day0(0)
            .and_then(|first| first.checked_add_months(Months::new(1)))
            .unwrap_or(NaiveDate::MAX);
        let end = month_end.min(range.to.succ_opt().unwrap_or(NaiveDate::MAX));
        let rows = repo
            .get_audit_logs(AuditLogFilter {
                from_date: Some(start.format("%Y-%m-%d").to_string()),
                // Timestamps compare as text: the bare next day sorts before
                // every entry of that day, so this bound ends with `end - 1`.
                to_date: Some(end.format("%Y-%m-%d").to_string()),
                action_type: None,
                service_wildcard: None,
                version_wildcard: None,
                stream: None,
                branch_id: None,
                limit: AUDIT_MONTH_CAP,
            })
            .await?;
        if rows.len() >= AUDIT_MONTH_CAP as usize {
            scan.capped_months.push(start.format("%Y-%m").to_string());
        }
        for row in rows {
            let Some(day) = row
                .timestamp
                .get(..10)
                .and_then(|d| NaiveDate::parse_from_str(d, "%Y-%m-%d").ok())
            else {
                continue;
            };
            let system = if row.action == "REQUIRE_SPEC" {
                require_consumer(&row.details)
            } else {
                row.service
            };
            scan.entries.push(ScannedEntry {
                day,
                action: row.action,
                system,
                stream: row.stream,
            });
        }
        start = end;
    }
    Ok(scan)
}

/// The audit actions that are calls of a system: what "active" counts.
pub const CALL_ACTIONS: [&str; 3] = ["PROVIDE_SPEC", "REQUIRE_SPEC", "CATALOG_PUSHED"];

/// A name and the systems in it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Group {
    pub name: String,
    pub members: Vec<String>,
}

/// A count per label.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LabelCount {
    pub label: String,
    pub count: usize,
}

/// One flow of the dependency sankey: Consumers of one domain requiring
/// Producers of another, counted as distinct Consumer–Producer pairs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Flow {
    pub from: String,
    pub to: String,
    pub pairs: usize,
}

/// The Landscape tab (san-1022): what exists and how it is grouped.
#[derive(Debug, Clone, Serialize)]
pub struct Landscape {
    pub producers: Vec<String>,
    pub consumers: Vec<String>,
    pub products: Vec<Group>,
    pub domains: Vec<Group>,
    pub tags: Vec<Group>,
    /// Peripheral Resources and the systems that depend on them (dev).
    pub peripheral_resources: Vec<Group>,
    pub external_producers: Vec<String>,
    /// Systems with a provide, require or catalog push in the range.
    pub active_in_range: Vec<String>,
    /// Systems with an open trunk pin or a trunk member version.
    pub on_main: Vec<String>,
    /// Version lines (Producer × API type) per API type.
    pub api_types: Vec<LabelCount>,
    pub flows: Vec<Flow>,
    pub capped_months: Vec<String>,
}

/// The domain label of a system without one.
pub const NO_DOMAIN: &str = "no domain";

fn groups(map: BTreeMap<String, BTreeSet<String>>) -> Vec<Group> {
    map.into_iter()
        .map(|(name, members)| Group {
            name,
            members: members.into_iter().collect(),
        })
        .collect()
}

/// The Landscape tab for `range`.
pub async fn landscape(
    repo: &impl SpecRepository,
    range: StatisticsRange,
) -> Result<Landscape, AppError> {
    let producers = repo.list_producers_detailed().await?;
    let consumers = repo.list_consumers().await?;
    let contexts = repo.list_system_contexts().await?;

    let mut domains: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut domain_of: BTreeMap<String, String> = BTreeMap::new();
    let mut lines: BTreeMap<&'static str, usize> = BTreeMap::new();
    let mut external = Vec::new();
    for p in &producers {
        let domain = p
            .domain
            .clone()
            .filter(|d| !d.trim().is_empty())
            .unwrap_or_else(|| NO_DOMAIN.to_string());
        domains
            .entry(domain.clone())
            .or_default()
            .insert(p.name.clone());
        domain_of.insert(p.name.clone(), domain);
        if p.external {
            external.push(p.name.clone());
        }
    }
    let version_lines: BTreeSet<(String, ApiType)> = repo
        .list_all_spec_versions()
        .await?
        .into_iter()
        .map(|(producer, meta, _)| (producer, meta.api_type))
        .collect();
    for (_, api_type) in version_lines {
        let n = lines.entry(api_type.as_str()).or_insert(0);
        *n = n.saturating_add(1);
    }

    let mut tags: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for (system, context) in &contexts {
        for tag in &context.tags {
            tags.entry(tag.clone()).or_default().insert(system.clone());
        }
    }

    let mut periphery: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for edge in repo.list_periphery(PeripheryTrack::Dev, None).await? {
        periphery
            .entry(edge.resource)
            .or_default()
            .insert(edge.system);
    }

    let products = repo
        .list_products()
        .await?
        .into_iter()
        .map(|p| Group {
            name: p.name,
            members: p.members.into_iter().map(|m| m.system).collect(),
        })
        .collect();

    let scan = scan_audit(repo, range).await?;
    let active: BTreeSet<String> = scan
        .entries
        .iter()
        .filter(|e| CALL_ACTIONS.contains(&e.action.as_str()))
        .filter_map(|e| e.system.clone())
        .collect();

    let mut on_main: BTreeSet<String> = BTreeSet::new();
    for pin in repo.list_current_trunk_pins().await? {
        on_main.insert(pin.client);
        on_main.insert(pin.service);
    }
    for member in repo.list_member_versions(MemberTrack::Trunk, None).await? {
        on_main.insert(member.service);
    }

    let report = repo.get_report().await?;
    let pairs: BTreeSet<(String, String)> = report
        .dependency_graph
        .iter()
        .map(|d| (d.client.clone(), d.service.clone()))
        .collect();
    let mut flows: BTreeMap<(String, String), usize> = BTreeMap::new();
    for (client, service) in pairs {
        let from = domain_of
            .get(&client)
            .cloned()
            .unwrap_or_else(|| NO_DOMAIN.to_string());
        let to = domain_of
            .get(&service)
            .cloned()
            .unwrap_or_else(|| NO_DOMAIN.to_string());
        let n = flows.entry((from, to)).or_insert(0);
        *n = n.saturating_add(1);
    }

    Ok(Landscape {
        producers: producers.into_iter().map(|p| p.name).collect(),
        consumers,
        products,
        domains: groups(domains),
        tags: groups(tags),
        peripheral_resources: groups(periphery),
        external_producers: external,
        active_in_range: active.into_iter().collect(),
        on_main: on_main.into_iter().collect(),
        api_types: lines
            .into_iter()
            .map(|(label, count)| LabelCount {
                label: label.to_string(),
                count,
            })
            .collect(),
        flows: flows
            .into_iter()
            .map(|((from, to), pairs)| Flow { from, to, pairs })
            .collect(),
        capped_months: scan.capped_months,
    })
}

/// What every statistics answer carries around its tab's data.
#[derive(Debug, Clone, Serialize)]
pub struct StatisticsAnswer<T: Serialize> {
    pub tab: &'static str,
    pub range: StatisticsRange,
    pub data: T,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    #[test]
    fn the_default_range_is_the_last_90_days_up_to_today() {
        let range = StatisticsRange::parse(None, None, d("2026-11-25")).unwrap();
        assert_eq!(range.from, d("2026-08-28"));
        assert_eq!(range.to, d("2026-11-25"));
        assert_eq!(range.days().len(), 90);
    }

    #[test]
    fn a_named_range_is_taken_as_it_is() {
        let range = StatisticsRange::parse(Some("2026-01-01"), Some("2026-01-31"), d("2026-11-25"))
            .unwrap();
        assert_eq!(range.days().len(), 31);
        let only_to = StatisticsRange::parse(None, Some("2026-01-31"), d("2026-11-25")).unwrap();
        assert_eq!(only_to.from, d("2025-11-03"));
        let empty = StatisticsRange::parse(Some(""), Some(""), d("2026-11-25")).unwrap();
        assert_eq!(empty.to, d("2026-11-25"));
    }

    #[test]
    fn a_backwards_or_broken_range_is_a_bad_request() {
        assert!(matches!(
            StatisticsRange::parse(Some("2026-02-01"), Some("2026-01-01"), d("2026-11-25")),
            Err(AppError::BadRequest(m)) if m.contains("before it starts")
        ));
        assert!(matches!(
            StatisticsRange::parse(Some("yesterday"), None, d("2026-11-25")),
            Err(AppError::BadRequest(m)) if m.contains("from must be a date")
        ));
    }

    #[test]
    fn tabs_round_trip_and_an_unknown_one_is_none() {
        assert_eq!(StatisticsTab::parse("nope"), None);
        assert_eq!(
            StatisticsTab::parse("landscape").map(StatisticsTab::as_str),
            Some("landscape")
        );
    }

    #[test]
    fn the_consumer_of_a_require_comes_from_its_details() {
        assert_eq!(
            require_consumer(
                "Consumer 'shop' required OpenApi GET /a of 'orders' at version 1.0.0 (ga)"
            ),
            Some("shop".to_string())
        );
        assert_eq!(
            require_consumer("Consumer 'shop' required a bundle of 2 endpoints of 'orders'"),
            Some("shop".to_string())
        );
        assert_eq!(require_consumer("something else"), None);
    }

    #[test]
    fn a_person_never_owns_a_system() {
        use crate::domain::models::SystemOwner;
        let mut contexts = BTreeMap::new();
        let context = |kind: &str, name: &str| SystemContext {
            owner: Some(SystemOwner {
                kind: kind.to_string(),
                name: name.to_string(),
            }),
            ..Default::default()
        };
        contexts.insert("a".to_string(), context("group", "payments"));
        contexts.insert("b".to_string(), context("user", "jdoe"));
        assert_eq!(owner_of(&contexts, "a"), "payments");
        assert_eq!(owner_of(&contexts, "b"), NO_OWNER);
        assert_eq!(owner_of(&contexts, "c"), NO_OWNER);
    }
}
