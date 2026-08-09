//! The Provide and Require use-cases of the version-line model (ADR-0003).
//!
//! A Provide carries a complete spec whose version is read from the document
//! itself; the caller declares its stability. A Require pins an exact version
//! and either gets exactly that or fails immediately — no fallback, no
//! waiting. All decisions live here; repositories persist what they are told.

use super::now_iso;
use super::require_service::{count_branch_update, method_for, resolve_pin};
use super::version_rules::{
    check_compatibility, classify_change, diff_endpoints, ga_baseline, propose_free,
};
use crate::asyncapi;
use crate::domain::models::*;
use crate::domain::permissions::{Actor, Permission};
use crate::domain::ports::{NewAuditLog, SpecRepository, UpsertSpecVersion};
use crate::openapi;
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};

use tracing::instrument;

pub struct ProvideSpecParams<'a> {
    pub producername: &'a str,
    pub api_type: ApiType,
    pub content: &'a str,
    /// Declared by the caller: `snapshot` (overwritable) or `ga` (immutable).
    pub stability: Stability,
    pub dry_run: bool,
    /// ADR-0004: this build belongs to the trunk stream — a real provide
    /// (including the idempotent no-op) refreshes the entry's
    /// `trunk_provided_at` marker. No effect on version rules or stability.
    pub trunk: bool,
    /// ADR-0005: this build belongs to the named sanshain-branch — a real
    /// provide marks the producer's member version within it. Mutually
    /// exclusive with `trunk`.
    pub tag: Option<&'a str>,
    /// The resolved caller: the GA gate checks it for
    /// [`Permission::ReleaseGa`], and its username is the single identity used
    /// for audit attribution and the version's `provided_by` credit. `None`
    /// (no authenticated Actor) can never release.
    pub caller: Option<Actor>,
    /// Compare-and-set for promote-by-copy (#28): when `true`, the write only
    /// lands if the stored entry still carries the hash of the exact bytes
    /// being submitted, so a snapshot overwritten between read and release
    /// answers 409 instead of silently going GA with stale bytes.
    /// The expected hash is derived in [`provide_spec`] from `content` itself —
    /// never passed in — so guard and payload cannot drift apart: if a
    /// replica's content cache is stale, the derived hash is stale with it,
    /// the stored row doesn't match, and the release refuses instead of
    /// freezing old bytes.
    pub require_prior_content_match: bool,
}

/// A compact fingerprint of submitted spec content for diagnostics: its byte
/// length and a short SHA-256 prefix. Logged instead of the full spec on a parse
/// failure so a large submission cannot flood the in-memory (admin-viewable) log
/// buffer. Spec content is public by design, so this is about log hygiene, not
/// secrecy.
fn spec_content_fingerprint(content: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    let hash = hex::encode(hasher.finalize());
    format!("{} bytes, sha256:{}", content.len(), &hash[..16])
}

fn content_hash(content: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    format!("sha256:{}", hex::encode(hasher.finalize()))
}

/// Resolve the declared stream (ADR-0004/0005): `trunk` and `tag` are
/// mutually exclusive, and a tag must name an existing sanshain-branch —
/// no auto-create, so a typo'd pipeline cannot mint a half-empty release
/// graph. Applies to dry-runs too: a misconfigured pipeline must hear it.
pub(crate) async fn resolve_stream(
    repo: &impl SpecRepository,
    trunk: bool,
    tag: Option<&str>,
) -> Result<Option<BranchInfo>, AppError> {
    if trunk && tag.is_some() {
        return Err(AppError::BadRequest(
            "`trunk` and `tag` are mutually exclusive — a build belongs to the trunk stream or to a named sanshain-branch, never both".to_string(),
        ));
    }
    match tag {
        Some(name) => Ok(Some(repo.find_branch(name).await?.ok_or_else(|| {
            AppError::NotFound(format!(
                "no sanshain-branch '{name}' — a releaser must create it first"
            ))
        })?)),
        None => Ok(None),
    }
}

/// Read the Producer-declared version out of the spec document.
fn extract_spec_version(api_type: ApiType, content: &str) -> Result<SemVer, AppError> {
    match api_type {
        ApiType::OpenApi | ApiType::AsyncApi => {
            openapi::extract_info_version(content).map_err(AppError::BadRequest)
        }
        ApiType::Proto => {
            crate::proto::extract_sanshain_version(content).map_err(AppError::BadRequest)
        }
    }
}

/// Two operations whose paths differ only in a parameter's name — or in a
/// trailing slash, or doubled separators — are the *same* endpoint: OpenAPI
/// forbids them outright ("templated paths with the same hierarchy but
/// different templated names MUST NOT exist"), and nothing downstream could
/// act on the difference. A Consumer pinning "/users/{id}" cannot say which of
/// the two it meant, and the dev and main graphs would count the edge
/// differently. Refused at publish, where the spec can still be fixed.
fn reject_colliding_endpoints(endpoints: &[openapi::EndpointSpec]) -> Result<(), AppError> {
    let mut seen: HashMap<(&str, &str), &str> = HashMap::new();
    for e in endpoints {
        if let Some(first) = seen.insert((&e.normalized_path, &e.method), &e.path)
            && first != e.path
        {
            return Err(AppError::BadRequest(format!(
                "'{}' and '{}' are the same {} endpoint — they differ only in a \
                 path-parameter name or separator, so nothing can tell them apart. \
                 Give them distinct paths, or publish only one.",
                first, e.path, e.method
            )));
        }
    }
    Ok(())
}

