//! The Provide and Require use-cases of the version-line model (ADR-0003).
//!
//! A Provide carries a complete spec whose version is read from the document
//! itself; the caller declares its stability. A Require pins an exact version
//! and either gets exactly that or fails immediately — no fallback, no
//! waiting. All decisions live here; repositories persist what they are told.

use crate::asyncapi;
use crate::domain::models::*;
use crate::domain::permissions::{Actor, Permission};
use crate::domain::ports::{
    NewAuditLog, RecordDependencyParams, SpecRepository, UpsertSpecVersion,
};
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
    /// The resolved caller: the GA gate checks it for
    /// [`Permission::ReleaseGa`], and its username is the single identity used
    /// for audit attribution and the version's `provided_by` credit. `None`
    /// (no authenticated Actor) can never release.
    pub caller: Option<Actor>,
    /// Compare-and-set for promote-by-copy (#28): when `Some`, the write only
    /// lands if the stored entry still carries this content hash, so a
    /// snapshot overwritten between read and release answers 409 instead of
    /// silently going GA with stale bytes.
    pub expected_prior_hash: Option<&'a str>,
}

pub struct RequireEndpointParams<'a> {
    pub consumername: &'a str,
    pub producername: &'a str,
    pub version: SemVer,
    pub api_type: ApiType,
    pub path: &'a str,
    pub method: &'a str,
}

pub struct RequireBundleParams<'a> {
    pub consumername: &'a str,
    pub producername: &'a str,
    pub version: SemVer,
    pub api_type: ApiType,
    pub endpoints: &'a [(String, String)],
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

fn now_iso() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
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

fn parse_spec_endpoints(
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
            caller,
            // Release exactly what was read: if the snapshot is overwritten
            // between this read and the write, the CAS refuses (409) instead
            // of releasing stale bytes immutably.
            expected_prior_hash: Some(&entry.content_hash),
        },
    )
    .await
}

/// Whole-document backward-compatibility verdict for one API type.
pub fn check_compatibility(api_type: ApiType, old: &str, new: &str) -> Result<(), String> {
    match api_type {
        ApiType::OpenApi => openapi::check_backward_compatibility(old, new),
        ApiType::AsyncApi => asyncapi::check_backward_compatibility(old, new),
        ApiType::Proto => crate::proto::check_backward_compatibility(old, new),
    }
}

/// The smallest version at or above `candidate` that no entry of `line` uses.
///
/// Rejections propose a next version; proposing a number that is itself taken
/// would send the Producer straight into the next rejection.
fn propose_free(line: &[SpecVersionMeta], candidate: SemVer) -> SemVer {
    let taken: HashSet<SemVer> = line.iter().map(|v| v.version).collect();
    let mut proposal = candidate;
    while taken.contains(&proposal) {
        proposal = proposal.increment(Impact::Patch);
    }
    proposal
}

/// Classify the change between two documents for the bump proposal:
/// breaking → major, additive → minor, shape-identical → patch.
fn classify_change(
    api_type: ApiType,
    old_content: &str,
    new_content: &str,
    inserts: usize,
) -> Impact {
    if check_compatibility(api_type, old_content, new_content).is_err() {
        return Impact::Major;
    }
    if api_type == ApiType::OpenApi {
        // The OpenAPI analyzer distinguishes additive schema changes a pure
        // endpoint diff cannot see.
        return openapi::analyze_impact(old_content, new_content).max(if inserts > 0 {
            Impact::Minor
        } else {
            Impact::Patch
        });
    }
    if inserts > 0 {
        Impact::Minor
    } else {
        Impact::Patch
    }
}

/// The endpoint-set diff between what a version stored and what a Provide
/// submits — the `changes` counts of the response.
fn diff_endpoints(
    old: &[EndpointRecord],
    new: &[openapi::EndpointSpec],
    api_type: ApiType,
) -> ProvideChanges {
    let mut old_map: HashMap<(String, String), &EndpointRecord> = old
        .iter()
        .map(|e| ((e.normalized_path.clone(), e.method.clone()), e))
        .collect();
    let mut inserts = 0;
    let mut updates = 0;
    for endpoint in new {
        match old_map.remove(&(endpoint.normalized_path.clone(), endpoint.method.clone())) {
            Some(existing) => {
                if existing.yaml_content != endpoint.yaml_content
                    || existing.path != endpoint.path
                    || existing.deprecated != endpoint.deprecated
                {
                    updates += 1;
                }
            }
            None => inserts += 1,
        }
    }
    let deletes = old_map.len();
    let _ = api_type;
    ProvideChanges {
        inserts,
        updates,
        deletes,
    }
}

