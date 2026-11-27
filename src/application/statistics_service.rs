//! The statistics page (san-1021): counts of what Sanshain holds, for
//! architects, management and maintainers, one tab per topic.
//!
//! Everything is aggregated here, on the server: a `report_viewer` holds
//! `view_statistics` and nothing else, so no answer may carry a person's name
//! or a single audit entry, not even one the page would hide. Activity is
//! counted per system and rolled up to the system's Backstage owner.

use crate::domain::models::{
    Annotation, ApiType, AppError, AuditLogFilter, MemberTrack, PeripheryTrack, SemVer, Stability,
    SystemContext, SystemLink, VersionOrigin,
};
use crate::domain::ports::SpecRepository;
use chrono::{Datelike, Days, Months, NaiveDate, Weekday};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

/// The range the page shows when it names none: the last 90 days, today
/// included.
pub const DEFAULT_RANGE_DAYS: u64 = 90;

/// The longest range a statistics answer covers: two years. The audit log
/// is read month by month, so an unbounded range would be an unbounded read.
pub const MAX_RANGE_DAYS: i64 = 731;

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
        if to.signed_duration_since(from).num_days() >= MAX_RANGE_DAYS {
            return Err(AppError::BadRequest(format!(
                "the range covers at most {MAX_RANGE_DAYS} days"
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
    Versions,
    Branches,
    Access,
    Catalog,
}

impl StatisticsTab {
    /// The tab a route segment names, or `None` for one that does not exist.
    pub fn parse(raw: &str) -> Option<StatisticsTab> {
        match raw {
            "landscape" => Some(StatisticsTab::Landscape),
            "activity" => Some(StatisticsTab::Activity),
            "versions" => Some(StatisticsTab::Versions),
            "branches" => Some(StatisticsTab::Branches),
            "access" => Some(StatisticsTab::Access),
            "catalog" => Some(StatisticsTab::Catalog),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            StatisticsTab::Landscape => "landscape",
            StatisticsTab::Activity => "activity",
            StatisticsTab::Versions => "versions",
            StatisticsTab::Branches => "branches",
            StatisticsTab::Access => "access",
            StatisticsTab::Catalog => "catalog",
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
    /// Who talks to whom (san-1028, the former isolation report): an
    /// AsyncAPI dependency puts both sides on `KAFKA`.
    pub isolation: Vec<Connection>,
    /// Every recorded dependency (san-1028, the former full dependency
    /// report), dev graph.
    pub dependencies: Vec<DependencyRow>,
    pub capped_months: Vec<String>,
}

/// One connection of the isolation view.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct Connection {
    pub source: String,
    pub target: String,
    pub protocol: &'static str,
}

/// One dependency: a Consumer's Pin on one endpoint of a Producer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DependencyRow {
    pub consumer: String,
    pub producer: String,
    pub api_type: &'static str,
    pub version: String,
    pub stability: &'static str,
    pub method: String,
    pub path: String,
}

/// The connections of the isolation view: a Consumer of an OpenAPI or
/// proto Producer talks to it over https; both sides of an AsyncAPI
/// dependency talk to `KAFKA`.
fn isolation(report: &crate::domain::models::DependencyReport) -> Vec<Connection> {
    let mut set: BTreeSet<Connection> = BTreeSet::new();
    for dep in &report.dependency_graph {
        match dep.api_type {
            ApiType::AsyncApi => {
                for side in [&dep.client, &dep.service] {
                    set.insert(Connection {
                        source: side.clone(),
                        target: "KAFKA".to_string(),
                        protocol: "https",
                    });
                }
            }
            ApiType::OpenApi | ApiType::Proto => {
                set.insert(Connection {
                    source: dep.client.clone(),
                    target: dep.service.clone(),
                    protocol: "https",
                });
            }
        }
    }
    set.into_iter().collect()
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
        isolation: isolation(&report),
        dependencies: report
            .dependency_graph
            .iter()
            .map(|d| DependencyRow {
                consumer: d.client.clone(),
                producer: d.service.clone(),
                api_type: d.api_type.as_str(),
                version: d.version.to_string(),
                stability: d.stability.as_str(),
                method: d.method.clone(),
                path: d.path.clone(),
            })
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

/// The setting that holds the first day of the week (san-1035).
pub const WEEK_START_SETTING: &str = "statistics_week_start";

/// The weekdays by their setting name, Monday first.
const WEEKDAYS: [(&str, Weekday); 7] = [
    ("monday", Weekday::Mon),
    ("tuesday", Weekday::Tue),
    ("wednesday", Weekday::Wed),
    ("thursday", Weekday::Thu),
    ("friday", Weekday::Fri),
    ("saturday", Weekday::Sat),
    ("sunday", Weekday::Sun),
];

/// The setting name of `day`.
pub fn weekday_name(day: Weekday) -> &'static str {
    WEEKDAYS
        .iter()
        .find(|(_, w)| *w == day)
        .map_or("monday", |(name, _)| name)
}

/// The weekday a setting name stands for, `None` for anything else.
pub fn parse_weekday(name: &str) -> Option<Weekday> {
    let name = name.trim().to_ascii_lowercase();
    WEEKDAYS.iter().find(|(n, _)| *n == name).map(|(_, w)| *w)
}

/// The first day of the week the statistics count from: Monday until an
/// admin sets another. An unreadable stored value reads as Monday.
pub async fn get_week_start(repo: &impl SpecRepository) -> Result<Weekday, AppError> {
    Ok(repo
        .get_setting(WEEK_START_SETTING)
        .await?
        .as_deref()
        .and_then(parse_weekday)
        .unwrap_or(Weekday::Mon))
}

/// Stores the first day of the week, by its English name (`monday` …
/// `sunday`, any case).
pub async fn set_week_start(repo: &impl SpecRepository, name: &str) -> Result<Weekday, AppError> {
    let day = parse_weekday(name).ok_or_else(|| {
        AppError::BadRequest(format!(
            "week_start must be one of monday, tuesday, wednesday, thursday, friday, saturday, sunday; got '{name}'"
        ))
    })?;
    repo.set_setting(WEEK_START_SETTING, weekday_name(day))
        .await?;
    Ok(day)
}

/// The periods of a range: its days, or the first days of its weeks when the
/// range is long. Weeks start on the configured day.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Periods {
    /// `day` or `week`.
    pub bucket: &'static str,
    /// The first day of the week, `monday` … `sunday`: the week bands of the
    /// daily charts start there.
    pub week_start: &'static str,
    /// The first day of each period, `YYYY-MM-DD`.
    pub starts: Vec<String>,
    #[serde(skip)]
    first_days: Vec<NaiveDate>,
    #[serde(skip)]
    first_weekday: Weekday,
}

impl Periods {
    pub fn of(range: StatisticsRange, first_weekday: Weekday) -> Periods {
        let days = range.days();
        let weekly = days.len() > DAILY_UP_TO_DAYS;
        let mut first_days: Vec<NaiveDate> = Vec::new();
        for day in days {
            let start = if weekly {
                start_of_week(day, first_weekday)
            } else {
                day
            };
            if first_days.last() != Some(&start) {
                first_days.push(start);
            }
        }
        Periods {
            bucket: if weekly { "week" } else { "day" },
            week_start: weekday_name(first_weekday),
            starts: first_days
                .iter()
                .map(|d| d.format("%Y-%m-%d").to_string())
                .collect(),
            first_days,
            first_weekday,
        }
    }

    /// The index of the period `day` falls into.
    pub fn index(&self, day: NaiveDate) -> Option<usize> {
        let start = if self.bucket == "week" {
            start_of_week(day, self.first_weekday)
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

/// The first day of `day`'s week, for weeks starting on `first`.
fn start_of_week(day: NaiveDate, first: Weekday) -> NaiveDate {
    day.checked_sub_days(Days::new(u64::from(day.weekday().days_since(first))))
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
    let periods = Periods::of(range, get_week_start(repo).await?);
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
        let counted = entry.action == "VERSION_REJECTED"
            || labels.iter().any(|(action, _)| *action == entry.action);
        if !counted {
            continue;
        }
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
    Ok(Activity {
        periods,
        calls,
        refusals,
        owners: owners.into_values().collect(),
        top_systems: top,
        capped_months: scan.capped_months,
    })
}

/// The versions of one Producer, as they stand now.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProducerVersions {
    pub producer: String,
    pub ga: usize,
    pub snapshot: usize,
}

/// A Consumer's Pin that is Outdated or Snapshot-pinned (`CONTEXT.md`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PinState {
    pub consumer: String,
    pub producer: String,
    pub api_type: &'static str,
    pub pin: String,
    /// The latest GA of the line, when it has one.
    pub latest_ga: Option<String>,
    /// `outdated` or `snapshot-pinned`.
    pub state: &'static str,
}

/// An endpoint that is deprecated and still required.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DeprecatedRequired {
    pub consumer: String,
    pub producer: String,
    pub version: String,
    pub method: String,
    pub path: String,
}

/// The Versions and releases tab (san-1024).
#[derive(Debug, Clone, Serialize)]
pub struct Versions {
    pub periods: Periods,
    /// Versions created per period, by their stability now.
    pub created: Vec<Series>,
    /// Promotions of a snapshot to GA per period.
    pub promotions: Vec<usize>,
    pub per_producer: Vec<ProducerVersions>,
    /// Producers whose API family was retired in the range, with how often.
    pub retired: Vec<LabelCount>,
    pub pins: Vec<PinState>,
    pub deprecated_required: Vec<DeprecatedRequired>,
    pub capped_months: Vec<String>,
}

/// The Versions and releases tab for `range`.
pub async fn versions(
    repo: &impl SpecRepository,
    range: StatisticsRange,
) -> Result<Versions, AppError> {
    let periods = Periods::of(range, get_week_start(repo).await?);
    let mut ga_created = vec![0usize; periods.len()];
    let mut snapshot_created = vec![0usize; periods.len()];
    let mut per_producer: BTreeMap<String, ProducerVersions> = BTreeMap::new();
    let mut latest_ga: BTreeMap<(String, ApiType), SemVer> = BTreeMap::new();
    for (producer, meta, _) in repo.list_all_spec_versions().await? {
        let line = per_producer
            .entry(producer.clone())
            .or_insert_with(|| ProducerVersions {
                producer: producer.clone(),
                ga: 0,
                snapshot: 0,
            });
        let is_ga = meta.stability == Stability::Ga;
        if is_ga {
            bump(&mut line.ga);
            let latest = latest_ga
                .entry((producer, meta.api_type))
                .or_insert(meta.version);
            if meta.version > *latest {
                *latest = meta.version;
            }
        } else {
            bump(&mut line.snapshot);
        }
        let created = meta
            .created_at
            .get(..10)
            .and_then(|d| NaiveDate::parse_from_str(d, "%Y-%m-%d").ok());
        if let Some(i) = created.and_then(|d| periods.index(d)) {
            let counts = if is_ga {
                &mut ga_created
            } else {
                &mut snapshot_created
            };
            if let Some(n) = counts.get_mut(i) {
                bump(n);
            }
        }
    }

    let scan = scan_audit(repo, range).await?;
    let mut promotions = vec![0usize; periods.len()];
    let mut retired: BTreeMap<String, usize> = BTreeMap::new();
    for entry in &scan.entries {
        match entry.action.as_str() {
            "VERSION_PROMOTED" => {
                if let Some(n) = periods.index(entry.day).and_then(|i| promotions.get_mut(i)) {
                    bump(n);
                }
            }
            "RETIRE_PROTOCOL" => {
                if let Some(system) = &entry.system {
                    bump(retired.entry(system.clone()).or_insert(0));
                }
            }
            _ => {}
        }
    }

    let report = repo.get_report().await?;
    let mut pins: BTreeSet<(String, String, ApiType, SemVer, bool)> = BTreeSet::new();
    let mut deprecated_required = Vec::new();
    for dep in &report.dependency_graph {
        pins.insert((
            dep.client.clone(),
            dep.service.clone(),
            dep.api_type,
            dep.version,
            dep.stability == Stability::Snapshot,
        ));
        if dep.deprecated {
            deprecated_required.push(DeprecatedRequired {
                consumer: dep.client.clone(),
                producer: dep.service.clone(),
                version: dep.version.to_string(),
                method: dep.method.clone(),
                path: dep.path.clone(),
            });
        }
    }
    let pins = pins
        .into_iter()
        .filter_map(|(consumer, producer, api_type, version, snapshot)| {
            let latest = latest_ga.get(&(producer.clone(), api_type)).copied();
            let state = if snapshot {
                "snapshot-pinned"
            } else if latest.is_some_and(|l| version < l) {
                "outdated"
            } else {
                return None;
            };
            Some(PinState {
                consumer,
                producer,
                api_type: api_type.as_str(),
                pin: version.to_string(),
                latest_ga: latest.map(|l| l.to_string()),
                state,
            })
        })
        .collect();

    Ok(Versions {
        periods,
        created: vec![
            Series {
                label: "GA".to_string(),
                counts: ga_created,
            },
            Series {
                label: "Snapshot".to_string(),
                counts: snapshot_created,
            },
        ],
        promotions,
        per_producer: per_producer.into_values().collect(),
        retired: retired
            .into_iter()
            .map(|(label, count)| LabelCount { label, count })
            .collect(),
        pins,
        deprecated_required,
        capped_months: scan.capped_months,
    })
}

/// One sanshain-branch: when it was cut and what it holds. Who cut it is a
/// person and is left out.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BranchSummary {
    pub name: String,
    pub cut_at: String,
    pub as_of: String,
    pub source: String,
    pub members: usize,
}

/// The Tags and branches tab (san-1025).
#[derive(Debug, Clone, Serialize)]
pub struct Branches {
    pub branches: Vec<BranchSummary>,
    pub periods: Periods,
    /// Provides and requires per period, per stream: dev, trunk, tag.
    pub builds: Vec<Series>,
    /// Builds per tag (sanshain-branch name) in the range.
    pub per_tag: Vec<LabelCount>,
    pub capped_months: Vec<String>,
}

/// The Tags and branches tab for `range`.
pub async fn branches(
    repo: &impl SpecRepository,
    range: StatisticsRange,
) -> Result<Branches, AppError> {
    let mut list = Vec::new();
    for branch in repo.list_branches().await? {
        let members = repo
            .list_member_versions(MemberTrack::Branch(branch.id), None)
            .await?
            .len();
        list.push(BranchSummary {
            name: branch.name,
            cut_at: branch.created_at,
            as_of: branch.as_of,
            source: branch.source,
            members,
        });
    }
    list.sort_by(|a, b| b.cut_at.cmp(&a.cut_at).then_with(|| a.name.cmp(&b.name)));

    let periods = Periods::of(range, get_week_start(repo).await?);
    let streams = ["dev", "trunk", "tag"];
    let mut builds: Vec<Series> = streams
        .iter()
        .map(|label| Series {
            label: (*label).to_string(),
            counts: vec![0; periods.len()],
        })
        .collect();
    let mut per_tag: BTreeMap<String, usize> = BTreeMap::new();
    let scan = scan_audit(repo, range).await?;
    for entry in &scan.entries {
        if entry.action != "PROVIDE_SPEC" && entry.action != "REQUIRE_SPEC" {
            continue;
        }
        let stream = match entry.stream.as_deref() {
            None | Some("") => 0,
            Some("trunk") => 1,
            Some(tag) => {
                bump(per_tag.entry(tag.to_string()).or_insert(0));
                2
            }
        };
        if let Some(n) = periods
            .index(entry.day)
            .and_then(|i| builds.get_mut(stream).and_then(|s| s.counts.get_mut(i)))
        {
            bump(n);
        }
    }
    Ok(Branches {
        branches: list,
        periods,
        builds,
        per_tag: per_tag
            .into_iter()
            .map(|(label, count)| LabelCount { label, count })
            .collect(),
        capped_months: scan.capped_months,
    })
}

/// The Users and access tab (san-1026): counts only, no names, not even of
/// groups.
#[derive(Debug, Clone, Serialize)]
pub struct Access {
    pub sign_in_mode: &'static str,
    pub users: usize,
    pub pending_approval: usize,
    /// Users per stored role (direct or through a group); `no role` for
    /// users without one.
    pub roles: Vec<LabelCount>,
    /// Groups per source (`native`, `ldap`).
    pub groups: Vec<LabelCount>,
    /// Native group memberships, summed over the groups.
    pub group_memberships: usize,
    /// Unexpired API tokens per scope.
    pub tokens: Vec<LabelCount>,
    pub expired_tokens: usize,
}

/// The label of users without any role.
pub const NO_ROLE: &str = "no role";

/// The Users and access tab, as of `now` (RFC 3339).
pub async fn access(repo: &impl SpecRepository, now: &str) -> Result<Access, AppError> {
    let users = repo.list_users().await?;
    let mut roles: BTreeMap<String, usize> = BTreeMap::new();
    let mut tokens: BTreeMap<&'static str, usize> = BTreeMap::new();
    let mut expired_tokens = 0usize;
    let mut pending = 0usize;
    for user in &users {
        if !user.approved {
            bump(&mut pending);
        }
        let held: BTreeSet<String> = repo
            .effective_stored_roles(user.id)
            .await?
            .into_iter()
            .collect();
        if held.is_empty() {
            bump(roles.entry(NO_ROLE.to_string()).or_insert(0));
        }
        for role in held {
            bump(roles.entry(role).or_insert(0));
        }
        for token in repo.list_api_tokens(user.id).await? {
            if token.expires_at.as_str() <= now {
                bump(&mut expired_tokens);
            } else {
                bump(tokens.entry(token.scope.as_str()).or_insert(0));
            }
        }
    }
    let mut groups: BTreeMap<&'static str, usize> = BTreeMap::new();
    let mut memberships = 0usize;
    for group in repo.list_groups().await? {
        bump(groups.entry(group.source.as_str()).or_insert(0));
        memberships = memberships.saturating_add(repo.list_group_member_ids(group.id).await?.len());
    }
    let labels = |m: BTreeMap<String, usize>| {
        m.into_iter()
            .map(|(label, count)| LabelCount { label, count })
            .collect::<Vec<_>>()
    };
    let static_labels = |m: BTreeMap<&'static str, usize>| {
        m.into_iter()
            .map(|(label, count)| LabelCount {
                label: label.to_string(),
                count,
            })
            .collect::<Vec<_>>()
    };
    Ok(Access {
        sign_in_mode: crate::application::auth_service::get_auth_mode(repo)
            .await?
            .as_str(),
        users: users.len(),
        pending_approval: pending,
        roles: labels(roles),
        groups: static_labels(groups),
        group_memberships: memberships,
        tokens: static_labels(tokens),
        expired_tokens,
    })
}