fn parse_spec_endpoints(
    api_type: ApiType,
    content: &str,
    producername: &str,
) -> Result<Vec<openapi::EndpointSpec>, AppError> {
    let endpoints = parse_spec_endpoints_inner(api_type, content, producername)?;
    reject_colliding_endpoints(&endpoints)?;
    Ok(endpoints)
}

fn parse_spec_endpoints_inner(
    api_type: ApiType,
    content: &str,
    producername: &str,
) -> Result<Vec<openapi::EndpointSpec>, AppError> {
    match api_type {
        ApiType::OpenApi => openapi::split_openapi(content).map_err(|e| {
            tracing::warn!(
                "Failed to split OpenAPI for service '{}' ({}): {}",
                producername,
                spec_content_fingerprint(content),
                e
            );
            AppError::BadRequest(e)
        }),
        ApiType::AsyncApi => {
            let all_specs = crate::asyncapi::split_asyncapi(content).map_err(|e| {
                tracing::warn!(
                    "Failed to split AsyncAPI for service '{}' ({}): {}",
                    producername,
                    spec_content_fingerprint(content),
                    e
                );
                AppError::BadRequest(e)
            })?;
            let sub_count = all_specs.iter().filter(|s| s.operation == "SUB").count();
            if sub_count > 0 {
                let sub_channels: Vec<&str> = all_specs
                    .iter()
                    .filter(|s| s.operation == "SUB")
                    .map(|s| s.channel.as_str())
                    .collect();
                tracing::warn!(
                    "Skipping {} SUB operation(s) for service '{}': subscribe channels {:?} are not stored via /provide. Declare them as 'requires' in sanshain.yaml instead.",
                    sub_count,
                    producername,
                    sub_channels
                );
            }
            Ok(all_specs
                .into_iter()
                .filter(|s| s.operation == "PUB")
                .map(|s| openapi::EndpointSpec {
                    normalized_path: s.channel.clone(),
                    path: s.channel,
                    method: s.operation,
                    yaml_content: s.yaml_content,
                    deprecated: s.deprecated,
                })
                .collect())
        }
        ApiType::Proto => Ok(crate::proto::split_proto(content)
            .map_err(|e| {
                tracing::warn!(
                    "Failed to split Proto for service '{}' ({}): {}",
                    producername,
                    spec_content_fingerprint(content),
                    e
                );
                AppError::BadRequest(e)
            })?
            .into_iter()
            .map(|s| openapi::EndpointSpec {
                normalized_path: s.service.clone(),
                path: s.service,
                method: s.method,
                yaml_content: s.content,
                deprecated: s.deprecated,
            })
            .collect()),
    }
}

/// Promote a stored snapshot to GA without re-uploading (#28).
///
/// Definitionally "provide the stored content with `stability: ga`": the call
/// funnels into [`provide_spec`], so the GA gate, promotion semantics,
/// attribution, audit entries and rejection telemetry are all the same as for
/// any release. An already-GA version is the idempotent no-op; an unknown
/// producer or version is a 404.
pub async fn promote_version(
    repo: &impl SpecRepository,
    producername: &str,
    api_type: ApiType,
    version: SemVer,
    caller: Option<Actor>,
) -> Result<ProvideResponse, AppError> {
    let entry = crate::application::admin_service::find_version_entry(
        repo,
        producername,
        api_type,
        version,
    )
    .await?;
    let content = repo
        .get_spec_content(entry.id)
        .await?
        .ok_or_else(|| AppError::NotFound("Version not found".to_string()))?;
    provide_spec(
        repo,
        ProvideSpecParams {
            producername,
            api_type,
            content: &content,
            stability: Stability::Ga,
            dry_run: false,
            // A promotion is a stability flip, not a stream declaration.
            trunk: false,
            tag: None,
            caller,
            require_prior_content_match: true,
        },
    )
    .await
}

/// A version-rule rejection is self-service, but it is also a signal worth
/// counting: the audit gets a `VERSION_REJECTED` entry and the Prometheus
/// exposition a `sanshain_version_rejected_total` counter. Dry runs are
/// previews and record neither. Best-effort — a failed audit write must not
/// turn a clean 409 into a 500.
async fn record_version_rejection(
    repo: &impl SpecRepository,
    dry_run: bool,
    username: Option<&str>,
    producername: &str,
    version: SemVer,
    reason: &'static str,
    message: &str,
) {
    if dry_run {
        return;
    }
    // The producer name becomes a Prometheus label and an audit row, so only
    // Producers that exist are recorded — otherwise any authenticated caller
    // could mint unbounded label cardinality out of made-up names. The check
    // lives here, not at call sites, so no future caller can forget it; and it
    // is best-effort like the rest of this function — a DB hiccup while
    // deciding whether to record must not turn a clean refusal into a 500.
    match repo.find_service(producername).await {
        Ok(Some(_)) => {}
        Ok(None) => return,
        Err(e) => {
            tracing::warn!(
                service = producername,
                "Could not check producer existence for rejection telemetry: {}",
                e
            );
            return;
        }
    }
    metrics::counter!(
        "sanshain_version_rejected_total",
        "reason" => reason,
        "producer" => producername.to_string()
    )
    .increment(1);
    let actor = username.unwrap_or("DevMode/Anonymous");
    if let Err(e) = repo
        .insert_audit_log(
            actor,
            NewAuditLog {
                action: "VERSION_REJECTED",
                details: message,
                service: Some(producername),
                version: Some(&version.to_string()),
                action_type: Some("REJECT"),
                diff: None,
                stream: None,
                branch_id: None,
            },
        )
        .await
    {
        tracing::warn!(
            service = producername,
            "Could not record the version rejection in the audit log: {}",
            e
        );
    }
}