/// The GA entry the mislabel check compares against: the highest GA of the
/// line strictly below the incoming version.
fn ga_baseline(line: &[SpecVersionMeta], below: SemVer) -> Option<SpecVersionMeta> {
    line.iter()
        .filter(|v| v.stability == Stability::Ga && v.version < below)
        .max_by_key(|v| v.version)
        .cloned()
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
        caller,
        expected_prior_hash,
    } = params;
    // One identity: authorization and audit attribution both come from the
    // Actor, so they cannot drift apart.
    let username: Option<&str> = caller.as_ref().map(|a| a.username.as_str());

    let version = extract_spec_version(api_type, content)?;
    let hash = content_hash(content);
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
        // Telemetry only for Producers that exist: the producer name becomes a
        // Prometheus label and an audit row, and this refusal fires before
        // `ensure_service` — without the check, any authenticated non-releaser
        // could mint unbounded label cardinality out of made-up names.
        if repo.find_service(producername).await?.is_some() {
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
        }
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
        if entry.stability == Stability::Snapshot {
            repo.touch_spec_version_provided(entry.id, &now_iso())
                .await?;
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
            let message = if stability == Stability::Ga {
                format!(
                    "version {} of {} '{}' is GA and immutable, but the submitted content differs — did you forget to increment info's version? Publish as {} instead",
                    version,
                    api_type.as_str(),
                    producername,
                    proposed
                )
            } else {
                format!(
                    "version {} of {} '{}' is GA — a released number can never carry a snapshot again; bump your version to {}",
                    version,
                    api_type.as_str(),
                    producername,
                    proposed
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
    repo.upsert_spec_version(record).await.map_err(|e| {
        // A CAS refusal deserves a remedy, not a bare "Conflict": the caller
        // read a snapshot that has since been overwritten.
        if matches!(e, crate::domain::ports::RepositoryError::Conflict)
            && expected_prior_hash.is_some()
        {
            AppError::Conflict(
                "the snapshot changed while releasing — reload and promote again".to_string(),
            )
        } else {
            e.into()
        }
    })?;

    apply_contract_ops(repo, contract_ops).await?;

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

/// Resolve a Producer's version-line entry for a Pin, or fail the way the
/// contract promises: `404` when the version does not exist in either
/// stability. Never creates anything.
async fn resolve_pin(
    repo: &impl SpecRepository,
    producername: &str,
    api_type: ApiType,
    version: SemVer,
) -> Result<(i64, SpecVersionMeta), AppError> {
    let Some(service_id) = repo.find_service(producername).await? else {
        return Err(AppError::NotFound(format!(
            "Producer '{}' is unknown",
            producername
        )));
    };
    let Some(entry) = repo
        .find_spec_version(service_id, api_type, version)
        .await?
    else {
        return Err(AppError::NotFound(format!(
            "Producer '{}' has no {} version {} (in either stability) — a missing pinned version is a configuration error, nothing waits for it to appear",
            producername,
            api_type.as_str(),
            version
        )));
    };
    Ok((service_id, entry))
}

fn method_for(api_type: ApiType, method: &str) -> String {
    match api_type {
        ApiType::OpenApi | ApiType::AsyncApi => method.to_uppercase(),
        ApiType::Proto => method.to_string(),
    }
}

#[derive(Debug)]
pub struct RequireResponse {
    pub yaml: String,
    pub deprecated: bool,
    /// How this was resolved, surfaced to Consumers as `X-Sanshain-Resolution`.
    pub state: ResolutionState,
    /// The version that served the request — always the Pin.
    pub version: SemVer,
    /// The stability it was served from, surfaced as `X-Sanshain-Stability`.
    pub stability: Stability,
}

pub async fn require_endpoint(
    repo: &impl SpecRepository,
    params: RequireEndpointParams<'_>,
) -> Result<RequireResponse, AppError> {
    require_endpoint_inner(repo, params, false).await
}

pub async fn require_endpoint_dry_run(
    repo: &impl SpecRepository,
    params: RequireEndpointParams<'_>,
) -> Result<RequireResponse, AppError> {
    require_endpoint_inner(repo, params, true).await
}

async fn require_endpoint_inner(
    repo: &impl SpecRepository,
    params: RequireEndpointParams<'_>,
    dry_run: bool,
) -> Result<RequireResponse, AppError> {
    let RequireEndpointParams {
        consumername,
        producername,
        version,
        api_type,
        path,
        method,
    } = params;

    let (_sid, entry) = resolve_pin(repo, producername, api_type, version).await?;
    let method = method_for(api_type, method);

    let Some((_, yaml, deprecated)) = repo
        .find_endpoint(entry.id, api_type, path, &method)
        .await?
    else {
        // The provided spec is complete, so absence is a deliberate choice of
        // this version — a definitive no, distinct from an unknown version.
        return Err(AppError::Gone(format!(
            "{} version {} of '{}' does not include {} {}",
            api_type.as_str(),
            version,
            producername,
            method,
            path
        )));
    };

    if !dry_run {
        record_pins(
            repo,
            consumername,
            &entry,
            &[(path.to_string(), method.clone())],
        )
        .await?;
    }

    Ok(RequireResponse {
        yaml,
        deprecated,
        state: ResolutionState::Served,
        version,
        stability: entry.stability,
    })
}

pub async fn require_bundle(
    repo: &impl SpecRepository,
    params: RequireBundleParams<'_>,
) -> Result<RequireResponse, AppError> {
    require_bundle_inner(repo, params, false).await
}

pub async fn require_bundle_dry_run(
    repo: &impl SpecRepository,
    params: RequireBundleParams<'_>,
) -> Result<RequireResponse, AppError> {
    require_bundle_inner(repo, params, true).await
}

async fn require_bundle_inner(
    repo: &impl SpecRepository,
    params: RequireBundleParams<'_>,
    dry_run: bool,
) -> Result<RequireResponse, AppError> {
    let RequireBundleParams {
        consumername,
        producername,
        version,
        api_type,
        endpoints,
    } = params;

    if endpoints.is_empty() {
        return Err(AppError::BadRequest(
            "A bundle needs at least one endpoint".to_string(),
        ));
    }

    let (_sid, entry) = resolve_pin(repo, producername, api_type, version).await?;

    let normalized: Vec<(String, String)> = endpoints
        .iter()
        .map(|(p, m)| (p.clone(), method_for(api_type, m)))
        .collect();

    let found = repo
        .find_endpoints_bulk(entry.id, api_type, &normalized)
        .await?;

    let missing: Vec<String> = normalized
        .iter()
        .filter(|(p, m)| !found.contains_key(&(p.clone(), m.clone())))
        .map(|(p, m)| format!("{} {}", m, p))
        .collect();
    if !missing.is_empty() {
        return Err(AppError::Gone(format!(
            "{} version {} of '{}' does not include: {}",
            api_type.as_str(),
            version,
            producername,
            missing.join(", ")
        )));
    }

    let mut ordered: Vec<&(String, String)> = normalized.iter().collect();
    ordered.sort();
    let mut yamls = Vec::with_capacity(ordered.len());
    let mut any_deprecated = false;
    for key in ordered {
        if let Some((_, yaml, deprecated)) = found.get(key) {
            yamls.push(yaml.clone());
            any_deprecated |= *deprecated;
        }
    }

    let yaml = match api_type {
        ApiType::OpenApi => openapi::merge_endpoint_yamls(&yamls).map_err(AppError::Internal)?,
        _ => yamls.join("\n---\n"),
    };

    if !dry_run {
        record_pins(repo, consumername, &entry, &normalized).await?;
    }

    Ok(RequireResponse {
        yaml,
        deprecated: any_deprecated,
        state: ResolutionState::Served,
        version,
        stability: entry.stability,
    })
}

/// Record the Consumer's Pins — only ever after a successful resolution
/// (resolution never creates graph entities) — and touch the version's
/// last-required time so use-based snapshot expiry counts the require.
async fn record_pins(
    repo: &impl SpecRepository,
    consumername: &str,
    entry: &SpecVersionMeta,
    endpoints: &[(String, String)],
) -> Result<(), AppError> {
    let client_id = repo.ensure_client(consumername).await?;
    let normalized: Vec<String> = endpoints
        .iter()
        .map(|(path, _)| openapi::lookup_path(entry.api_type, path))
        .collect();
    let deps: Vec<RecordDependencyParams> = endpoints
        .iter()
        .zip(normalized.iter())
        .map(|((path, method), normalized_path)| RecordDependencyParams {
            client_id,
            spec_version_id: entry.id,
            api_type: entry.api_type,
            path,
            normalized_path,
            method,
        })
        .collect();
    repo.record_dependencies_bulk(deps).await?;
    repo.touch_spec_version_required(entry.id, &now_iso())
        .await?;
    Ok(())
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
