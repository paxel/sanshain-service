//! Version → endpoints, and free moves (san-1031).
//!
//! A Producer's page summarises, version by version, which endpoints each
//! version added, changed, removed or newly deprecated, against the highest
//! GA below it. A Consumer's page lists, per Pin, the newer versions of the
//! line with only the endpoints that Consumer requires, compared with the
//! pinned version; the newest version where every one of them keeps its
//! shape is proposed as a free move, on the page and on the require answer
//! (`X-Sanshain-Upgrade`). A proposal never changes what is served.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use serde::Serialize;

use super::version_rules::{classify_change, ga_baseline};
use crate::domain::models::{
    ApiType, AppError, EndpointRecord, Impact, SemVer, SpecVersionMeta, Stability,
};
use crate::domain::ports::SpecRepository;

/// One endpoint, as a page names it.
#[derive(Serialize, Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct EndpointRef {
    pub method: String,
    pub path: String,
}

/// What one version changed against the highest GA below it.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct VersionEndpointChanges {
    pub api_type: ApiType,
    pub version: SemVer,
    pub stability: Stability,
    /// The GA it is compared with; `None` shows every endpoint as added.
    pub baseline: Option<SemVer>,
    pub added: Vec<EndpointRef>,
    pub changed: Vec<EndpointRef>,
    pub removed: Vec<EndpointRef>,
    /// In both, deprecated here and not in the baseline.
    pub deprecated: Vec<EndpointRef>,
}

/// The endpoint changes of every version of a Producer, line by line
/// (API type, then version ascending). `404` for an unknown Producer.
pub async fn producer_endpoint_changes(
    repo: &impl SpecRepository,
    producername: &str,
) -> Result<Vec<VersionEndpointChanges>, AppError> {
    let Some(service_id) = repo.find_service(producername).await? else {
        return Err(AppError::NotFound(format!(
            "Producer '{producername}' is unknown"
        )));
    };
    let mut line = repo.list_spec_versions(service_id).await?;
    line.sort_by(|a, b| {
        a.api_type
            .as_str()
            .cmp(b.api_type.as_str())
            .then(a.version.cmp(&b.version))
    });
    let mut endpoints: HashMap<i64, Vec<EndpointRecord>> = HashMap::new();
    let mut out = Vec::with_capacity(line.len());
    for meta in &line {
        let same_type: Vec<SpecVersionMeta> = line
            .iter()
            .filter(|v| v.api_type == meta.api_type)
            .cloned()
            .collect();
        let baseline = ga_baseline(&same_type, meta.version);
        let new = cached_endpoints(repo, &mut endpoints, meta.id).await?;
        let base = match &baseline {
            Some(b) => cached_endpoints(repo, &mut endpoints, b.id).await?,
            None => Vec::new(),
        };
        out.push(endpoint_diff(
            meta,
            baseline.map(|b| b.version),
            &base,
            &new,
        ));
    }
    Ok(out)
}

async fn cached_endpoints(
    repo: &impl SpecRepository,
    cache: &mut HashMap<i64, Vec<EndpointRecord>>,
    id: i64,
) -> Result<Vec<EndpointRecord>, AppError> {
    if let Some(found) = cache.get(&id) {
        return Ok(found.clone());
    }
    let found = repo.get_endpoints_for_version(id).await?;
    cache.insert(id, found.clone());
    Ok(found)
}

/// Whether two snippets of one endpoint say the same, apart from the version
/// every snippet carries (`info.version`, proto's `// sanshain-version:`).
pub(crate) fn same_snippet(api_type: ApiType, a: &str, b: &str) -> bool {
    if a == b {
        return true;
    }
    match api_type {
        ApiType::Proto => {
            let strip = |text: &str| -> Vec<String> {
                text.lines()
                    .filter(|l| !l.trim_start().starts_with("// sanshain-version:"))
                    .map(str::to_string)
                    .collect()
            };
            strip(a) == strip(b)
        }
        ApiType::OpenApi | ApiType::AsyncApi => {
            let parse = |text: &str| -> Option<serde_json::Value> {
                let mut value: serde_json::Value = serde_saphyr::from_str(text).ok()?;
                if let Some(info) = value.get_mut("info").and_then(|i| i.as_object_mut()) {
                    info.remove("version");
                }
                Some(value)
            };
            match (parse(a), parse(b)) {
                (Some(x), Some(y)) => x == y,
                _ => false,
            }
        }
    }
}