#[instrument(skip_all)]
pub async fn provide_spec(
    repo: &impl SpecRepository,
    params: ProvideSpecParams<'_>,
) -> Result<ProvideResponse, AppError> {
    let ProvideSpecParams {
        producername,
        api_type,
        content,
        stability,
        dry_run,
        trunk,
        tag,
        caller,
        require_prior_content_match,
    } = params;
    // One identity: authorization and audit attribution both come from the
    // Actor, so they cannot drift apart.
    let username: Option<&str> = caller.as_ref().map(|a| a.username.as_str());
    let tag_branch = resolve_stream(repo, trunk, tag).await?;

    let version = extract_spec_version(api_type, content)?;
    let hash = content_hash(content);
    // The one derivation point of the CAS hash (see the field's doc): the
    // guard compares the stored row against the very bytes being written.
    let expected_prior_hash = require_prior_content_match.then_some(hash.as_str());
    let endpoints = parse_spec_endpoints(api_type, content, producername)?;

    // The GA gate (#27): releasing is a permission, snapshots are open to any
    // authenticated caller. Checked after parsing, so the refusal names a real
    // version and malformed documents keep their 400 — and before any write,
    // so an unauthorized attempt cannot even create the service. All GA
    // shapes are gated, including promotions and the idempotent no-op: a 202
    // must never tell a misconfigured CI that its credentials can release.
    if stability == Stability::Ga
        && !caller
            .as_ref()
            .is_some_and(|a| a.has_permission(Permission::ReleaseGa))
    {
        let message = format!(
            "publishing GA for '{}' requires the 'releaser' role — publish as a snapshot, or ask an administrator to grant the role",
            producername
        );
        record_version_rejection(
            repo,
            dry_run,
            username,
            producername,
            version,
            "ga_requires_releaser",
            &message,
        )
        .await;
        return Err(AppError::ForbiddenWithReason(message));
    }

    tracing::debug!(
        "Providing {:?} {} for service '{}' as {} (dry_run: {})",
        api_type,
        version,
        producername,
        stability.as_str(),
        dry_run
    );

    let sid = if dry_run {
        repo.find_service(producername).await?.unwrap_or(0)
    } else {
        let sid = repo.ensure_service(producername).await?;
        let auto_tag = match api_type {
            ApiType::AsyncApi => Some("messaging".to_string()),
            ApiType::Proto => Some("grpc".to_string()),
            ApiType::OpenApi => None,
        };
        if let Some(tag) = auto_tag {
            repo.add_service_tags(sid, &[tag]).await?;
        }
        sid
    };

    let line: Vec<SpecVersionMeta> = if sid != 0 {
        repo.list_spec_versions(sid)
            .await?
            .into_iter()
            .filter(|v| v.api_type == api_type)
            .collect()
    } else {
        Vec::new()
    };
    let existing = line.iter().find(|v| v.version == version).cloned();

    // Identical content is an idempotent no-op regardless of Actor: CI
    // re-runs of the same commit must never fight. The one exception is a GA
    // Provide for a same-content *snapshot* — that is a promotion, the normal
    // release flow (the released content usually IS the last snapshot), and it
    // must fall through to flip the stability in place.
    if let Some(ref entry) = existing
        && entry.content_hash == hash
        && !(entry.stability == Stability::Snapshot && stability == Stability::Ga)
    {
        // The no-op still counts as "provided": a snapshot a CI re-provides
        // every night is in active use and must not age out of the use-based
        // expiry just because its content never changed.
        if entry.stability == Stability::Snapshot && !dry_run {
            repo.touch_spec_version_provided(entry.id, &now_iso())
                .await?;
        }
        // A no-op trunk re-provide still says "this is trunk's version" — the
        // nightly trunk CI whose content didn't change is what keeps the
        // marker fresh for the trunk TTL.
        if trunk && !dry_run {
            repo.touch_spec_version_trunk(entry.id, &now_iso()).await?;
        }
        // Likewise a no-op tagged re-provide still marks the member version.
        if let Some(branch) = &tag_branch
            && !dry_run
        {
            repo.record_branch_member_version(branch.id, sid, api_type, version, &now_iso())
                .await?;
            count_branch_update(&branch.name);
        }
        return Ok(ProvideResponse {
            version,
            stability: entry.stability,
            content_hash: hash,
            changes: ProvideChanges::default(),
            promoted: false,
        });
    }

    match (&existing, stability) {
        // GA permanently claims its number: different content under the same
        // number is the forgot-to-bump mistake, caught at the door.
        (Some(entry), _) if entry.stability == Stability::Ga => {
            let old_content = repo.get_spec_content(entry.id).await?.unwrap_or_default();
            let changes = diff_endpoints(
                &repo.get_endpoints_for_version(entry.id).await?,
                &endpoints,
                api_type,
            );
            let impact = classify_change(api_type, &old_content, content, changes.inserts);
            let proposed = propose_free(&line, version.increment(impact));
            // Bytes differ but no endpoint the splitter sees changed: the
            // difference is almost certainly whitespace, line endings or
            // comments — name that, or the refusal reads as gaslighting to a
            // caller who changed nothing.
            let cosmetic_hint = if changes.inserts == 0
                && changes.updates == 0
                && changes.deletes == 0
            {
                " (no endpoint content changed — the difference may be whitespace, line endings or comments; documents are compared byte-for-byte)"
            } else {
                ""
            };
            let message = if stability == Stability::Ga {
                format!(
                    "version {} of {} '{}' is GA and immutable, but the submitted content differs — did you forget to increment info's version? Publish as {} instead{}",
                    version,
                    api_type.as_str(),
                    producername,
                    proposed,
                    cosmetic_hint
                )
            } else {
                format!(
                    "version {} of {} '{}' is GA — a released number can never carry a snapshot again; bump your version to {}{}",
                    version,
                    api_type.as_str(),
                    producername,
                    proposed,
                    cosmetic_hint
                )
            };
            let reason = if stability == Stability::Ga {
                "immutable_ga"
            } else {
                "snapshot_on_ga"
            };
            record_version_rejection(
                repo,
                dry_run,
                username,
                producername,
                version,
                reason,
                &message,
            )
            .await;
            return Err(AppError::VersionConflict { message, proposed });
        }
        _ => {}
    }

    // Semver honesty, enforced on GA only: breaking without a major bump is a
    // lie that detonates when a Consumer "safely" repins within the major.
    // Snapshots are declared work-in-progress and are never compat-checked.
    if stability == Stability::Ga
        && let Some(baseline) = ga_baseline(&line, version)
        && version.major <= baseline.version.major
        && let Some(baseline_content) = repo.get_spec_content(baseline.id).await?
        && let Err(reason) = check_compatibility(api_type, &baseline_content, content)
    {
        let proposed = propose_free(&line, SemVer::new(baseline.version.major + 1, 0, 0));
        let message = format!(
            "version {} of {} '{}' is breaking relative to GA {} but does not bump the major — publish as {} instead: {}",
            version,
            api_type.as_str(),
            producername,
            baseline.version,
            proposed,
            reason
        );
        record_version_rejection(
            repo,
            dry_run,
            username,
            producername,
            version,
            "breaking_without_major",
            &message,
        )
        .await;
        return Err(AppError::VersionConflict { message, proposed });
    }

    // `changes` counts are relative to what this exact version stored before
    // (snapshot overwrite / promotion); a brand-new version reports its whole
    // endpoint set as inserts.
    let changes = match &existing {
        Some(entry) => diff_endpoints(
            &repo.get_endpoints_for_version(entry.id).await?,
            &endpoints,
            api_type,
        ),
        None => ProvideChanges {
            inserts: endpoints.len(),
            updates: 0,
            deletes: 0,
        },
    };

    // Item #20: message-level channel contracts, GA provides only. A snapshot
    // claiming global topic ownership would let one developer's WIP block
    // another Producer's release.
    let contract_ops = if api_type == ApiType::AsyncApi && stability == Stability::Ga {
        plan_channel_message_contracts(repo, sid, content).await?
    } else {
        Vec::new()
    };

    let promoting = existing.as_ref().map(|e| e.stability) == Some(Stability::Snapshot)
        && stability == Stability::Ga;

    if dry_run {
        return Ok(ProvideResponse {
            version,
            stability,
            content_hash: hash,
            changes,
            promoted: promoting,
        });
    }

    let overwriting = existing.as_ref().map(|e| e.provided_by.clone());
    // Attribution is the authenticated Actor — with one exception: promoting a
    // snapshot with byte-identical content keeps the snapshot's provider, so
    // the human who built it stays on the released version (the audit's
    // VERSION_PROMOTED entry still names the promoting Actor). Different
    // content means the promoter owns what they pushed.
    let credited = if promoting && existing.as_ref().is_some_and(|e| e.content_hash == hash) {
        existing
            .as_ref()
            .map(|e| e.provided_by.clone())
            .unwrap_or_default()
    } else {
        username.unwrap_or("").to_string()
    };
    let record = UpsertSpecVersion {
        service_id: sid,
        api_type,
        version,
        stability,
        content,
        content_hash: &hash,
        provided_by: &credited,
        expected_prior_hash,
        now_iso: &now_iso(),
        endpoints: endpoints
            .into_iter()
            .map(|e| EndpointRecord {
                id: None,
                api_type,
                path: e.path,
                normalized_path: e.normalized_path,
                method: e.method,
                yaml_content: e.yaml_content,
                deprecated: e.deprecated,
            })
            .collect(),
    };
    let entry_id = match repo.upsert_spec_version(record).await {
        Ok(id) => id,
        // A CAS refusal deserves a remedy, not a bare "Conflict" — and an
        // accurate one: the guard has three arms (row released concurrently,
        // row overwritten, row vanished), so re-read to say which happened.
        Err(e) => {
            if matches!(e, crate::domain::ports::RepositoryError::Conflict)
                && expected_prior_hash.is_some()
            {
                // The refusal is already decided; the re-read only makes the
                // message accurate. If it fails — most likely under exactly the
                // write load that caused the conflict — degrade the message
                // rather than escalate the 409 into a 500.
                let message = match repo.find_spec_version(sid, api_type, version).await {
                    Ok(Some(entry)) if entry.stability == Stability::Ga => format!(
                        "version {} was released concurrently — nothing left to do",
                        version
                    ),
                    Ok(Some(_)) => {
                        "the snapshot changed while releasing — reload and promote again"
                            .to_string()
                    }
                    Ok(None) => format!(
                        "version {} was deleted while releasing — nothing left to promote",
                        version
                    ),
                    Err(_) => "the version moved while releasing — reload and retry".to_string(),
                };
                return Err(AppError::Conflict(message));
            }
            return Err(e.into());
        }
    };

    apply_contract_ops(repo, contract_ops).await?;

    // The write landed; stamp the trunk marker on the (possibly fresh) entry.
    if trunk {
        repo.touch_spec_version_trunk(entry_id, &now_iso()).await?;
    }
    // A tagged provide marks the member version within its branch (ADR-0005).
    if let Some(branch) = &tag_branch {
        repo.record_branch_member_version(branch.id, sid, api_type, version, &now_iso())
            .await?;
        count_branch_update(&branch.name);
    }

    // Promotion is a state change worth its own audit entry even when the
    // content is byte-identical to the snapshot it releases.
    if promoting && let Some(actor) = username {
        let details = format!(
            "Promoted {} {} of '{}' to GA — the number is permanently claimed",
            api_type.as_str(),
            version,
            producername
        );
        if let Err(e) = repo
            .insert_audit_log(
                actor,
                NewAuditLog {
                    action: "VERSION_PROMOTED",
                    details: &details,
                    service: Some(producername),
                    version: Some(&version.to_string()),
                    action_type: Some("WRITE"),
                    diff: None,
                    stream: None,
                    branch_id: None,
                },
            )
            .await
        {
            tracing::warn!(
                service = producername,
                "Could not record the promotion in the audit log: {}",
                e
            );
        }
    }

    // An overwrite by a different Actor is allowed (last writer wins) but
    // never silent: the previous provider is named in the audit trail.
    if let Some(previous) = overwriting
        && !promoting
        && let Some(actor) = username
        && !previous.is_empty()
        && previous != actor
    {
        let details = format!(
            "Snapshot {} {} of '{}' overwritten by '{}' (previously provided by '{}')",
            api_type.as_str(),
            version,
            producername,
            actor,
            previous
        );
        if let Err(e) = repo
            .insert_audit_log(
                actor,
                NewAuditLog {
                    action: "SNAPSHOT_OVERWRITTEN",
                    details: &details,
                    service: Some(producername),
                    version: Some(&version.to_string()),
                    action_type: Some("WRITE"),
                    diff: None,
                    stream: None,
                    branch_id: None,
                },
            )
            .await
        {
            tracing::warn!(
                service = producername,
                "Could not record the snapshot overwrite in the audit log: {}",
                e
            );
        }
    }

    tracing::info!(
        service = producername,
        version = %version,
        "Provided {:?} spec {} as {} (changes: +{} ~{} -{})",
        api_type,
        version,
        stability.as_str(),
        changes.inserts,
        changes.updates,
        changes.deletes
    );

    Ok(ProvideResponse {
        version,
        stability,
        content_hash: hash,
        changes,
        promoted: promoting,
    })
}

