//! Product snapshots (san-1033): one release of a Product per umbrella chart
//! version, refreshed by Helm chart pushes. A snapshot keeps each member
//! with the chart version the umbrella asks for, a sales name, and the graph
//! scope its API versions are read from; the release sheet describes it as a
//! whole, and two snapshots can be compared.

use std::cmp::Ordering;
use std::collections::BTreeMap;

use serde::Serialize;
use tracing::instrument;

use super::move_service::{EndpointRef, move_proposal, same_snippet};
use super::product_service::{ProductImportReport, import_product_chart, require_product};
use super::report_service::{ReportScope, generate_scoped_report, get_graph_scope};
use super::spec_service::content_hash;
use crate::domain::models::{
    ApiType, AppError, ProductSnapshot, ProductSnapshotMember, RequireSet, SemVer, Stability,
};
use crate::domain::ports::SpecRepository;

/// A sales name is at most this many characters.
pub const MAX_SALES_NAME_CHARS: usize = 100;

/// A chart version (SemVer 2): `MAJOR.MINOR.PATCH`, an optional pre-release
/// and build metadata, which does not order. A leading `v` is accepted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChartVersion {
    core: (u64, u64, u64),
    pre: Vec<String>,
}

impl ChartVersion {
    pub fn parse(raw: &str) -> Result<ChartVersion, String> {
        let text = raw.trim();
        let text = text.strip_prefix('v').unwrap_or(text);
        let text = text.split('+').next().unwrap_or(text);
        let (core, pre) = match text.split_once('-') {
            Some((core, pre)) => (core, Some(pre)),
            None => (text, None),
        };
        let parts: Vec<&str> = core.split('.').collect();
        let number = |i: usize| -> Result<u64, String> {
            parts
                .get(i)
                .and_then(|p| p.parse::<u64>().ok())
                .ok_or_else(|| format!("chart version '{raw}' is not MAJOR.MINOR.PATCH"))
        };
        if parts.len() != 3 {
            return Err(format!("chart version '{raw}' is not MAJOR.MINOR.PATCH"));
        }
        let pre: Vec<String> = match pre {
            Some(pre) if pre.is_empty() || pre.split('.').any(str::is_empty) => {
                return Err(format!(
                    "chart version '{raw}' has an empty pre-release part"
                ));
            }
            Some(pre) => pre.split('.').map(str::to_string).collect(),
            None => Vec::new(),
        };
        Ok(ChartVersion {
            core: (number(0)?, number(1)?, number(2)?),
            pre,
        })
    }
}

impl Ord for ChartVersion {
    fn cmp(&self, other: &Self) -> Ordering {
        self.core
            .cmp(&other.core)
            .then_with(|| match (self.pre.is_empty(), other.pre.is_empty()) {
                (true, true) => Ordering::Equal,
                (true, false) => Ordering::Greater,
                (false, true) => Ordering::Less,
                (false, false) => {
                    for (a, b) in self.pre.iter().zip(other.pre.iter()) {
                        let order = match (a.parse::<u64>(), b.parse::<u64>()) {
                            (Ok(x), Ok(y)) => x.cmp(&y),
                            (Ok(_), Err(_)) => Ordering::Less,
                            (Err(_), Ok(_)) => Ordering::Greater,
                            (Err(_), Err(_)) => a.cmp(b),
                        };
                        if order != Ordering::Equal {
                            return order;
                        }
                    }
                    self.pre.len().cmp(&other.pre.len())
                }
            })
    }
}

impl PartialOrd for ChartVersion {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// What a chart push did.
#[derive(Serialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SnapshotOutcome {
    Created,
    Replaced,
    Unchanged,
}

/// The answer to a chart push or paste.
#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct ChartPushReport {
    /// `None` for a pasted chart without a version: members only, no snapshot.
    pub chart_version: Option<String>,
    pub outcome: Option<SnapshotOutcome>,
    /// The member import, when this chart decides the members (the highest
    /// chart version, or a chart without a version).
    pub members: Option<ProductImportReport>,
}