/// The diff of two endpoint sets, keyed by normalized path and method. A
/// newly deprecated endpoint is listed as deprecated, not also as changed.
fn endpoint_diff(
    meta: &SpecVersionMeta,
    baseline: Option<SemVer>,
    base: &[EndpointRecord],
    new: &[EndpointRecord],
) -> VersionEndpointChanges {
    let key = |e: &EndpointRecord| (e.normalized_path.clone(), e.method.clone());
    let named = |e: &EndpointRecord| EndpointRef {
        method: e.method.clone(),
        path: e.path.clone(),
    };
    let old: BTreeMap<(String, String), &EndpointRecord> =
        base.iter().map(|e| (key(e), e)).collect();
    let current: BTreeMap<(String, String), &EndpointRecord> =
        new.iter().map(|e| (key(e), e)).collect();
    let mut changes = VersionEndpointChanges {
        api_type: meta.api_type,
        version: meta.version,
        stability: meta.stability,
        baseline,
        added: Vec::new(),
        changed: Vec::new(),
        removed: Vec::new(),
        deprecated: Vec::new(),
    };
    for (k, e) in &current {
        match old.get(k) {
            None => changes.added.push(named(e)),
            Some(before) if e.deprecated && !before.deprecated => {
                changes.deprecated.push(named(e));
            }
            Some(before) if !same_snippet(e.api_type, &before.yaml_content, &e.yaml_content) => {
                changes.changed.push(named(e));
            }
            Some(_) => {}
        }
    }
    for (k, e) in &old {
        if !current.contains_key(k) {
            changes.removed.push(named(e));
        }
    }
    changes
}

/// How one required endpoint fares in a newer version.
#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MoveChange {
    /// Same shape: descriptions and examples may differ, generated code not.
    Unchanged,
    Changed,
    Removed,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct EndpointMove {
    pub method: String,
    pub path: String,
    pub change: MoveChange,
    /// Deprecated in that version.
    pub deprecated: bool,
}

/// A newer version of a pinned line, seen through one Consumer's endpoints.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct CandidateVersion {
    pub version: SemVer,
    pub stability: Stability,
    pub endpoints: Vec<EndpointMove>,
    /// Every required endpoint keeps its shape: moving costs nothing.
    pub free: bool,
}

/// The newest free move of a Pin.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct MoveProposal {
    pub version: SemVer,
    pub stability: Stability,
    /// Required endpoints deprecated in that version.
    pub deprecated: Vec<EndpointRef>,
}

/// One Pin of a Consumer with the newer versions it could move to.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct PinMoves {
    pub producer: String,
    pub api_type: ApiType,
    pub version: SemVer,
    pub stability: Stability,
    /// Ascending.
    pub newer: Vec<CandidateVersion>,
    pub proposal: Option<MoveProposal>,
}