/// Apply planned channel-message-contract mutations.
async fn apply_contract_ops(
    repo: &impl SpecRepository,
    ops: Vec<ContractOp>,
) -> Result<(), AppError> {
    for op in ops {
        match op {
            ContractOp::Upsert(contract) => {
                repo.upsert_channel_message_contract(&contract).await?;
            }
            ContractOp::Delete {
                channel,
                message_name,
            } => {
                repo.delete_channel_message_contract(&channel, &message_name)
                    .await?;
            }
        }
    }
    Ok(())
}

enum ContractOp {
    Upsert(ChannelMessageContract),
    Delete {
        channel: String,
        message_name: String,
    },
}

/// Plan the channel-message-contract changes for a GA AsyncAPI provide.
///
/// For every named PUB message in the submitted document:
/// - no contract yet → insert, owned by this service;
/// - owned by this service (or by a service that no longer exists) → this
///   service (re)owns it, after the owner-widen payload compatibility check;
/// - owned by a *different* live service → accepted only if the payload schema
///   is semantically identical, otherwise rejected `409` naming the owner.
///
/// Messages this service currently owns but no longer provides are planned for
/// deletion. Performs no writes — it only reads and returns the planned ops.
async fn plan_channel_message_contracts(
    repo: &impl SpecRepository,
    service_id: i64,
    content: &str,
) -> Result<Vec<ContractOp>, AppError> {
    let messages = asyncapi::extract_pub_messages(content).map_err(AppError::BadRequest)?;

    let mut ops = Vec::new();
    let mut provided: HashSet<(String, String)> = HashSet::new();

    for msg in &messages {
        provided.insert((msg.channel.clone(), msg.message_name.clone()));

        let existing = repo
            .get_channel_message_contract(&msg.channel, &msg.message_name)
            .await?;

        let owner_alive = match &existing {
            Some(c) => repo
                .get_service_name_by_id(c.owner_service_id)
                .await?
                .is_some(),
            None => false,
        };

        match existing {
            None => ops.push(ContractOp::Upsert(new_contract(msg, service_id))),
            Some(_) if !owner_alive => ops.push(ContractOp::Upsert(new_contract(msg, service_id))),
            Some(contract) if contract.owner_service_id == service_id => {
                if let Err(reason) =
                    asyncapi::check_payload_compatible(&contract.payload_yaml, &msg.payload_yaml)
                {
                    return Err(AppError::BreakingChange(format!(
                        "Incompatible change to owned AsyncAPI message '{}' on channel '{}': {}",
                        msg.message_name, msg.channel, reason
                    )));
                }
                ops.push(ContractOp::Upsert(new_contract(msg, service_id)));
            }
            Some(contract) => {
                if !asyncapi::payloads_equal(&contract.payload_yaml, &msg.payload_yaml) {
                    let owner = repo
                        .get_service_name_by_id(contract.owner_service_id)
                        .await?
                        .unwrap_or_else(|| "another service".to_string());
                    return Err(AppError::Conflict(format!(
                        "AsyncAPI message '{}' on channel '{}' is owned by service '{}' with a different schema; align the schema or rename your message",
                        msg.message_name, msg.channel, owner
                    )));
                }
            }
        }
    }

    for contract in repo.list_channel_message_contracts().await? {
        if contract.owner_service_id == service_id
            && !provided.contains(&(contract.channel.clone(), contract.message_name.clone()))
        {
            ops.push(ContractOp::Delete {
                channel: contract.channel,
                message_name: contract.message_name,
            });
        }
    }

    Ok(ops)
}

