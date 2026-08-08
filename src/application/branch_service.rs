//! Sanshain-branches (ADR-0005): named graphs created by a releaser as a copy
//! of a source graph (trunk, or another branch) at a chosen instant — the
//! release-cut act, retroactively repairable by picking a past date.

use super::now_iso;
use crate::domain::models::*;
use crate::domain::ports::{NewAuditLog, RepositoryError, SpecRepository};
use tracing::instrument;

/// Normalize a user-supplied RFC 3339 instant to the stored timestamp shape:
/// UTC, second precision, `Z` suffix. Stored `valid_from`/`valid_to` stamps
/// take part in lexicographic TEXT comparisons, so a query instant in any
/// other offset or precision would compare wrongly instead of erroring.
pub(crate) fn normalize_instant(field: &str, raw: &str) -> Result<String, AppError> {
    Ok(chrono::DateTime::parse_from_rfc3339(raw)
        .map_err(|e| AppError::BadRequest(format!("{field} is not an RFC 3339 instant: {e}")))?
        .to_utc()
        .to_rfc3339_opts(chrono::SecondsFormat::Secs, true))
}

/// Split a graph selector `name[@instant]`. The trailing `@<instant>` is only
/// split off when it parses as a date, so branch names containing `@` keep
/// working; the instant comes back normalized (see [`normalize_instant`]).
///
/// The one case where a failed parse is an error rather than a name: the
/// prefix is a built-in selector, which can never be a branch name (they are
/// reserved), so `main@2026-13-45` is a date typo — saying "no sanshain-branch
/// 'main@2026-13-45'" would answer the wrong question.
pub(crate) fn split_selector(raw: &str) -> Result<(&str, Option<String>), AppError> {
    match raw.rsplit_once('@') {
        Some((name, at)) => match normalize_instant("the selector's instant", at) {
            Ok(normalized) => Ok((name, Some(normalized))),
            Err(e) if RESERVED_BRANCH_NAMES.contains(&name) => Err(e),
            Err(_) => Ok((raw, None)),
        },
        None => Ok((raw, None)),
    }
}

/// Names the stream vocabulary already claims: `main` (the trunk graph) and
/// `dev` (accumulated activity) are fixed graph selectors, and `trunk` is the
/// stream sentinel a trunk-flagged build writes into the audit trail. A branch
/// carrying one of these is creatable but ambiguous everywhere it is read —
/// shadowed in `/report?scope=` and the diff endpoint, or indistinguishable
/// from trunk CI in the audit stream filter — so it is refused up front.
const RESERVED_BRANCH_NAMES: [&str; 3] = ["main", "dev", "trunk"];

fn validate_branch_name(name: &str) -> Result<(), AppError> {
    if name.is_empty() {
        return Err(AppError::BadRequest(
            "a sanshain-branch needs a non-empty name".to_string(),
        ));
    }
    if RESERVED_BRANCH_NAMES.contains(&name) {
        return Err(AppError::BadRequest(format!(
            "'{name}' is reserved for the built-in graph views and streams, and cannot name a sanshain-branch"
        )));
    }
    Ok(())
}

pub struct CreateBranchParams<'a> {
    pub name: &'a str,
    /// An existing branch's name; trunk when `None`.
    pub source: Option<&'a str>,
    /// RFC 3339 instant; now when `None`. Retroactive creation is a
    /// first-class path (ADR-0005).
    pub as_of: Option<&'a str>,
    /// The authenticated releaser, for provenance and the audit trail.
    pub created_by: &'a str,
}