/// A Pin: Producer, API type (as text, for ordering) and version.
type PinKey = (String, &'static str, SemVer);
/// The `(path, method)` pairs a Consumer requires of one Pin.
type RequiredEndpoints = BTreeSet<(String, String)>;

/// Every Pin of a Consumer with its newer versions and its free move.
pub async fn consumer_moves(
    repo: &impl SpecRepository,
    consumername: &str,
) -> Result<Vec<PinMoves>, AppError> {
    let mut pins: BTreeMap<PinKey, (ApiType, RequiredEndpoints)> = BTreeMap::new();
    for e in repo.list_consumer_endpoints(consumername).await? {
        pins.entry((e.service.clone(), e.api_type.as_str(), e.version))
            .or_insert_with(|| (e.api_type, BTreeSet::new()))
            .1
            .insert((e.path, e.method));
    }
    let mut out = Vec::new();
    for ((producer, _, version), (api_type, required)) in pins {
        if let Some(moves) = pin_moves(repo, &producer, api_type, version, &required, false).await?
        {
            out.push(moves);
        }
    }
    Ok(out)
}

/// The version a Consumer could move its Pin to for free, for the require
/// answer's `X-Sanshain-Upgrade`: the endpoints recorded for that Pin and the
/// ones of this call.
pub async fn move_proposal(
    repo: &impl SpecRepository,
    consumername: &str,
    producername: &str,
    api_type: ApiType,
    version: SemVer,
    requested: &[(String, String)],
) -> Result<Option<SemVer>, AppError> {
    let mut required: BTreeSet<(String, String)> = requested.iter().cloned().collect();
    for e in repo.list_consumer_endpoints(consumername).await? {
        if e.service == producername && e.api_type == api_type && e.version == version {
            required.insert((e.path, e.method));
        }
    }
    Ok(
        pin_moves(repo, producername, api_type, version, &required, true)
            .await?
            .and_then(|m| m.proposal)
            .map(|p| p.version),
    )
}

/// The newer versions of one Pin against its required endpoints. A GA Pin
/// counts newer GAs only, a snapshot Pin newer GAs and snapshots. With
/// `first_free_only` the walk stops at the newest free version (the require
/// path needs nothing else). `None` when the Pin no longer exists.
async fn pin_moves(
    repo: &impl SpecRepository,
    producer: &str,
    api_type: ApiType,
    version: SemVer,
    required: &BTreeSet<(String, String)>,
    first_free_only: bool,
) -> Result<Option<PinMoves>, AppError> {
    let Some(service_id) = repo.find_service(producer).await? else {
        return Ok(None);
    };
    let line: Vec<SpecVersionMeta> = repo
        .list_spec_versions(service_id)
        .await?
        .into_iter()
        .filter(|v| v.api_type == api_type)
        .collect();
    let Some(pinned) = line.iter().find(|v| v.version == version).cloned() else {
        return Ok(None);
    };
    // What the Consumer builds against today; an endpoint the pinned version
    // lacks (a stale record) has nothing to compare with and is left out.
    let mut today: Vec<(String, String, String)> = Vec::new();
    for (path, method) in required {
        if let Some((_, yaml, _)) = repo
            .find_endpoint(pinned.id, api_type, path, method)
            .await?
        {
            today.push((path.clone(), method.clone(), yaml));
        }
    }
    let mut candidates: Vec<SpecVersionMeta> = line
        .iter()
        .filter(|v| {
            v.version > version
                && (pinned.stability == Stability::Snapshot || v.stability == Stability::Ga)
        })
        .cloned()
        .collect();
    candidates.sort_by_key(|c| std::cmp::Reverse(c.version));
    let mut newer = Vec::new();
    let mut proposal = None;
    for candidate in candidates {
        let seen = seen_through(repo, api_type, &candidate, &today).await?;
        let free = !today.is_empty() && seen.iter().all(|e| e.change == MoveChange::Unchanged);
        if free && proposal.is_none() {
            proposal = Some(MoveProposal {
                version: candidate.version,
                stability: candidate.stability,
                deprecated: seen
                    .iter()
                    .filter(|e| e.deprecated)
                    .map(|e| EndpointRef {
                        method: e.method.clone(),
                        path: e.path.clone(),
                    })
                    .collect(),
            });
            if first_free_only {
                break;
            }
        }
        newer.push(CandidateVersion {
            version: candidate.version,
            stability: candidate.stability,
            endpoints: seen,
            free,
        });
    }
    newer.reverse();
    Ok(Some(PinMoves {
        producer: producer.to_string(),
        api_type,
        version,
        stability: pinned.stability,
        newer,
        proposal,
    }))
}

/// The required endpoints as one newer version has them.
async fn seen_through(
    repo: &impl SpecRepository,
    api_type: ApiType,
    candidate: &SpecVersionMeta,
    today: &[(String, String, String)],
) -> Result<Vec<EndpointMove>, AppError> {
    let mut seen = Vec::with_capacity(today.len());
    let mut to_classify: Vec<(usize, String, String)> = Vec::new();
    for (path, method, old) in today {
        let found = repo
            .find_endpoint(candidate.id, api_type, path, method)
            .await?;
        let (change, deprecated) = match found {
            None => (MoveChange::Removed, false),
            Some((_, yaml, deprecated)) if same_snippet(api_type, &yaml, old) => {
                (MoveChange::Unchanged, deprecated)
            }
            // Only the OpenAPI analyzer tells an additive schema change from
            // one that keeps the shape; for AsyncAPI and proto any other
            // bytes count as changed.
            Some((_, yaml, deprecated)) if api_type == ApiType::OpenApi => {
                to_classify.push((seen.len(), old.clone(), yaml));
                (MoveChange::Changed, deprecated)
            }
            Some((_, _, deprecated)) => (MoveChange::Changed, deprecated),
        };
        seen.push(EndpointMove {
            method: method.clone(),
            path: path.clone(),
            change,
            deprecated,
        });
    }
    if !to_classify.is_empty() {
        // The analyzer parses both snippets: CPU work, off the async worker.
        let kept: Vec<usize> = super::run_cpu_bound(move || {
            Ok(to_classify
                .into_iter()
                .filter(|(_, old, new)| {
                    matches!(
                        classify_change(ApiType::OpenApi, old, new, 0),
                        Impact::Patch | Impact::None
                    )
                })
                .map(|(i, _, _)| i)
                .collect())
        })
        .await?;
        for i in kept {
            if let Some(e) = seen.get_mut(i) {
                e.change = MoveChange::Unchanged;
            }
        }
    }
    Ok(seen)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(path: &str, yaml: &str, deprecated: bool) -> EndpointRecord {
        EndpointRecord {
            id: None,
            api_type: ApiType::OpenApi,
            path: path.to_string(),
            normalized_path: path.to_string(),
            method: "GET".to_string(),
            yaml_content: yaml.to_string(),
            deprecated,
        }
    }

    fn meta(version: &str) -> SpecVersionMeta {
        SpecVersionMeta {
            id: 1,
            service_id: 1,
            api_type: ApiType::OpenApi,
            version: version.parse().unwrap(),
            stability: Stability::Snapshot,
            content_hash: String::new(),
            provided_by: String::new(),
            created_at: String::new(),
            updated_at: String::new(),
            trunk_provided_at: None,
            last_required_at: None,
        }
    }

    fn refs(paths: &[&str]) -> Vec<EndpointRef> {
        paths
            .iter()
            .map(|p| EndpointRef {
                method: "GET".to_string(),
                path: p.to_string(),
            })
            .collect()
    }

    #[test]
    fn snippets_that_differ_only_in_their_version_are_the_same() {
        let v = |version: &str, text: &str| {
            format!(
                "openapi: 3.0.3\ninfo:\n  title: T\n  version: {version}\npaths:\n  /a:\n    get:\n      description: {text}\n"
            )
        };
        assert!(same_snippet(
            ApiType::OpenApi,
            &v("1.0.0", "x"),
            &v("2.0.0", "x")
        ));
        assert!(!same_snippet(
            ApiType::OpenApi,
            &v("1.0.0", "x"),
            &v("1.0.0", "y")
        ));
        assert!(!same_snippet(ApiType::AsyncApi, "a: [", "a: b"));
        assert!(same_snippet(
            ApiType::Proto,
            "// sanshain-version: 1.0.0\nservice S {}",
            "// sanshain-version: 1.1.0\nservice S {}"
        ));
        assert!(!same_snippet(
            ApiType::Proto,
            "service S {}",
            "service T {}"
        ));
    }

    #[test]
    fn the_diff_names_added_changed_removed_and_newly_deprecated_endpoints() {
        let base = vec![
            record("/a", "a", false),
            record("/b", "b", false),
            record("/c", "c", false),
            record("/d", "d", true),
        ];
        let new = vec![
            record("/a", "a", false),
            record("/b", "b2", false),
            record("/c", "c-deprecated", true),
            record("/d", "d", true),
            record("/e", "e", false),
        ];
        let changes = endpoint_diff(&meta("1.1.0"), Some("1.0.0".parse().unwrap()), &base, &new);
        assert_eq!(changes.added, refs(&["/e"]));
        assert_eq!(changes.changed, refs(&["/b"]));
        assert_eq!(changes.deprecated, refs(&["/c"]));
        assert_eq!(changes.removed, Vec::<EndpointRef>::new());
        let gone = endpoint_diff(&meta("2.0.0"), None, &new, &base[..1]);
        assert_eq!(gone.removed, refs(&["/b", "/c", "/d", "/e"]));
        assert_eq!(gone.baseline, None);
    }
}