/// A scope a snapshot may name: `main` or a sanshain-branch, at an instant.
/// One without an instant is frozen at `now`. Dev is no release; an unknown
/// branch is `404`.
async fn frozen_scope(
    repo: &impl SpecRepository,
    raw: Option<&str>,
    now: &str,
) -> Result<String, AppError> {
    let raw = raw
        .map(str::trim)
        .filter(|r| !r.is_empty())
        .unwrap_or("main");
    match ReportScope::parse(Some(raw))? {
        ReportScope::Dev => Err(AppError::BadRequest(
            "a Product snapshot is read from main or a sanshain-branch, not from dev".to_string(),
        )),
        ReportScope::Main { at } => Ok(format!("main@{}", at.as_deref().unwrap_or(now))),
        ReportScope::Branch { name, at } => {
            repo.find_branch(&name)
                .await?
                .ok_or_else(|| AppError::NotFound(format!("no sanshain-branch '{name}'")))?;
            Ok(format!("{name}@{}", at.as_deref().unwrap_or(now)))
        }
    }
}

fn valid_sales_name(raw: Option<&str>) -> Result<Option<String>, AppError> {
    let Some(name) = raw.map(str::trim).filter(|n| !n.is_empty()) else {
        return Ok(None);
    };
    if name.chars().count() > MAX_SALES_NAME_CHARS || name.chars().any(char::is_control) {
        return Err(AppError::BadRequest(format!(
            "a sales name has at most {MAX_SALES_NAME_CHARS} characters and no control characters"
        )));
    }
    Ok(Some(name.to_string()))
}

/// Push (or paste) an umbrella `Chart.yaml` into a Product. With a chart
/// `version` it creates or replaces the snapshot of that version (the same
/// content is a no-op; a sales name or scope set on the page survives); the
/// highest chart version also decides the Product's members. A chart without
/// a version is refused on a push (`require_version`) and, pasted, only sets
/// the members, as before.
#[instrument(skip(repo, chart_yaml))]
pub async fn push_product_chart(
    repo: &impl SpecRepository,
    product: &str,
    chart_yaml: &str,
    scope: Option<&str>,
    actor: Option<&str>,
    require_version: bool,
) -> Result<ChartPushReport, AppError> {
    let found = require_product(repo, product).await?;
    let yaml = chart_yaml.to_string();
    let release = super::run_cpu_bound(move || {
        crate::helm::read_chart(&yaml).map_err(|e| AppError::BadRequest(e.to_string()))
    })
    .await?;
    let Some(version) = release.version.clone() else {
        if require_version {
            return Err(AppError::BadRequest(
                "Chart.yaml has no version — a Product snapshot is keyed by the chart's version"
                    .to_string(),
            ));
        }
        let members = import_product_chart(repo, product, chart_yaml).await?;
        return Ok(ChartPushReport {
            chart_version: None,
            outcome: None,
            members: Some(members),
        });
    };
    let parsed = ChartVersion::parse(&version).map_err(AppError::BadRequest)?;
    let sales_name = valid_sales_name(release.sales_name.as_deref())?;
    let now = super::now_iso();
    let scope = frozen_scope(repo, scope, &now).await?;
    let hash = content_hash(&super::spec_service::normalize_spec_text(chart_yaml));
    let snapshots = repo.list_product_snapshots(found.id).await?;
    let existing = snapshots.iter().find(|s| s.chart_version == version);
    let outcome = match existing {
        Some(old) if old.content_hash == hash => SnapshotOutcome::Unchanged,
        Some(old) => {
            repo.save_product_snapshot(
                found.id,
                &ProductSnapshot {
                    chart_version: version.clone(),
                    content_hash: hash,
                    sales_name: if old.sales_name_override {
                        old.sales_name.clone()
                    } else {
                        sales_name
                    },
                    sales_name_override: old.sales_name_override,
                    scope: if old.scope_override {
                        old.scope.clone()
                    } else {
                        scope
                    },
                    scope_override: old.scope_override,
                    pushed_at: now,
                    pushed_by: actor.map(str::to_string),
                    members: snapshot_members(&release),
                },
            )
            .await?;
            SnapshotOutcome::Replaced
        }
        None => {
            repo.save_product_snapshot(
                found.id,
                &ProductSnapshot {
                    chart_version: version.clone(),
                    content_hash: hash,
                    sales_name,
                    sales_name_override: false,
                    scope,
                    scope_override: false,
                    pushed_at: now,
                    pushed_by: actor.map(str::to_string),
                    members: snapshot_members(&release),
                },
            )
            .await?;
            SnapshotOutcome::Created
        }
    };
    let highest = snapshots
        .iter()
        .filter_map(|s| ChartVersion::parse(&s.chart_version).ok())
        .all(|other| other <= parsed);
    let members = if highest && outcome != SnapshotOutcome::Unchanged {
        Some(import_product_chart(repo, product, chart_yaml).await?)
    } else {
        None
    };
    Ok(ChartPushReport {
        chart_version: Some(version),
        outcome: Some(outcome),
        members,
    })
}