#[instrument(skip_all)]
pub async fn create_branch(
    repo: &impl SpecRepository,
    params: CreateBranchParams<'_>,
) -> Result<BranchInfo, AppError> {
    let name = params.name.trim();
    validate_branch_name(name)?;
    let as_of = match params.as_of {
        Some(raw) => normalize_instant("as_of", raw)?,
        None => now_iso(),
    };

    // Resolve the source before writing anything: an unknown source must not
    // leave an empty branch behind.
    let source_branch = match params.source {
        Some(source_name) => Some(repo.find_branch(source_name).await?.ok_or_else(|| {
            AppError::NotFound(format!("no sanshain-branch '{source_name}' to branch from"))
        })?),
        None => None,
    };
    let source_label = source_branch
        .as_ref()
        .map(|b| b.name.clone())
        .unwrap_or_else(|| "trunk".to_string());

    let now = now_iso();
    let id = repo
        .insert_branch(name, &now, params.created_by, &source_label, &as_of)
        .await
        .map_err(|e| match e {
            RepositoryError::Conflict => AppError::Conflict(format!(
                "a sanshain-branch named '{name}' already exists — pick another name, or have an admin rename/delete the existing one"
            )),
            other => other.into(),
        })?;
    let copied = match &source_branch {
        Some(src) => {
            repo.copy_branch_graph_to_branch(id, src.id, &as_of, &now)
                .await
        }
        None => repo.copy_trunk_graph_to_branch(id, &as_of, &now).await,
    };
    if let Err(e) = copied {
        // The branch row is already committed; left behind it would hold the
        // name as a plausible-looking empty release cut and 409 every retry.
        if let Err(cleanup) = repo.delete_branch(id).await {
            tracing::warn!(
                "Could not remove sanshain-branch '{name}' after its graph copy failed: {cleanup}"
            );
        }
        return Err(e.into());
    }

    let details = format!("Created sanshain-branch '{name}' from {source_label} as of {as_of}");
    if let Err(e) = repo
        .insert_audit_log(
            params.created_by,
            NewAuditLog {
                action: "BRANCH_CREATED",
                details: &details,
                service: None,
                version: None,
                action_type: Some("WRITE"),
                diff: None,
                stream: None,
            },
        )
        .await
    {
        tracing::warn!("Could not record the branch creation in the audit log: {e}");
    }

    if let Ok(branches) = repo.list_branches().await {
        metrics::gauge!("sanshain_branches").set(branches.len() as f64);
    }
    Ok(BranchInfo {
        id,
        name: name.to_string(),
        created_at: now,
        created_by: params.created_by.to_string(),
        source: source_label,
        as_of,
    })
}

/// Every sanshain-branch, newest first.
pub async fn list_branches(repo: &impl SpecRepository) -> Result<Vec<BranchInfo>, AppError> {
    Ok(repo.list_branches().await?)
}

/// Rename a branch — the repair for a botched name at creation (ADR-0005).
/// Identity is the id: membership, timeline and audit stamps survive; a
/// pipeline still sending the old tag name gets the instructive 404 until
/// reconfigured, which is intended.
#[instrument(skip_all)]
pub async fn rename_branch(
    repo: &impl SpecRepository,
    name: &str,
    new_name: &str,
    actor: &str,
) -> Result<BranchInfo, AppError> {
    let new_name = new_name.trim();
    validate_branch_name(new_name)?;
    let mut branch = repo
        .find_branch(name)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("no sanshain-branch '{name}'")))?;
    repo.rename_branch(branch.id, new_name)
        .await
        .map_err(|e| match e {
            RepositoryError::Conflict => AppError::Conflict(format!(
                "a sanshain-branch named '{new_name}' already exists"
            )),
            other => other.into(),
        })?;

    let details = format!("Renamed sanshain-branch '{name}' to '{new_name}'");
    if let Err(e) = repo
        .insert_audit_log(
            actor,
            NewAuditLog {
                action: "BRANCH_RENAMED",
                details: &details,
                service: None,
                version: None,
                action_type: Some("WRITE"),
                diff: None,
                stream: None,
            },
        )
        .await
    {
        tracing::warn!("Could not record the branch rename in the audit log: {e}");
    }
    branch.name = new_name.to_string();
    Ok(branch)
}

/// Delete a branch — the deliberate, audited EOL act. Frees the name.
#[instrument(skip_all)]
pub async fn delete_branch(
    repo: &impl SpecRepository,
    name: &str,
    actor: &str,
) -> Result<(), AppError> {
    let branch = repo
        .find_branch(name)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("no sanshain-branch '{name}'")))?;
    repo.delete_branch(branch.id).await?;

    let details = format!("Deleted sanshain-branch '{name}' — the name is free again");
    if let Err(e) = repo
        .insert_audit_log(
            actor,
            NewAuditLog {
                action: "BRANCH_DELETED",
                details: &details,
                service: None,
                version: None,
                action_type: Some("WRITE"),
                diff: None,
                stream: None,
            },
        )
        .await
    {
        tracing::warn!("Could not record the branch deletion in the audit log: {e}");
    }
    if let Ok(branches) = repo.list_branches().await {
        metrics::gauge!("sanshain_branches").set(branches.len() as f64);
    }
    Ok(())
}