fn new_contract(msg: &asyncapi::PubMessage, service_id: i64) -> ChannelMessageContract {
    ChannelMessageContract {
        channel: msg.channel.clone(),
        message_name: msg.message_name.clone(),
        owner_service_id: service_id,
        payload_yaml: msg.payload_yaml.clone(),
    }
}

/// An endpoint as shown in the UI: what was served and how that was decided.
/// Always answers — the caller never has to infer a state from an error.
#[derive(Debug, serde::Serialize)]
pub struct EndpointView {
    pub state: ResolutionState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<SemVer>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stability: Option<Stability>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub yaml: Option<String>,
    pub deprecated: bool,
}

pub async fn get_endpoint_yaml(
    repo: &impl SpecRepository,
    producername: &str,
    version: SemVer,
    api_type: ApiType,
    path: &str,
    method: &str,
) -> Result<EndpointView, AppError> {
    let unknown = EndpointView {
        state: ResolutionState::Unknown,
        version: None,
        stability: None,
        yaml: None,
        deprecated: false,
    };

    let Some(service_id) = repo.find_service(producername).await? else {
        return Ok(unknown);
    };
    let Some(entry) = repo
        .find_spec_version(service_id, api_type, version)
        .await?
    else {
        return Ok(unknown);
    };

    let method = method_for(api_type, method);
    match repo
        .find_endpoint(entry.id, api_type, path, &method)
        .await?
    {
        Some((_, yaml, deprecated)) => Ok(EndpointView {
            state: ResolutionState::Served,
            version: Some(version),
            stability: Some(entry.stability),
            yaml: Some(yaml),
            deprecated,
        }),
        None => Ok(EndpointView {
            state: ResolutionState::Absent,
            version: Some(version),
            stability: Some(entry.stability),
            yaml: None,
            deprecated: false,
        }),
    }
}