fn snapshot_members(release: &crate::helm::ChartRelease) -> Vec<ProductSnapshotMember> {
    release
        .members
        .iter()
        .map(|m| ProductSnapshotMember {
            system: m.name.clone(),
            chart_version: m.version.clone(),
        })
        .collect()
}

/// A Product's snapshots, the highest chart version first.
pub async fn list_product_snapshots(
    repo: &impl SpecRepository,
    product: &str,
) -> Result<Vec<ProductSnapshot>, AppError> {
    let found = require_product(repo, product).await?;
    let mut snapshots = repo.list_product_snapshots(found.id).await?;
    snapshots.sort_by(|a, b| {
        match (
            ChartVersion::parse(&a.chart_version),
            ChartVersion::parse(&b.chart_version),
        ) {
            (Ok(x), Ok(y)) => y.cmp(&x),
            _ => b.chart_version.cmp(&a.chart_version),
        }
    });
    Ok(snapshots)
}

async fn require_snapshot(
    repo: &impl SpecRepository,
    product: &str,
    version: &str,
) -> Result<(i64, ProductSnapshot), AppError> {
    let found = require_product(repo, product).await?;
    repo.list_product_snapshots(found.id)
        .await?
        .into_iter()
        .find(|s| s.chart_version == version)
        .map(|s| (found.id, s))
        .ok_or_else(|| AppError::NotFound(format!("Product '{product}' has no snapshot {version}")))
}

/// Set a snapshot's sales name or scope on the page; each one set is kept by
/// later pushes of the same chart version. A blank sales name clears it.
pub async fn edit_product_snapshot(
    repo: &impl SpecRepository,
    product: &str,
    version: &str,
    sales_name: Option<Option<&str>>,
    scope: Option<&str>,
) -> Result<ProductSnapshot, AppError> {
    let (product_id, mut snapshot) = require_snapshot(repo, product, version).await?;
    if let Some(name) = sales_name {
        snapshot.sales_name = valid_sales_name(name)?;
        snapshot.sales_name_override = true;
    }
    if let Some(scope) = scope {
        snapshot.scope = frozen_scope(repo, Some(scope), &super::now_iso()).await?;
        snapshot.scope_override = true;
    }
    repo.save_product_snapshot(product_id, &snapshot).await?;
    Ok(snapshot)
}

/// One version a member provides in the snapshot's scope.
#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct ProvidedVersion {
    pub api_type: ApiType,
    pub version: SemVer,
    pub stability: Option<Stability>,
}

/// One Pin of a member in the snapshot's scope.
#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct SheetPin {
    pub producer: String,
    pub api_type: ApiType,
    pub version: SemVer,
    /// The version the snapshot ships of that Producer, when it is a member.
    pub shipped: Option<SemVer>,
    /// The Pin does not match what the snapshot ships.
    pub mismatch: bool,
    /// A free move (san-1031) for a mismatched Pin.
    pub proposal: Option<SemVer>,
}

/// One member row of the release sheet.
#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct SheetMember {
    pub system: String,
    pub chart_version: Option<String>,
    pub provides: Vec<ProvidedVersion>,
    /// The newest require-set version (san-1032) seen by the scope's instant.
    pub require_set: Option<RequireSet>,
    pub pins: Vec<SheetPin>,
}

/// A Product snapshot described as a whole.
#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct ReleaseSheet {
    pub product: String,
    pub chart_version: String,
    pub sales_name: Option<String>,
    pub scope: String,
    pub pushed_at: String,
    pub members: Vec<SheetMember>,
}