/// Check every pin against the version store: a deleted version renders as a
/// dangling reference (never silently dropped) and heals when re-provided.
///
/// Memoized per pin key rather than one sweep of the version store: this runs
/// on every graph read, and the lookups below are cached, so repeat keys cost
/// nothing while a full `list_all_spec_versions` scan (unbounded, with a
/// per-row endpoint count) would be paid in full every time.
pub(crate) async fn mark_dangling(
    repo: &impl SpecRepository,
    pins: &mut [TrunkPinInfo],
) -> Result<(), AppError> {
    let mut service_ids: std::collections::HashMap<String, Option<i64>> = Default::default();
    let mut seen: std::collections::HashMap<(i64, ApiType, SemVer), bool> = Default::default();
    for pin in pins.iter_mut() {
        let sid = match service_ids.get(&pin.service) {
            Some(cached) => *cached,
            None => {
                let looked_up = repo.find_service(&pin.service).await?;
                service_ids.insert(pin.service.clone(), looked_up);
                looked_up
            }
        };
        pin.dangling = match sid {
            Some(sid) => match seen.get(&(sid, pin.api_type, pin.version)) {
                Some(cached) => *cached,
                None => {
                    let missing = repo
                        .find_spec_version(sid, pin.api_type, pin.version)
                        .await?
                        .is_none();
                    seen.insert((sid, pin.api_type, pin.version), missing);
                    missing
                }
            },
            None => true,
        };
    }
    Ok(())
}

/// A branch's pin set — current, or as it was at `at` — dangling references
/// marked (see [`mark_dangling`]).
pub async fn get_branch_graph(
    repo: &impl SpecRepository,
    name: &str,
    at: Option<&str>,
) -> Result<Vec<TrunkPinInfo>, AppError> {
    let at = at.map(|a| normalize_instant("at", a)).transpose()?;
    let branch = repo
        .find_branch(name)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("no sanshain-branch '{name}'")))?;
    let mut pins = repo.list_branch_pins(branch.id, at.as_deref()).await?;
    mark_dangling(repo, &mut pins).await?;
    Ok(pins)
}

/// The main graph — current, or as it was at `at` — with dangling references
/// marked exactly like a branch graph: a pin on a deleted version must look
/// broken in every view, not only in branch view.
pub async fn get_trunk_graph(
    repo: &impl SpecRepository,
    at: Option<&str>,
) -> Result<Vec<TrunkPinInfo>, AppError> {
    let at = at.map(|a| normalize_instant("at", a)).transpose()?;
    let mut pins = match at.as_deref() {
        Some(at) => repo.list_trunk_pins_at(at).await?,
        None => repo.list_current_trunk_pins().await?,
    };
    mark_dangling(repo, &mut pins).await?;
    Ok(pins)
}

/// One side of a graph diff: `main[@instant]` or `<branch>[@instant]`
/// (see [`split_selector`] for the grammar).
async fn resolve_graph_selection(
    repo: &impl SpecRepository,
    raw: &str,
) -> Result<Vec<TrunkPinInfo>, AppError> {
    let (name, at) = split_selector(raw)?;
    if name == "main" {
        return Ok(match at.as_deref() {
            Some(at) => repo.list_trunk_pins_at(at).await?,
            None => repo.list_current_trunk_pins().await?,
        });
    }
    let branch = repo
        .find_branch(name)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("no sanshain-branch '{name}'")))?;
    Ok(repo.list_branch_pins(branch.id, at.as_deref()).await?)
}

/// One pin whose version differs between the two sides of a diff.
#[derive(serde::Serialize, Clone, Debug)]
pub struct PinChange {
    pub client: String,
    pub service: String,
    pub api_type: ApiType,
    pub path: String,
    pub method: String,
    pub from: SemVer,
    pub to: SemVer,
}

/// A structured diff between two graph selections (ADR-0005): what a release
/// train changed, or what trunk changed since a cut — stated, not eyeballed.
#[derive(serde::Serialize, Clone, Debug)]
pub struct GraphDiff {
    pub left: String,
    pub right: String,
    pub services_added: Vec<String>,
    pub services_removed: Vec<String>,
    pub pins_added: Vec<TrunkPinInfo>,
    pub pins_removed: Vec<TrunkPinInfo>,
    pub pins_changed: Vec<PinChange>,
}