/// The endpoints of one version-line entry. Read path: resolve, never create.
#[instrument(skip_all)]
pub async fn list_producer_endpoints(
    repo: &impl SpecRepository,
    producername: &str,
    api_type: ApiType,
    version: SemVer,
) -> Result<Vec<EndpointRecord>, AppError> {
    let Some(service_id) = repo.find_service(producername).await? else {
        return Ok(Vec::new());
    };
    let Some(entry) = repo
        .find_spec_version(service_id, api_type, version)
        .await?
    else {
        return Ok(Vec::new());
    };
    Ok(repo.get_endpoints_for_version(entry.id).await?)
}

/// The full provided document of one version-line entry — stored verbatim on
/// provide, so this is exactly what the Producer submitted.
pub async fn get_full_spec(
    repo: &impl SpecRepository,
    producername: &str,
    api_type: ApiType,
    version: SemVer,
) -> Result<String, AppError> {
    let (_sid, entry) = resolve_pin(repo, producername, api_type, version).await?;
    repo.get_spec_content(entry.id).await?.ok_or_else(|| {
        AppError::NotFound(format!(
            "No stored document for {} version {} of '{}'",
            api_type.as_str(),
            version,
            producername
        ))
    })
}

/// Every version-line entry of a Producer, newest-first within each API type.
pub async fn list_versions(
    repo: &impl SpecRepository,
    producername: &str,
) -> Result<Vec<SpecVersionMeta>, AppError> {
    let Some(service_id) = repo.find_service(producername).await? else {
        return Ok(Vec::new());
    };
    let mut versions = repo.list_spec_versions(service_id).await?;
    versions.sort_by(|a, b| {
        a.api_type
            .as_str()
            .cmp(b.api_type.as_str())
            .then_with(|| b.version.cmp(&a.version))
    });
    Ok(versions)
}