/// The release sheet of one snapshot.
#[instrument(skip(repo))]
pub async fn product_release_sheet(
    repo: &impl SpecRepository,
    product: &str,
    version: &str,
) -> Result<ReleaseSheet, AppError> {
    let (_, snapshot) = require_snapshot(repo, product, version).await?;
    let scope = ReportScope::parse(Some(&snapshot.scope))?;
    let instant = snapshot.scope.split_once('@').map(|(_, at)| at.to_string());
    let scoped = get_graph_scope(repo, &scope).await?;
    let mut provides: BTreeMap<String, Vec<ProvidedVersion>> = BTreeMap::new();
    for producer in scoped.producers {
        provides.insert(
            producer.name,
            producer
                .versions
                .into_iter()
                .map(|v| ProvidedVersion {
                    api_type: v.api_type,
                    version: v.version,
                    stability: v.stability,
                })
                .collect(),
        );
    }
    let report = generate_scoped_report(repo, ReportScope::parse(Some(&snapshot.scope))?).await?;
    let mut pins: BTreeMap<String, BTreeMap<(String, &'static str, SemVer), ApiType>> =
        BTreeMap::new();
    for edge in report.dependency_graph {
        pins.entry(edge.client).or_default().insert(
            (edge.service, edge.api_type.as_str(), edge.version),
            edge.api_type,
        );
    }
    let in_snapshot: std::collections::BTreeSet<&str> =
        snapshot.members.iter().map(|m| m.system.as_str()).collect();
    let mut members = Vec::with_capacity(snapshot.members.len());
    for member in &snapshot.members {
        let mut sheet_pins = Vec::new();
        for ((producer, _, pinned), api_type) in
            pins.get(&member.system).cloned().unwrap_or_default()
        {
            let shipped = if in_snapshot.contains(producer.as_str()) {
                provides
                    .get(&producer)
                    .and_then(|lines| lines.iter().find(|l| l.api_type == api_type))
                    .map(|l| l.version)
            } else {
                None
            };
            let mismatch = shipped.is_some_and(|s| s != pinned);
            let proposal = if mismatch {
                move_proposal(repo, &member.system, &producer, api_type, pinned, &[]).await?
            } else {
                None
            };
            sheet_pins.push(SheetPin {
                producer,
                api_type,
                version: pinned,
                shipped,
                mismatch,
                proposal,
            });
        }
        let require_set = repo
            .list_require_sets(&member.system)
            .await?
            .into_iter()
            .rfind(|s| {
                instant
                    .as_deref()
                    .is_none_or(|at| s.first_seen_at.as_str() <= at)
            });
        members.push(SheetMember {
            system: member.system.clone(),
            chart_version: member.chart_version.clone(),
            provides: provides.get(&member.system).cloned().unwrap_or_default(),
            require_set,
            pins: sheet_pins,
        });
    }
    Ok(ReleaseSheet {
        product: product.to_string(),
        chart_version: snapshot.chart_version,
        sales_name: snapshot.sales_name,
        scope: snapshot.scope,
        pushed_at: snapshot.pushed_at,
        members,
    })
}

/// One API line of a member that moved between two snapshots.
#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct MovedLine {
    pub api_type: ApiType,
    pub from: Option<SemVer>,
    pub to: Option<SemVer>,
    pub added: Vec<EndpointRef>,
    pub changed: Vec<EndpointRef>,
    pub removed: Vec<EndpointRef>,
}

/// One member in the comparison of two snapshots.
#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct MemberComparison {
    pub system: String,
    pub from_chart: Option<String>,
    pub to_chart: Option<String>,
    /// In one snapshot only.
    pub only_in: Option<String>,
    pub moved: Vec<MovedLine>,
}

/// Compare two snapshots: per member the API versions that moved, with the
/// endpoints each moved line added, changed or removed.
#[instrument(skip(repo))]
pub async fn compare_product_snapshots(
    repo: &impl SpecRepository,
    product: &str,
    from: &str,
    to: &str,
) -> Result<Vec<MemberComparison>, AppError> {
    let a = product_release_sheet(repo, product, from).await?;
    let b = product_release_sheet(repo, product, to).await?;
    let mut systems: Vec<String> = a.members.iter().map(|m| m.system.clone()).collect();
    for m in &b.members {
        if !systems.contains(&m.system) {
            systems.push(m.system.clone());
        }
    }
    let mut out = Vec::with_capacity(systems.len());
    for system in systems {
        let left = a.members.iter().find(|m| m.system == system);
        let right = b.members.iter().find(|m| m.system == system);
        let only_in = match (left, right) {
            (Some(_), None) => Some(from.to_string()),
            (None, Some(_)) => Some(to.to_string()),
            _ => None,
        };
        let mut types: Vec<ApiType> = Vec::new();
        for p in left
            .map(|m| m.provides.as_slice())
            .unwrap_or_default()
            .iter()
            .chain(right.map(|m| m.provides.as_slice()).unwrap_or_default())
        {
            if !types.contains(&p.api_type) {
                types.push(p.api_type);
            }
        }
        let version_of = |m: Option<&SheetMember>, t: ApiType| {
            m.and_then(|m| m.provides.iter().find(|p| p.api_type == t))
                .map(|p| p.version)
        };
        let mut moved = Vec::new();
        for api_type in types {
            let (old, new) = (version_of(left, api_type), version_of(right, api_type));
            if old == new {
                continue;
            }
            let (added, changed, removed) = line_changes(repo, &system, api_type, old, new).await?;
            moved.push(MovedLine {
                api_type,
                from: old,
                to: new,
                added,
                changed,
                removed,
            });
        }
        out.push(MemberComparison {
            system,
            from_chart: left.and_then(|m| m.chart_version.clone()),
            to_chart: right.and_then(|m| m.chart_version.clone()),
            only_in,
            moved,
        });
    }
    Ok(out)
}

