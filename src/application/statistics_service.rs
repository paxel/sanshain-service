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
    Activity,
}

impl StatisticsTab {
    /// The tab a route segment names, or `None` for one that does not exist.
    pub fn parse(raw: &str) -> Option<StatisticsTab> {
        match raw {
            "landscape" => Some(StatisticsTab::Landscape),
            "activity" => Some(StatisticsTab::Activity),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            StatisticsTab::Landscape => "landscape",
            StatisticsTab::Activity => "activity",
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

/// A range longer than this many days is counted per week, not per day.
pub const DAILY_UP_TO_DAYS: usize = 92;

/// The periods of a range: its days, or the Mondays of its weeks when the
/// range is long.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Periods {
    /// `day` or `week`.
    pub bucket: &'static str,
    /// The first day of each period, `YYYY-MM-DD`.
    pub starts: Vec<String>,
    #[serde(skip)]
    first_days: Vec<NaiveDate>,
}

impl Periods {
    pub fn of(range: StatisticsRange) -> Periods {
        let days = range.days();
        let weekly = days.len() > DAILY_UP_TO_DAYS;
        let mut first_days: Vec<NaiveDate> = Vec::new();
        for day in days {
            let start = if weekly { week_start(day) } else { day };
            if first_days.last() != Some(&start) {
                first_days.push(start);
            }
        }
        Periods {
            bucket: if weekly { "week" } else { "day" },
            starts: first_days
                .iter()
                .map(|d| d.format("%Y-%m-%d").to_string())
                .collect(),
            first_days,
        }
    }

    /// The index of the period `day` falls into.
    pub fn index(&self, day: NaiveDate) -> Option<usize> {
        let start = if self.bucket == "week" {
            week_start(day)
        } else {
            day
        };
        self.first_days.binary_search(&start).ok()
    }

    pub fn len(&self) -> usize {
        self.first_days.len()
    }

    pub fn is_empty(&self) -> bool {
        self.first_days.is_empty()
    }
}

/// The Monday of `day`'s week.
fn week_start(day: NaiveDate) -> NaiveDate {
    day.checked_sub_days(Days::new(u64::from(day.weekday().num_days_from_monday())))
        .unwrap_or(day)
}

/// A count per period.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Series {
    pub label: String,
    pub counts: Vec<usize>,
}

/// What one owner's systems did in the range.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Default)]
pub struct OwnerActivity {
    pub owner: String,
    pub provides: usize,
    pub requires: usize,
    pub catalog_pushes: usize,
    pub refusals: usize,
}

/// The calls of one system in the range.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SystemCalls {
    pub system: String,
    pub owner: String,
    pub calls: usize,
}

/// The Activity tab (san-1023): what the teams did, counted, never named.
#[derive(Debug, Clone, Serialize)]
pub struct Activity {
    pub periods: Periods,
    /// Provides, requires and catalog pushes per period.
    pub calls: Vec<Series>,
    /// Refused provides per period.
    pub refusals: Vec<usize>,
    pub owners: Vec<OwnerActivity>,
    /// The systems with the most calls, at most [`TOP_SYSTEMS`].
    pub top_systems: Vec<SystemCalls>,
    pub capped_months: Vec<String>,
}

/// How many systems the top list shows.
pub const TOP_SYSTEMS: usize = 15;

fn bump(n: &mut usize) {
    *n = n.saturating_add(1);
}

/// The Activity tab for `range`.
pub async fn activity(
    repo: &impl SpecRepository,
    range: StatisticsRange,
) -> Result<Activity, AppError> {
    let contexts = repo.list_system_contexts().await?;
    let scan = scan_audit(repo, range).await?;
    let periods = Periods::of(range);
    let labels = [
        ("PROVIDE_SPEC", "Provides"),
        ("REQUIRE_SPEC", "Requires"),
        ("CATALOG_PUSHED", "Catalog pushes"),
    ];
    let mut calls: Vec<Series> = labels
        .iter()
        .map(|(_, label)| Series {
            label: (*label).to_string(),
            counts: vec![0; periods.len()],
        })
        .collect();
    let mut refusals = vec![0usize; periods.len()];
    let mut owners: BTreeMap<String, OwnerActivity> = BTreeMap::new();
    let mut systems: BTreeMap<String, usize> = BTreeMap::new();
    for entry in &scan.entries {
        let Some(period) = periods.index(entry.day) else {
            continue;
        };
        let owner = entry
            .system
            .as_deref()
            .map_or_else(|| NO_OWNER.to_string(), |s| owner_of(&contexts, s));
        let line = owners
            .entry(owner.clone())
            .or_insert_with(|| OwnerActivity {
                owner,
                ..Default::default()
            });
        if entry.action == "VERSION_REJECTED" {
            if let Some(n) = refusals.get_mut(period) {
                bump(n);
            }
            bump(&mut line.refusals);
            continue;
        }
        let Some(series) = labels
            .iter()
            .position(|(action, _)| *action == entry.action)
            .and_then(|i| calls.get_mut(i))
        else {
            continue;
        };
        if let Some(n) = series.counts.get_mut(period) {
            bump(n);
        }
        match entry.action.as_str() {
            "PROVIDE_SPEC" => bump(&mut line.provides),
            "REQUIRE_SPEC" => bump(&mut line.requires),
            _ => bump(&mut line.catalog_pushes),
        }
        if let Some(system) = &entry.system {
            bump(systems.entry(system.clone()).or_insert(0));
        }
    }
    let mut top: Vec<SystemCalls> = systems
        .into_iter()
        .map(|(system, calls)| SystemCalls {
            owner: owner_of(&contexts, &system),
            system,
            calls,
        })
        .collect();
    top.sort_by(|a, b| b.calls.cmp(&a.calls).then_with(|| a.system.cmp(&b.system)));
    top.truncate(TOP_SYSTEMS);
    // Owners that only exist because a refusal or call named nothing stay;
    // owners with nothing at all never got an entry.
    Ok(Activity {
        periods,
        calls,
        refusals,
        owners: owners.into_values().collect(),
        top_systems: top,
        capped_months: scan.capped_months,
    })
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
    fn a_short_range_counts_days_and_a_long_one_weeks() {
        let short = Periods::of(
            StatisticsRange::parse(Some("2026-11-01"), Some("2026-11-30"), d("2026-11-30"))
                .unwrap(),
        );
        assert_eq!(short.bucket, "day");
        assert_eq!(short.len(), 30);
        assert_eq!(short.index(d("2026-11-03")), Some(2));
        assert_eq!(short.index(d("2026-12-01")), None);
        let long = Periods::of(
            StatisticsRange::parse(Some("2026-01-01"), Some("2026-12-31"), d("2026-12-31"))
                .unwrap(),
        );
        assert_eq!(long.bucket, "week");
        // 2026-01-01 is a Thursday: its week starts on Monday 2025-12-29.
        assert_eq!(long.starts.first().map(String::as_str), Some("2025-12-29"));
        assert_eq!(long.index(d("2026-01-04")), Some(0));
        assert_eq!(long.index(d("2026-01-05")), Some(1));
        assert!(!long.is_empty());
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