/// Unified diff between two versions of a line, oldest side first. Snapshots
/// participate as a single entry ("what's coming" vs the last GA).
pub async fn diff_versions(
    repo: &impl SpecRepository,
    producername: &str,
    api_type: ApiType,
    from: SemVer,
    to: SemVer,
) -> Result<String, AppError> {
    let (_sid, from_entry) = resolve_pin(repo, producername, api_type, from).await?;
    let (_sid2, to_entry) = resolve_pin(repo, producername, api_type, to).await?;
    let from_content = repo
        .get_spec_content(from_entry.id)
        .await?
        .unwrap_or_default();
    let to_content = repo
        .get_spec_content(to_entry.id)
        .await?
        .unwrap_or_default();
    Ok(similar::TextDiff::from_lines(&from_content, &to_content)
        .unified_diff()
        .context_radius(3)
        .header(
            &format!("{} {}", producername, from),
            &format!("{} {}", producername, to),
        )
        .to_string())
}

/// One step of an endpoint's blame trail: the version that last changed it
/// and the (unverified) Author attribution recorded with that Provide.
#[derive(Debug, serde::Serialize)]
pub struct EndpointHistoryEntry {
    pub version: SemVer,
    pub stability: Stability,
    pub provided_by: String,
    pub updated_at: String,
    /// What this version stored for the endpoint; `None` when the version
    /// does not include it (the endpoint was absent or had been removed).
    pub yaml_content: Option<String>,
    /// True when this version changed the endpoint relative to the previous
    /// version of the line (including introducing or removing it).
    pub changed: bool,
}

/// Walk a version line semantically and report, per version, whether it
/// changed the endpoint — the "last changed in version X by Y" blame.
pub async fn get_endpoint_history(
    repo: &impl SpecRepository,
    producername: &str,
    api_type: ApiType,
    path: &str,
    method: &str,
) -> Result<Vec<EndpointHistoryEntry>, AppError> {
    let Some(service_id) = repo.find_service(producername).await? else {
        return Err(AppError::NotFound("Producer not found".to_string()));
    };
    let mut line: Vec<SpecVersionMeta> = repo
        .list_spec_versions(service_id)
        .await?
        .into_iter()
        .filter(|v| v.api_type == api_type)
        .collect();
    if line.is_empty() {
        return Err(AppError::NotFound("No versions".to_string()));
    }
    line.sort_by_key(|v| v.version);

    let method = method_for(api_type, method);
    let normalized = openapi::lookup_path(api_type, path);
    let mut entries = Vec::with_capacity(line.len());
    let mut previous: Option<String> = None;
    for meta in line {
        let yaml = repo
            .get_endpoints_for_version(meta.id)
            .await?
            .into_iter()
            .find(|e| e.normalized_path == normalized && e.method == method)
            .map(|e| e.yaml_content);
        let changed = yaml != previous;
        previous = yaml.clone();
        entries.push(EndpointHistoryEntry {
            version: meta.version,
            stability: meta.stability,
            provided_by: meta.provided_by,
            updated_at: meta.updated_at,
            yaml_content: yaml,
            changed,
        });
    }
    Ok(entries)
}

/// The most recent Provides instance-wide — the dashboard timeline.
#[derive(Debug, serde::Serialize)]
pub struct TimelineEntry {
    pub service: String,
    pub api_type: ApiType,
    pub version: SemVer,
    pub stability: Stability,
    pub provided_by: String,
    pub updated_at: String,
    pub endpoint_count: i64,
}

pub async fn get_provide_timeline(
    repo: &impl SpecRepository,
    limit: u32,
) -> Result<Vec<TimelineEntry>, AppError> {
    let mut all = repo.list_all_spec_versions().await?;
    all.sort_by(|a, b| b.1.updated_at.cmp(&a.1.updated_at));
    Ok(all
        .into_iter()
        .take(limit as usize)
        .map(|(service, meta, endpoint_count)| TimelineEntry {
            service,
            api_type: meta.api_type,
            version: meta.version,
            stability: meta.stability,
            provided_by: meta.provided_by,
            updated_at: meta.updated_at,
            endpoint_count,
        })
        .collect())
}