#[instrument(skip_all)]
pub async fn diff_graphs(
    repo: &impl SpecRepository,
    left: &str,
    right: &str,
) -> Result<GraphDiff, AppError> {
    let left_pins = resolve_graph_selection(repo, left).await?;
    let right_pins = resolve_graph_selection(repo, right).await?;

    // Keyed on the normalized path, because that is what the pin stores match
    // on: a pin refreshed after its raw path was respelled across versions
    // keeps the old spelling, so keying on `path` would report one moved pin
    // as a removal plus an addition — the wrong answer in the endpoint release
    // engineers read to see what a train changed. The raw path still travels
    // on the pin for display.
    let key = |p: &TrunkPinInfo| {
        (
            p.client.clone(),
            p.service.clone(),
            p.api_type,
            p.normalized_path.clone(),
            p.method.clone(),
        )
    };
    let left_map: std::collections::HashMap<_, TrunkPinInfo> =
        left_pins.into_iter().map(|p| (key(&p), p)).collect();
    let right_map: std::collections::HashMap<_, TrunkPinInfo> =
        right_pins.into_iter().map(|p| (key(&p), p)).collect();

    let mut pins_added = Vec::new();
    let mut pins_removed = Vec::new();
    let mut pins_changed = Vec::new();
    for (k, r) in &right_map {
        match left_map.get(k) {
            None => pins_added.push(r.clone()),
            Some(l) if l.version != r.version => pins_changed.push(PinChange {
                client: r.client.clone(),
                service: r.service.clone(),
                api_type: r.api_type,
                path: r.path.clone(),
                method: r.method.clone(),
                from: l.version,
                to: r.version,
            }),
            Some(_) => {}
        }
    }
    for (k, l) in &left_map {
        if !right_map.contains_key(k) {
            pins_removed.push(l.clone());
        }
    }

    // Producers only: the field says "services", the UI renders it as
    // "+ service X", and a name is unique per table — folding Consumers in
    // would both mislabel them and let a same-named Consumer already on the
    // left mask a genuinely added Producer.
    let names =
        |m: &std::collections::HashMap<(String, String, ApiType, String, String), TrunkPinInfo>| {
            m.values()
                .map(|p| p.service.clone())
                .collect::<std::collections::BTreeSet<String>>()
        };
    let left_names = names(&left_map);
    let right_names = names(&right_map);
    let services_added = right_names.difference(&left_names).cloned().collect();
    let services_removed = left_names.difference(&right_names).cloned().collect();

    let sort_pins = |v: &mut Vec<TrunkPinInfo>| {
        v.sort_by(|a, b| {
            (&a.client, &a.service, &a.path, &a.method)
                .cmp(&(&b.client, &b.service, &b.path, &b.method))
        })
    };
    sort_pins(&mut pins_added);
    sort_pins(&mut pins_removed);
    pins_changed.sort_by(|a, b| {
        (&a.client, &a.service, &a.path, &a.method)
            .cmp(&(&b.client, &b.service, &b.path, &b.method))
    });

    Ok(GraphDiff {
        left: left.to_string(),
        right: right.to_string(),
        services_added,
        services_removed,
        pins_added,
        pins_removed,
        pins_changed,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_selector_normalizes_instants_into_the_stored_form() {
        assert_eq!(split_selector("main").unwrap(), ("main", None));
        assert_eq!(
            split_selector("main@2026-01-01T02:00:00+02:00").unwrap(),
            ("main", Some("2026-01-01T00:00:00Z".to_string()))
        );
        assert_eq!(
            split_selector("rel@2026-01-01T00:00:00.250Z").unwrap(),
            ("rel", Some("2026-01-01T00:00:00Z".to_string()))
        );
        // A trailing @part that is no date stays part of the name.
        assert_eq!(
            split_selector("release@candidate").unwrap(),
            ("release@candidate", None)
        );
    }

    #[test]
    fn a_bad_instant_on_a_built_in_selector_is_a_date_error() {
        // 'main' and 'dev' are reserved, so they can never be branch names —
        // a failed instant there is a typo, and must say so rather than send
        // the caller looking for a branch called 'main@2026-13-45'.
        for raw in ["main@2026-13-45", "dev@yesterday", "trunk@nope"] {
            match split_selector(raw) {
                Err(AppError::BadRequest(msg)) => {
                    assert!(msg.contains("instant"), "{raw} got: {msg}")
                }
                other => panic!("{raw} expected a date error, got {other:?}"),
            }
        }
        // A real branch name may still carry an '@' with junk behind it.
        assert_eq!(
            split_selector("rel@candidate").unwrap(),
            ("rel@candidate", None)
        );
    }

    #[test]
    fn normalize_instant_rejects_non_dates_and_names_the_field() {
        let err = normalize_instant("at", "banana").unwrap_err();
        match err {
            AppError::BadRequest(msg) => assert!(msg.starts_with("at is not"), "got: {msg}"),
            other => panic!("expected BadRequest, got {other:?}"),
        }
        assert_eq!(
            normalize_instant("at", "2026-01-01T02:00:00+02:00").unwrap(),
            "2026-01-01T00:00:00Z"
        );
    }

    #[test]
    fn reserved_and_empty_branch_names_are_refused() {
        assert!(validate_branch_name("").is_err());
        // The graph selectors, and the stream sentinel a trunk build writes.
        for reserved in RESERVED_BRANCH_NAMES {
            assert!(
                validate_branch_name(reserved).is_err(),
                "'{reserved}' must be refused"
            );
        }
        assert!(validate_branch_name("rel-1").is_ok());
        // Only the exact names are claimed — not anything containing them.
        assert!(validate_branch_name("trunk-2").is_ok());
        assert!(validate_branch_name("main-release").is_ok());
    }
}
