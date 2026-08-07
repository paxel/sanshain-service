//! Sanshain-branches (ADR-0005): named graphs created by a releaser as a copy
//! of a source graph (trunk, or another branch) at a chosen instant — the
//! release-cut act, retroactively repairable by picking a past date.

use crate::domain::models::*;
use crate::domain::ports::{NewAuditLog, RepositoryError, SpecRepository};
use tracing::instrument;

fn now_iso() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
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
    if name.is_empty() {
        return Err(AppError::BadRequest(
            "a sanshain-branch needs a non-empty name".to_string(),
        ));
    }
    let as_of = match params.as_of {
        Some(raw) => chrono::DateTime::parse_from_rfc3339(raw)
            .map_err(|e| AppError::BadRequest(format!("as_of is not an RFC 3339 instant: {e}")))?
            .to_utc()
            .to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
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
    match &source_branch {
        Some(src) => {
            repo.copy_branch_graph_to_branch(id, src.id, &as_of, &now)
                .await?
        }
        None => repo.copy_trunk_graph_to_branch(id, &as_of, &now).await?,
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
    if new_name.is_empty() {
        return Err(AppError::BadRequest(
            "a sanshain-branch needs a non-empty name".to_string(),
        ));
    }
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

/// A branch's pin set — current, or as it was at `at`. Every pin is checked
/// against the version store: a deleted version renders as a dangling
/// reference (never silently dropped) and heals when re-provided.
pub async fn get_branch_graph(
    repo: &impl SpecRepository,
    name: &str,
    at: Option<&str>,
) -> Result<Vec<TrunkPinInfo>, AppError> {
    let branch = repo
        .find_branch(name)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("no sanshain-branch '{name}'")))?;
    let mut pins = repo.list_branch_pins(branch.id, at).await?;
    for pin in &mut pins {
        pin.dangling = match repo.find_service(&pin.service).await? {
            Some(sid) => repo
                .find_spec_version(sid, pin.api_type, pin.version)
                .await?
                .is_none(),
            None => true,
        };
    }
    Ok(pins)
}

/// One side of a graph diff: `main[@instant]` or `<branch>[@instant]`.
/// A trailing `@<instant>` is only split off when it parses as a date, so
/// branch names containing `@` keep working.
async fn resolve_graph_selection(
    repo: &impl SpecRepository,
    raw: &str,
) -> Result<Vec<TrunkPinInfo>, AppError> {
    let (name, at) = match raw.rsplit_once('@') {
        Some((name, at)) if chrono::DateTime::parse_from_rfc3339(at).is_ok() => {
            (name, Some(at.to_string()))
        }
        _ => (raw, None),
    };
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

    let key = |p: &TrunkPinInfo| {
        (
            p.client.clone(),
            p.service.clone(),
            p.api_type,
            p.path.clone(),
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

    let names =
        |m: &std::collections::HashMap<(String, String, ApiType, String, String), TrunkPinInfo>| {
            m.values()
                .flat_map(|p| [p.client.clone(), p.service.clone()])
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