/// The free validator's verdict: exactly what a Provide of this document
/// would see, minus persistence. Always a verdict, never an error — a
/// validator that answers invalid input with HTTP errors makes every caller
/// parse two shapes.
#[derive(Debug, serde::Serialize)]
pub struct ValidationReport {
    pub valid: bool,
    pub api_type: ApiType,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<SemVer>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub endpoint_count: Option<usize>,
    /// The first entries of the split preview ("METHOD path"), so the caller
    /// sees what Sanshain would store.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub endpoints: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// How many split entries the validator previews.
const VALIDATE_PREVIEW_LIMIT: usize = 20;

/// Validate a pasted document against the exact provide-side pipeline:
/// version extraction (strict semver) and splitting. No producer context, no
/// version-line rules — those need a Provide.
pub fn validate_spec(api_type: ApiType, content: &str) -> ValidationReport {
    let version_result = match api_type {
        ApiType::OpenApi | ApiType::AsyncApi => openapi::extract_info_version(content),
        ApiType::Proto => crate::proto::extract_sanshain_version(content),
    };
    let split_result = parse_spec_endpoints(api_type, content, "validator");

    let (endpoint_count, endpoints) = match &split_result {
        Ok(split) => (
            Some(split.len()),
            split
                .iter()
                .take(VALIDATE_PREVIEW_LIMIT)
                .map(|e| format!("{} {}", e.method, e.path))
                .collect(),
        ),
        Err(_) => (None, Vec::new()),
    };

    // The version error leads: it is the mistake people actually make, and
    // the split preview still renders when only the version is wrong.
    let error = match (&version_result, &split_result) {
        (Err(e), _) => Some(e.clone()),
        (_, Err(AppError::BadRequest(e))) => Some(e.clone()),
        (_, Err(e)) => Some(e.to_string()),
        _ => None,
    };

    ValidationReport {
        valid: error.is_none(),
        api_type,
        version: version_result.ok(),
        endpoint_count,
        endpoints,
        error,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meta(version: &str, stability: Stability) -> SpecVersionMeta {
        SpecVersionMeta {
            id: 1,
            service_id: 1,
            api_type: ApiType::OpenApi,
            version: version.parse().unwrap(),
            stability,
            content_hash: "sha256:x".into(),
            trunk_provided_at: None,
            provided_by: "ci".into(),
            created_at: "2026-01-01T00:00:00Z".into(),
            updated_at: "2026-01-01T00:00:00Z".into(),
            last_required_at: None,
        }
    }

    #[test]
    fn propose_free_skips_taken_numbers() {
        let line = vec![
            meta("1.2.0", Stability::Ga),
            meta("1.3.0", Stability::Snapshot),
        ];
        assert_eq!(
            propose_free(&line, "1.3.0".parse().unwrap()).to_string(),
            "1.3.1"
        );
        assert_eq!(
            propose_free(&line, "2.0.0".parse().unwrap()).to_string(),
            "2.0.0"
        );
    }

    #[test]
    fn ga_baseline_is_highest_ga_strictly_below() {
        let line = vec![
            meta("1.0.0", Stability::Ga),
            meta("1.2.0", Stability::Ga),
            meta("1.3.0", Stability::Snapshot),
            meta("2.0.0", Stability::Ga),
        ];
        let base = ga_baseline(&line, "1.4.0".parse().unwrap()).unwrap();
        assert_eq!(base.version.to_string(), "1.2.0");
        assert!(ga_baseline(&line, "1.0.0".parse().unwrap()).is_none());
    }

    #[test]
    fn diff_endpoints_counts_all_three_kinds() {
        let old = vec![
            EndpointRecord {
                id: None,
                api_type: ApiType::OpenApi,
                path: "/kept".into(),
                normalized_path: "/kept".into(),
                method: "GET".into(),
                yaml_content: "a".into(),
                deprecated: false,
            },
            EndpointRecord {
                id: None,
                api_type: ApiType::OpenApi,
                path: "/gone".into(),
                normalized_path: "/gone".into(),
                method: "GET".into(),
                yaml_content: "b".into(),
                deprecated: false,
            },
        ];
        let new = vec![
            openapi::EndpointSpec {
                path: "/kept".into(),
                normalized_path: "/kept".into(),
                method: "GET".into(),
                yaml_content: "a-changed".into(),
                deprecated: false,
            },
            openapi::EndpointSpec {
                path: "/new".into(),
                normalized_path: "/new".into(),
                method: "GET".into(),
                yaml_content: "c".into(),
                deprecated: false,
            },
        ];
        let changes = diff_endpoints(&old, &new, ApiType::OpenApi);
        assert_eq!(changes.inserts, 1);
        assert_eq!(changes.updates, 1);
        assert_eq!(changes.deletes, 1);
    }

    #[test]
    fn classify_change_flags_breaking_as_major() {
        let old = "openapi: 3.0.3\ninfo:\n  title: T\n  version: 1.0.0\npaths:\n  /a:\n    get:\n      responses:\n        '200':\n          description: OK\n";
        let new = "openapi: 3.0.3\ninfo:\n  title: T\n  version: 1.0.1\npaths: {}\n";
        assert_eq!(
            classify_change(ApiType::OpenApi, old, new, 0),
            Impact::Major
        );
    }
}