/// One version line a system provides, on its Catalog sheet.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ProvidedLine {
    pub api_type: &'static str,
    pub versions: usize,
    pub latest: String,
    pub latest_ga: Option<String>,
    /// The Origin of the latest version: build, change, tickets, note.
    pub origin: Option<VersionOrigin>,
}

/// One dependency on a Catalog sheet: the other system, the API type and the
/// Pin, with how many endpoints it covers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SheetDependency {
    pub system: String,
    pub api_type: &'static str,
    pub version: String,
    pub endpoints: usize,
}

/// The Catalog sheet of one system (san-1027): both roles on one page. The
/// last provider of a version is a person and is left out.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SystemSheet {
    pub name: String,
    pub owner: String,
    pub domain: String,
    pub products: Vec<String>,
    pub tags: Vec<String>,
    pub description: Option<String>,
    pub source_location: Option<String>,
    pub links: Vec<SystemLink>,
    pub annotations: Vec<Annotation>,
    pub external: bool,
    pub provides: Vec<ProvidedLine>,
    pub consumes: Vec<SheetDependency>,
    pub consumed_by: Vec<SheetDependency>,
}

/// The Catalog tab: a sheet per system, sorted by name.
#[derive(Debug, Clone, Serialize)]
pub struct Catalog {
    pub systems: Vec<SystemSheet>,
}

