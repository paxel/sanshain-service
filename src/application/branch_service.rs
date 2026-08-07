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
            },
        )
        .await
    {
        tracing::warn!("Could not record the branch creation in the audit log: {e}");
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

/// A branch's pin set — current, or as it was at `at`.
pub async fn get_branch_graph(
    repo: &impl SpecRepository,
    name: &str,
    at: Option<&str>,
) -> Result<Vec<TrunkPinInfo>, AppError> {
    let branch = repo
        .find_branch(name)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("no sanshain-branch '{name}'")))?;
    Ok(repo.list_branch_pins(branch.id, at).await?)
}