type LineChanges = (Vec<EndpointRef>, Vec<EndpointRef>, Vec<EndpointRef>);

/// The endpoints one line added, changed and removed between two versions.
async fn line_changes(
    repo: &impl SpecRepository,
    producer: &str,
    api_type: ApiType,
    from: Option<SemVer>,
    to: Option<SemVer>,
) -> Result<LineChanges, AppError> {
    let Some(service_id) = repo.find_service(producer).await? else {
        return Ok((Vec::new(), Vec::new(), Vec::new()));
    };
    let endpoints = |version: Option<SemVer>| async move {
        let Some(version) = version else {
            return Ok::<_, AppError>(Vec::new());
        };
        match repo
            .find_spec_version(service_id, api_type, version)
            .await?
        {
            Some(meta) => Ok(repo.get_endpoints_for_version(meta.id).await?),
            None => Ok(Vec::new()),
        }
    };
    let old = endpoints(from).await?;
    let new = endpoints(to).await?;
    let key =
        |e: &crate::domain::models::EndpointRecord| (e.normalized_path.clone(), e.method.clone());
    let named = |e: &crate::domain::models::EndpointRecord| EndpointRef {
        method: e.method.clone(),
        path: e.path.clone(),
    };
    let (mut added, mut changed, mut removed) = (Vec::new(), Vec::new(), Vec::new());
    for e in &new {
        match old.iter().find(|o| key(o) == key(e)) {
            None => added.push(named(e)),
            Some(o) if !same_snippet(api_type, &o.yaml_content, &e.yaml_content) => {
                changed.push(named(e));
            }
            Some(_) => {}
        }
    }
    for o in &old {
        if !new.iter().any(|e| key(e) == key(o)) {
            removed.push(named(o));
        }
    }
    Ok((added, changed, removed))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(raw: &str) -> ChartVersion {
        ChartVersion::parse(raw).unwrap()
    }

    #[test]
    fn chart_versions_order_as_semver_2() {
        assert!(v("1.3.0") > v("1.2.9"));
        assert!(v("1.3.0") > v("1.3.0-rc.1"));
        assert!(v("1.3.0-rc.2") > v("1.3.0-rc.1"));
        assert!(v("1.3.0-rc.10") > v("1.3.0-rc.2"));
        assert!(v("1.3.0-beta") > v("1.3.0-alpha.9"));
        assert!(v("1.3.0-alpha.1") > v("1.3.0-alpha"));
        assert!(v("1.3.0-1") < v("1.3.0-alpha"));
        assert_eq!(v("v1.3.0+build.7"), v("1.3.0"));
        assert!(ChartVersion::parse("1.3").is_err());
        assert!(ChartVersion::parse("1.3.x").is_err());
        assert!(ChartVersion::parse("1.3.0-").is_err());
        assert!(ChartVersion::parse("1.3.0-a..b").is_err());
    }

    #[test]
    fn a_sales_name_is_bounded() {
        assert_eq!(valid_sales_name(Some("  ")).unwrap(), None);
        assert_eq!(
            valid_sales_name(Some(" Bulbao ]|[ - alpha "))
                .unwrap()
                .as_deref(),
            Some("Bulbao ]|[ - alpha")
        );
        assert!(valid_sales_name(Some(&"x".repeat(101))).is_err());
        assert!(valid_sales_name(Some("a\nb")).is_err());
    }
}