type DependencyKey = (String, String, ApiType, SemVer);

/// A version line while it is summed up: versions, latest, latest GA, and the
/// id of the latest.
type LineTally = (usize, SemVer, Option<SemVer>, i64);

fn sheet_dependencies(
    pairs: &BTreeMap<DependencyKey, usize>,
    system: &str,
    consumer_side: bool,
) -> Vec<SheetDependency> {
    pairs
        .iter()
        .filter(|((client, service, _, _), _)| {
            if consumer_side {
                client == system
            } else {
                service == system
            }
        })
        .map(
            |((client, service, api_type, version), endpoints)| SheetDependency {
                system: if consumer_side {
                    service.clone()
                } else {
                    client.clone()
                },
                api_type: api_type.as_str(),
                version: version.to_string(),
                endpoints: *endpoints,
            },
        )
        .collect()
}

/// The Catalog tab: every Producer and Consumer with its sheet.
pub async fn catalog(repo: &impl SpecRepository) -> Result<Catalog, AppError> {
    let producers = repo.list_producers_detailed().await?;
    let consumers = repo.list_consumers().await?;
    let contexts = repo.list_system_contexts().await?;
    let annotations = repo.list_system_annotations().await?;
    let origins = super::git_branch_service::list_read_version_origins(repo).await?;
    let mut products_of: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for product in repo.list_products().await? {
        for member in product.members {
            products_of
                .entry(member.system)
                .or_default()
                .push(product.name.clone());
        }
    }

    // (producer, api type) -> (versions, latest, latest GA, id of latest)
    let mut lines: BTreeMap<(String, ApiType), LineTally> = BTreeMap::new();
    for (producer, meta, _) in repo.list_all_spec_versions().await? {
        let line =
            lines
                .entry((producer, meta.api_type))
                .or_insert((0, meta.version, None, meta.id));
        line.0 = line.0.saturating_add(1);
        if meta.version >= line.1 {
            line.1 = meta.version;
            line.3 = meta.id;
        }
        if meta.stability == Stability::Ga && line.2.is_none_or(|g| meta.version > g) {
            line.2 = Some(meta.version);
        }
    }

    let report = repo.get_report().await?;
    let mut pairs: BTreeMap<DependencyKey, usize> = BTreeMap::new();
    for dep in &report.dependency_graph {
        bump(
            pairs
                .entry((
                    dep.client.clone(),
                    dep.service.clone(),
                    dep.api_type,
                    dep.version,
                ))
                .or_insert(0),
        );
    }

    let mut domain_of: BTreeMap<String, (String, bool)> = BTreeMap::new();
    for p in &producers {
        let domain = p
            .domain
            .clone()
            .filter(|d| !d.trim().is_empty())
            .unwrap_or_else(|| NO_DOMAIN.to_string());
        domain_of.insert(p.name.clone(), (domain, p.external));
    }
    let names: BTreeSet<String> = producers
        .iter()
        .map(|p| p.name.clone())
        .chain(consumers)
        .collect();
    let systems = names
        .into_iter()
        .map(|name| {
            let context = contexts.get(&name).cloned().unwrap_or_default();
            let (domain, external) = domain_of
                .get(&name)
                .cloned()
                .unwrap_or_else(|| (NO_DOMAIN.to_string(), false));
            let provides = lines
                .iter()
                .filter(|((producer, _), _)| *producer == name)
                .map(
                    |((_, api_type), (versions, latest, latest_ga, id))| ProvidedLine {
                        api_type: api_type.as_str(),
                        versions: *versions,
                        latest: latest.to_string(),
                        latest_ga: latest_ga.map(|g| g.to_string()),
                        origin: origins.get(id).cloned(),
                    },
                )
                .collect();
            SystemSheet {
                owner: owner_of(&contexts, &name),
                domain,
                products: products_of.get(&name).cloned().unwrap_or_default(),
                tags: context.tags,
                description: context.description,
                source_location: context.source_location,
                links: context.links,
                annotations: annotations.get(&name).cloned().unwrap_or_default(),
                external,
                provides,
                consumes: sheet_dependencies(&pairs, &name, true),
                consumed_by: sheet_dependencies(&pairs, &name, false),
                name,
            }
        })
        .collect();
    Ok(Catalog { systems })
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
            StatisticsRange::parse(Some("0002-01-01"), Some("2026-01-01"), d("2026-11-25")),
            Err(AppError::BadRequest(m)) if m.contains("at most 731 days")
        ));
        assert!(
            StatisticsRange::parse(Some("2025-01-01"), Some("2026-12-31"), d("2026-11-25")).is_ok()
        );
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
            Weekday::Mon,
        );
        assert_eq!(short.bucket, "day");
        assert_eq!(short.len(), 30);
        assert_eq!(short.index(d("2026-11-03")), Some(2));
        assert_eq!(short.index(d("2026-12-01")), None);
        let long = Periods::of(
            StatisticsRange::parse(Some("2026-01-01"), Some("2026-12-31"), d("2026-12-31"))
                .unwrap(),
            Weekday::Mon,
        );
        assert_eq!(long.bucket, "week");
        // 2026-01-01 is a Thursday: its week starts on Monday 2025-12-29.
        assert_eq!(long.starts.first().map(String::as_str), Some("2025-12-29"));
        assert_eq!(long.index(d("2026-01-04")), Some(0));
        assert_eq!(long.index(d("2026-01-05")), Some(1));
        assert!(!long.is_empty());
        assert_eq!(long.week_start, "monday");
    }

    #[test]
    fn weeks_start_on_the_configured_day() {
        let range = StatisticsRange::parse(Some("2026-01-01"), Some("2026-12-31"), d("2026-12-31"))
            .unwrap();
        let sunday = Periods::of(range, Weekday::Sun);
        assert_eq!(sunday.week_start, "sunday");
        // 2026-01-01 is a Thursday: its week starts on Sunday 2025-12-28.
        assert_eq!(
            sunday.starts.first().map(String::as_str),
            Some("2025-12-28")
        );
        assert_eq!(sunday.index(d("2026-01-03")), Some(0));
        assert_eq!(sunday.index(d("2026-01-04")), Some(1));
        let saturday = Periods::of(range, Weekday::Sat);
        assert_eq!(
            saturday.starts.first().map(String::as_str),
            Some("2025-12-27")
        );
    }

    #[test]
    fn weekday_names_round_trip_and_others_are_refused() {
        for (name, day) in WEEKDAYS {
            assert_eq!(parse_weekday(name), Some(day));
            assert_eq!(weekday_name(day), name);
        }
        assert_eq!(parse_weekday(" Sunday "), Some(Weekday::Sun));
        assert_eq!(parse_weekday("sun"), None);
        assert_eq!(parse_weekday(""), None);
    }

    #[tokio::test]
    async fn the_week_start_is_monday_until_set_and_refuses_a_non_day() {
        let repo = crate::application::mock_repo::MockRepo::new();
        assert_eq!(get_week_start(&repo).await.unwrap(), Weekday::Mon);
        assert_eq!(set_week_start(&repo, "Sunday").await.unwrap(), Weekday::Sun);
        assert_eq!(get_week_start(&repo).await.unwrap(), Weekday::Sun);
        assert_eq!(
            repo.get_setting(WEEK_START_SETTING)
                .await
                .unwrap()
                .as_deref(),
            Some("sunday")
        );
        let refused = set_week_start(&repo, "funday").await.unwrap_err();
        assert!(matches!(refused, AppError::BadRequest(m) if m.contains("funday")));
        assert_eq!(get_week_start(&repo).await.unwrap(), Weekday::Sun);
        repo.set_setting(WEEK_START_SETTING, "garbage")
            .await
            .unwrap();
        assert_eq!(get_week_start(&repo).await.unwrap(), Weekday::Mon);
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
