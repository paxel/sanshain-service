use crate::asyncapi;
use crate::domain::branch_pattern::branch_matches_any;
use crate::domain::models::*;
use crate::domain::ports::{
    NewAuditLog, RecordDependencyParams, RepositoryError, SpecRepository, UpdateEndpointParams,
};
use crate::openapi;
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};

use tracing::instrument;

pub struct RequireEndpointParams<'a> {
    pub consumername: &'a str,
    pub producername: &'a str,
    pub branch: &'a str,
    pub api_type: ApiType,
    pub path: &'a str,
    pub method: &'a str,
    pub timeout_secs: Option<u64>,
    /// Sticky hint (item #17): see `RequireQuery::source_protected_branch`.
    pub source_protected_branch: Option<&'a str>,
    /// One-shot override (item #17): see `RequireQuery::pull_from_branch`.
    pub pull_from_branch: Option<&'a str>,
}

pub struct RequireBundleParams<'a> {
    pub consumername: &'a str,
    pub producername: &'a str,
    pub branch: &'a str,
    pub api_type: ApiType,
    pub endpoints: &'a [(String, String)],
    pub timeout_secs: Option<u64>,
    /// Sticky hint (item #17): see `RequireQuery::source_protected_branch`.
    /// Honoured identically to the single-endpoint `/require` — a bundle that
    /// resolved lineage differently would hand a client the wrong branch's API.
    pub source_protected_branch: Option<&'a str>,
    /// One-shot override (item #17): see `RequireQuery::pull_from_branch`.
    pub pull_from_branch: Option<&'a str>,
}

#[instrument(skip_all)]
pub async fn provide_spec(
    repo: &impl SpecRepository,
    producername: &str,
    branch: &str,
    api_type: ApiType,
    content: &str,
    base_version: Option<String>,
    force: bool,
) -> Result<ProvideResponse, AppError> {
    provide_spec_inner(
        repo,
        ProvideInternalParams {
            producername,
            branch,
            api_type,
            content,
            dry_run: false,
            extra_tags: &[],
            base_version,
            force,
            username: None,
            source_protected_branch: None,
            author: None,
            waived: false,
        },
    )
    .await
}

pub async fn provide_spec_dry_run(
    repo: &impl SpecRepository,
    producername: &str,
    branch: &str,
    api_type: ApiType,
    content: &str,
    force: bool,
) -> Result<ProvideResponse, AppError> {
    provide_spec_inner(
        repo,
        ProvideInternalParams {
            producername,
            branch,
            api_type,
            content,
            dry_run: true,
            extra_tags: &[],
            base_version: None,
            force,
            username: None,
            source_protected_branch: None,
            author: None,
            waived: false,
        },
    )
    .await
}

/// Inputs describing the spec a service provides for a branch. Grouped so the
/// public provide entry points stay within a sane argument count.
pub struct ProvideSpecParams<'a> {
    pub producername: &'a str,
    pub branch: &'a str,
    pub api_type: ApiType,
    pub content: &'a str,
    pub base_version: Option<String>,
    pub force: bool,
    /// Sticky hint (item #17): see `RequireQuery::source_protected_branch`.
    pub source_protected_branch: Option<&'a str>,
    /// Client-supplied blame override (item #15): see `ProvideRequest::author`.
    pub author: Option<&'a str>,
}

pub async fn provide_spec_with_tags(
    repo: &impl SpecRepository,
    params: ProvideSpecParams<'_>,
    tags: &[String],
) -> Result<ProvideResponse, AppError> {
    provide_spec_inner(
        repo,
        ProvideInternalParams {
            producername: params.producername,
            branch: params.branch,
            api_type: params.api_type,
            content: params.content,
            dry_run: false,
            extra_tags: tags,
            base_version: params.base_version,
            force: params.force,
            username: None,
            source_protected_branch: params.source_protected_branch,
            author: params.author,
            waived: false,
        },
    )
    .await
}

pub async fn provide_spec_with_actor(
    repo: &impl SpecRepository,
    params: ProvideSpecParams<'_>,
    username: Option<&str>,
) -> Result<ProvideResponse, AppError> {
    provide_spec_inner(
        repo,
        ProvideInternalParams {
            producername: params.producername,
            branch: params.branch,
            api_type: params.api_type,
            content: params.content,
            dry_run: false,
            extra_tags: &[],
            base_version: params.base_version,
            force: params.force,
            username,
            source_protected_branch: params.source_protected_branch,
            author: params.author,
            waived: false,
        },
    )
    .await
}

struct ProvideInternalParams<'a> {
    pub producername: &'a str,
    pub branch: &'a str,
    pub api_type: ApiType,
    pub content: &'a str,
    pub dry_run: bool,
    pub extra_tags: &'a [String],
    pub base_version: Option<String>,
    pub force: bool,
    /// The real authenticated actor — always used for the audit log.
    pub username: Option<&'a str>,
    pub source_protected_branch: Option<&'a str>,
    /// Client-supplied blame override (item #15) — used for version-history
    /// attribution only, in place of `username`, when present. Never affects
    /// the audit log.
    pub author: Option<&'a str>,
    /// Set when an approver is applying a previously refused Provide. Suspends
    /// the refusals for this call only.
    pub waived: bool,
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

fn parse_spec_endpoints(
    api_type: ApiType,
    content: &str,
    producername: &str,
    branch: &str,
) -> Result<Vec<openapi::EndpointSpec>, AppError> {
    match api_type {
        ApiType::OpenApi => openapi::split_openapi(content).map_err(|e| {
            tracing::warn!(
                "Failed to split OpenAPI for service '{}' branch '{}' ({}): {}",
                producername,
                branch,
                spec_content_fingerprint(content),
                e
            );
            AppError::BadRequest(e)
        }),
        ApiType::AsyncApi => {
            let all_specs = crate::asyncapi::split_asyncapi(content).map_err(|e| {
                tracing::warn!(
                    "Failed to split AsyncAPI for service '{}' branch '{}' ({}): {}",
                    producername,
                    branch,
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
                    "Skipping {} SUB operation(s) for service '{}' branch '{}': subscribe channels {:?} are not stored via /provide. Declare them as 'requires' in sanshain.yaml instead.",
                    sub_count,
                    producername,
                    branch,
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
                    "Failed to split Proto for service '{}' branch '{}' ({}): {}",
                    producername,
                    branch,
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

#[instrument(skip_all)]
async fn provide_spec_inner(
    repo: &impl SpecRepository,
    params: ProvideInternalParams<'_>,
) -> Result<ProvideResponse, AppError> {
    let ProvideInternalParams {
        producername,
        branch,
        api_type,
        content,
        dry_run,
        extra_tags,
        base_version,
        force,
        username,
        source_protected_branch,
        author,
        waived,
    } = params;
    // Blame attribution (item #15): a client-supplied author overrides the real
    // authenticated actor for version-history display only. The audit log always
    // uses the real actor (`username`) and is untouched by this — for a
    // successful provide it is written by the presentation-layer handler, for a
    // protected-branch refusal by `quarantine` below, which the handler never
    // reaches because the refusal returns `Err`.
    let blame_username = author.or(username);

    tracing::debug!(
        "Providing {:?} for service '{}' branch '{}' (dry_run: {}, base_version: {:?})",
        api_type,
        producername,
        branch,
        dry_run,
        base_version
    );

    let content_hash = {
        let mut hasher = Sha256::new();
        hasher.update(content.as_bytes());
        format!("sha256:{}", hex::encode(hasher.finalize()))
    };

    let endpoints = parse_spec_endpoints(api_type, content, producername, branch)?;

    tracing::debug!(
        "Successfully split {:?} into {} endpoints for service '{}'",
        api_type,
        endpoints.len(),
        producername
    );

    let (sid, bid) = if dry_run {
        match repo.find_service(producername).await? {
            Some(sid) => match repo.find_branch(sid, branch).await? {
                Some(bid) => (sid, bid),
                None => (sid, 0),
            },
            None => (0, 0),
        }
    } else {
        let sid = repo.ensure_service(producername).await?;
        let bid = repo.ensure_branch(sid, branch).await?;

        // Auto-tag based on API type
        let auto_tag = match api_type {
            ApiType::AsyncApi => Some("messaging".to_string()),
            ApiType::Proto => Some("grpc".to_string()),
            ApiType::OpenApi => None,
        };
        let mut all_tags: Vec<String> = extra_tags.to_vec();
        if let Some(tag) = auto_tag
            && !all_tags.contains(&tag)
        {
            all_tags.push(tag);
        }
        if !all_tags.is_empty() {
            repo.add_service_tags(sid, &all_tags).await?;
        }

        (sid, bid)
    };

    if !dry_run {
        apply_source_protected_branch_hint(
            repo,
            bid,
            producername,
            branch,
            source_protected_branch,
        )
        .await?;
    }

    // Only the version is read. The stored fingerprint is written but never
    // compared against: one record serves a producer and branch across all API
    // types, so it cannot tell whether *this* type's content changed.
    //
    // `None` means no version has been established for this producer and branch
    // yet, which is distinct from a version of 0.0.0 and is what the no-op guard
    // below keys on.
    let existing_version = if sid != 0 && bid != 0 {
        repo.get_spec_version(sid, bid).await?.map(|(v, _)| v)
    } else {
        None
    };
    let current_version = existing_version.unwrap_or_default();

    if let Some(base_v_str) = base_version {
        let base_v = base_v_str
            .parse::<SemVer>()
            .map_err(|e| AppError::BadRequest(format!("Invalid base_version: {}", e)))?;
        if base_v != current_version {
            return Err(AppError::Conflict(format!(
                "Outdated spec version: your base version is {}, but current version is {}. Pull latest changes.",
                base_v, current_version
            )));
        }
    }

    let is_protected = repo.is_branch_protected(branch).await?;

    // Onboarding suspends the gatekeeping, not the record-keeping. `is_protected`
    // still drives soft deletes and version history below; `enforce_protection`
    // is what the five refusals consult, so a Producer whose API is not yet
    // stable keeps its history instead of having its branch deleted by hand.
    let onboarding = if sid != 0 {
        repo.is_producer_onboarding(sid).await?
    } else {
        false
    };
    // `waived` is set only when an approver applies a Provide they have read and
    // accepted. It joins onboarding at the same seam rather than getting its own,
    // so there is one place that decides whether the refusals apply.
    let enforce_protection = is_protected && !onboarding && !waived;

    let refusal = RefusalContext {
        service_id: sid,
        actor: username,
        api_type,
        producername,
        branch,
        content,
        dry_run,
    };

    if force && enforce_protection {
        return Err(AppError::BadRequest(
            "Force mode is not allowed on protected branches.".to_string(),
        ));
    }

    let existing = repo.get_endpoints_for_branch(bid).await?;
    let existing_yamls: Vec<String> = existing
        .iter()
        .filter(|e| e.api_type == ApiType::OpenApi)
        .map(|e| e.yaml_content.clone())
        .collect();

    let old_full_yaml = if !existing_yamls.is_empty() {
        Some(openapi::merge_endpoint_yamls(&existing_yamls).map_err(|e| {
            AppError::Internal(format!(
                "Failed to merge existing endpoints for impact analysis: {}",
                e
            ))
        })?)
    } else {
        None
    };

    // The check runs whenever the branch is protected, even in onboarding: the
    // refusal is suspended, but the *reason* is what the audit entry carries, and
    // it only exists if the comparison is actually made.
    if let (Some(old_yaml), true) = (&old_full_yaml, is_protected && api_type == ApiType::OpenApi) {
        if let Err(reason) = openapi::check_backward_compatibility(old_yaml, content) {
            if enforce_protection {
                return Err(quarantine(repo, &refusal, &reason).await);
            }
            if !dry_run {
                record_accepted_breaking(repo, &refusal, &reason).await;
            }
        } else {
            tracing::info!(
                "Spec update is backward-compatible for protected branch '{}'",
                branch
            );
        }
    }

    let mut existing_map: HashMap<(ApiType, String, String), (String, String, bool)> = existing
        .into_iter()
        .map(|e| {
            (
                (e.api_type, e.normalized_path, e.method),
                (e.path, e.yaml_content, e.deprecated),
            )
        })
        .collect();

    let mut changes = Vec::new();
    let mut inserts = 0;
    let mut updates = 0;
    // Aggregated SemVer impact for AsyncAPI provides. OpenAPI additionally
    // consults the merged whole-spec `analyze_impact` below; every API type
    // takes at least the impact implied by the endpoint diff.
    let mut async_impact = Impact::None;

    for endpoint in endpoints {
        let key = (
            api_type,
            endpoint.normalized_path.clone(),
            endpoint.method.clone(),
        );
        if let Some((old_path, existing_yaml, _)) = existing_map.remove(&key) {
            if existing_yaml != endpoint.yaml_content || old_path != endpoint.path {
                // OpenAPI is covered by the merged whole-spec check above;
                // AsyncAPI/proto content changes are checked per endpoint here.
                if is_protected
                    && api_type != ApiType::OpenApi
                    && existing_yaml != endpoint.yaml_content
                    && let Err(reason) =
                        check_compatibility(api_type, &existing_yaml, &endpoint.yaml_content)
                {
                    if enforce_protection {
                        return Err(quarantine(repo, &refusal, &reason).await);
                    }
                    if !dry_run {
                        record_accepted_breaking(repo, &refusal, &reason).await;
                    }
                }
                if api_type == ApiType::AsyncApi {
                    async_impact = async_impact.max(asyncapi::analyze_impact(
                        &existing_yaml,
                        &endpoint.yaml_content,
                    ));
                }
                changes.push(SpecChange::Update {
                    api_type,
                    path: endpoint.path,
                    normalized_path: endpoint.normalized_path,
                    method: endpoint.method,
                    yaml_content: endpoint.yaml_content,
                    deprecated: endpoint.deprecated,
                    external: false,
                });
                updates += 1;
            }
        } else {
            if enforce_protection
                && repo
                    .is_endpoint_deleted(bid, api_type, &endpoint.path, &endpoint.method)
                    .await?
            {
                return Err(AppError::Conflict(format!(
                    "Re-introduction of deleted {:?} endpoint {} {} on protected branch '{}'",
                    api_type, endpoint.method, endpoint.path, branch
                )));
            }
            if api_type == ApiType::AsyncApi {
                // A newly provided channel/operation is an additive change.
                async_impact = async_impact.max(Impact::Minor);
            }
            changes.push(SpecChange::Insert {
                api_type,
                path: endpoint.path,
                normalized_path: endpoint.normalized_path,
                method: endpoint.method,
                yaml_content: endpoint.yaml_content,
                deprecated: endpoint.deprecated,
                external: false,
            });
            inserts += 1;
        }
    }

    let mut deletes = 0;
    for ((old_api_type, _norm_path, method), (path, _, was_deprecated)) in existing_map {
        if old_api_type != api_type {
            continue;
        }

        if is_protected && !dry_run && !was_deprecated {
            let reason = format!(
                "Removing non-deprecated endpoint {} {} is a breaking change on a protected branch; mark it deprecated first",
                method, path
            );
            if enforce_protection {
                return Err(quarantine(repo, &refusal, &reason).await);
            }
            record_accepted_breaking(repo, &refusal, &reason).await;
        }

        if api_type == ApiType::AsyncApi {
            // Removing a provided channel/operation is a breaking change.
            async_impact = async_impact.max(Impact::Major);
        }
        changes.push(SpecChange::Delete {
            api_type,
            path,
            method,
            soft_delete: is_protected,
        });
        deletes += 1;
    }

    // Item #20: reconcile message-level channel contracts for AsyncAPI provides.
    // Runs on both dry-run and real provides so cross-service conflicts are
    // reported by dry-run too; writes are applied only after commit below.
    // Placed after the endpoint delete loop so protected-branch removal
    // rejections have already fired before any contract mutation is planned.
    let contract_ops = if api_type == ApiType::AsyncApi {
        plan_channel_message_contracts(repo, branch, sid, content).await?
    } else {
        Vec::new()
    };

    if dry_run {
        return Ok(ProvideResponse {
            version: current_version,
            content_hash,
            changes: ProvideChanges {
                inserts,
                updates,
                deletes,
            },
        });
    }

    // The counts alone decide whether anything changed. The stored fingerprint
    // is deliberately not consulted: one version record serves a producer and
    // branch across *all* API types, so on a producer that provides both
    // OpenAPI and AsyncAPI each provide would compare its own hash against the
    // other type's, never match, and fall through here forever.
    //
    // `existing_version.is_some()` keeps the *first* provide out of this branch.
    // A spec with no endpoints at all (`paths: {}`) produces no diff, and must
    // still establish 1.0.0 rather than report a version that was never stored.
    if inserts == 0 && updates == 0 && deletes == 0 && existing_version.is_some() {
        // Channel-message contracts are still reconciled. The endpoint diff
        // being empty does not mean the contracts are settled: ownership moves
        // when the owning producer is deleted, and a contract can be missing
        // for endpoints that already exist. Those upserts are idempotent, so
        // running them here costs nothing and skipping them would strand
        // ownership on a producer that only ever re-provides unchanged specs.
        apply_contract_ops(repo, branch, contract_ops).await?;

        return Ok(ProvideResponse {
            version: current_version,
            content_hash,
            changes: ProvideChanges {
                inserts,
                updates,
                deletes,
            },
        });
    }

    repo.apply_spec_changes(bid, changes, is_protected, blame_username, Some(branch))
        .await?;

    // Applied after the endpoint changes commit (endpoints are the source of truth).
    apply_contract_ops(repo, branch, contract_ops).await?;

    // Impact implied by the endpoint diff alone.
    //
    // This is what catches a modification the parsed API surface does not
    // distinguish — a changed description or example, say. `analyze_impact`
    // cannot see those: it compares parsed documents, and its "old" side is a
    // lossy reconstruction rather than what was provided last time. The diff is
    // fragment-against-fragment and is the comparison that holds.
    //
    // Note this is *not* simply the negation of the guard above: the first
    // provide for a producer and branch reaches here with an empty diff, which
    // is why the `None` arm exists.
    let diff_impact = if inserts > 0 {
        // A new endpoint is additive. `has_additions` normally reports this,
        // but it only runs when there are already OpenAPI endpoints to
        // reconstruct an "old" document from — not, for instance, on a branch
        // that so far holds only AsyncAPI endpoints.
        Impact::Minor
    } else if updates > 0 || deletes > 0 {
        Impact::Patch
    } else {
        Impact::None
    };

    let impact = if api_type == ApiType::AsyncApi {
        async_impact.max(diff_impact)
    } else if let Some(old_yaml) = old_full_yaml {
        openapi::analyze_impact(&old_yaml, content).max(diff_impact)
    } else {
        diff_impact
    };

    // The Producer fixed it themselves, so whatever was being held for this key
    // is dead. Leaving it would let an approver later apply a stale submission
    // over the top of the change that resolved the problem — a silent revert,
    // invisible until a Consumer noticed an endpoint had come back.
    if !dry_run && sid != 0 {
        repo.clear_pending_spec(sid, branch, api_type).await?;
    }

    let new_version = repo
        .increment_spec_version(sid, bid, &content_hash, impact)
        .await?;

    tracing::info!(
        service = producername,
        branch = branch,
        "Provided {:?} spec (version {}, changes: +{} ~{} -{})",
        api_type,
        new_version,
        inserts,
        updates,
        deletes
    );

    Ok(ProvideResponse {
        version: new_version,
        content_hash,
        changes: ProvideChanges {
            inserts,
            updates,
            deletes,
        },
    })
}

/// Applies a caller-supplied `source_protected_branch` hint (item #17) to a
/// branch: if the branch has no stored value yet, persists the hint (first
/// write wins). If it already has a value, the stored value always wins —
/// but if the supplied hint *differs* from what's stored, the mismatch is
/// logged (service/branch-tagged, reusing the observability logging added for
/// item #7) so a persistent disagreement between what a caller believes and
/// what's recorded stays visible rather than being silently swallowed. Only
/// an admin can change an already-set value (see `admin_set_source_protected_branch`).
/// A no-op when no hint was supplied.
async fn apply_source_protected_branch_hint(
    repo: &impl SpecRepository,
    branch_id: i64,
    producername: &str,
    branch: &str,
    hint: Option<&str>,
) -> Result<(), AppError> {
    let Some(hint) = hint else {
        return Ok(());
    };
    let newly_set = repo
        .set_source_protected_branch_if_unset(branch_id, hint)
        .await?;
    if newly_set {
        return Ok(());
    }
    if let Some(stored) = repo.get_source_protected_branch(branch_id).await?
        && stored != hint
    {
        tracing::warn!(
            service = producername,
            branch = branch,
            "source_protected_branch mismatch: caller supplied '{}' but branch already has '{}' recorded; keeping the stored value",
            hint,
            stored
        );
    }
    Ok(())
}

/// A planned mutation of the message-level channel-contract store (item #20),
/// computed during a provide and applied only after the endpoint changes commit.
enum ContractOp {
    Upsert(ChannelMessageContract),
    Delete {
        channel: String,
        message_name: String,
    },
}

/// The parts of a Provide a refusal needs, fixed for the whole call.
///
/// Grouped so `quarantine` stays within a sane argument count while still
/// having everything it needs to hold the submission — in particular the
/// content, which is the thing the old refusal path threw away.
struct RefusalContext<'a> {
    service_id: i64,
    actor: Option<&'a str>,
    api_type: ApiType,
    producername: &'a str,
    branch: &'a str,
    content: &'a str,
    dry_run: bool,
}

impl RefusalContext<'_> {
    fn actor(&self) -> &str {
        // Matches the fallback the presentation layer uses for an
        // unauthenticated caller in dev mode.
        self.actor.unwrap_or("DevMode/Anonymous")
    }
}

/// Hold a refused Provide for review and build the error the caller sees.
///
/// Refusal used to discard the submission, leaving only an audit line saying
/// that *something* was rejected. Nothing retained what was actually submitted,
/// so there was no way for anyone to look at it and decide it was fine. The
/// submission is now kept — one entry per Producer, branch and API type, latest
/// replacing previous — and the caller is told which entry theirs became.
///
/// Still a `409`: the spec is not live and no Consumer can resolve it, so a
/// build must stay red. A green build for a spec nobody can see would be the
/// worst outcome.
///
/// A dry run asks whether a Provide *would* be refused. Nothing was submitted,
/// so nothing is held and nothing is audited.
async fn quarantine(
    repo: &impl SpecRepository,
    ctx: &RefusalContext<'_>,
    reason: &str,
) -> AppError {
    tracing::warn!(
        service = ctx.producername,
        branch = ctx.branch,
        "Held {:?} spec for service '{}' branch '{}': breaking changes: {}",
        ctx.api_type,
        ctx.producername,
        ctx.branch,
        reason
    );

    let message = format!(
        "Breaking changes detected on protected branch '{}' of service '{}': {}",
        ctx.branch, ctx.producername, reason
    );

    if ctx.dry_run || ctx.service_id == 0 {
        return AppError::BreakingChange(message);
    }

    let actor = ctx.actor();
    let pending_id = match repo
        .upsert_pending_spec(
            ctx.service_id,
            ctx.branch,
            ctx.api_type,
            ctx.content,
            reason,
            actor,
        )
        .await
    {
        Ok(id) => id,
        Err(e) => {
            // Failing to hold the submission must not turn a clean refusal into
            // a 500 — the caller's build fails either way, and the refusal is
            // the more useful answer.
            tracing::error!(
                service = ctx.producername,
                branch = ctx.branch,
                "Could not hold the refused spec for review: {}",
                e
            );
            return AppError::BreakingChange(message);
        }
    };

    let details = format!(
        "Held {:?} spec for service '{}' on protected branch '{}' as #{} pending review: {}",
        ctx.api_type, ctx.producername, ctx.branch, pending_id, reason
    );
    if let Err(e) = repo
        .insert_audit_log(
            actor,
            NewAuditLog {
                action: "QUARANTINED_SPEC",
                details: &details,
                service: Some(ctx.producername),
                branch: Some(ctx.branch),
                // Keeps the existing "Rejected (Blocked)" timeline filter
                // working: a held Provide is still a Provide that did not land.
                action_type: Some("REJECT"),
                diff: None,
            },
        )
        .await
    {
        tracing::warn!(
            service = ctx.producername,
            branch = ctx.branch,
            "Could not record the quarantine in the audit log: {}",
            e
        );
    }

    AppError::Quarantined {
        message,
        pending_id,
    }
}

/// Record a breaking change that onboarding let through.
///
/// Mirrors [`quarantine`] deliberately: the same reason string, so the audit log
/// reads as a record of what each Producer broke rather than a separate
/// vocabulary. Best-effort — losing the entry must not turn an accepted Provide
/// into a 500.
async fn record_accepted_breaking(
    repo: &impl SpecRepository,
    ctx: &RefusalContext<'_>,
    reason: &str,
) {
    let details = format!(
        "Accepted {:?} spec for service '{}' on protected branch '{}' while onboarding: {}",
        ctx.api_type, ctx.producername, ctx.branch, reason
    );

    if let Err(e) = repo
        .insert_audit_log(
            ctx.actor(),
            NewAuditLog {
                action: "ACCEPTED_BREAKING",
                details: &details,
                service: Some(ctx.producername),
                branch: Some(ctx.branch),
                action_type: Some("ACCEPT"),
                diff: None,
            },
        )
        .await
    {
        tracing::warn!(
            service = ctx.producername,
            branch = ctx.branch,
            "Could not record the accepted breaking change in the audit log: {}",
            e
        );
    }
}

/// Apply planned channel-message-contract mutations.
///
/// Runs on both the changed and the unchanged path, so contract reconciliation
/// does not depend on whether the endpoint diff happened to find anything.
async fn apply_contract_ops(
    repo: &impl SpecRepository,
    branch: &str,
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
                repo.delete_channel_message_contract(branch, &channel, &message_name)
                    .await?;
            }
        }
    }
    Ok(())
}

/// Plan the channel-message-contract changes for an AsyncAPI provide.
///
/// For every named PUB message in the submitted document:
/// - no contract yet → insert, owned by this service;
/// - owned by this service (or by a service that no longer exists) → this
///   service (re)owns it, after the owner-widen payload compatibility check;
/// - owned by a *different* live service → accepted only if the payload schema
///   is semantically identical, otherwise rejected `409` naming the owner.
///
/// Messages this service currently owns on the branch but no longer provides
/// are planned for deletion. Protected-branch removal of a non-deprecated
/// message is already rejected by the per-endpoint compatibility check before
/// this runs, so a delete reaching here is always permitted.
///
/// Enforced identically on all branches; `force` does not bypass it. Performs no
/// writes — it only reads and returns the planned [`ContractOp`]s (or an error).
async fn plan_channel_message_contracts(
    repo: &impl SpecRepository,
    branch: &str,
    service_id: i64,
    content: &str,
) -> Result<Vec<ContractOp>, AppError> {
    let messages = asyncapi::extract_pub_messages(content).map_err(AppError::BadRequest)?;

    let mut ops = Vec::new();
    let mut provided: HashSet<(String, String)> = HashSet::new();

    for msg in &messages {
        provided.insert((msg.channel.clone(), msg.message_name.clone()));

        let existing = repo
            .get_channel_message_contract(branch, &msg.channel, &msg.message_name)
            .await?;

        let owner_alive = match &existing {
            Some(c) => repo
                .get_service_name_by_id(c.owner_service_id)
                .await?
                .is_some(),
            None => false,
        };

        match existing {
            // Unowned (new) or orphaned by a deleted owner → this service owns it.
            None => ops.push(ContractOp::Upsert(new_contract(msg, branch, service_id))),
            Some(_) if !owner_alive => {
                ops.push(ContractOp::Upsert(new_contract(msg, branch, service_id)))
            }
            Some(contract) if contract.owner_service_id == service_id => {
                // The owner may widen its own message, but not break it.
                if let Err(reason) =
                    asyncapi::check_payload_compatible(&contract.payload_yaml, &msg.payload_yaml)
                {
                    return Err(AppError::BreakingChange(format!(
                        "Incompatible change to owned AsyncAPI message '{}' on channel '{}': {}",
                        msg.message_name, msg.channel, reason
                    )));
                }
                ops.push(ContractOp::Upsert(new_contract(msg, branch, service_id)));
            }
            Some(contract) => {
                // A different live service owns this message: co-publishing is
                // allowed only with a byte-for-byte identical payload schema.
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
                // Identical schema → accepted, ownership unchanged, no write.
            }
        }
    }

    // Messages this service owns on the branch but no longer provides are dropped.
    for contract in repo.list_channel_message_contracts(branch).await? {
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

fn new_contract(
    msg: &asyncapi::PubMessage,
    branch: &str,
    service_id: i64,
) -> ChannelMessageContract {
    ChannelMessageContract {
        branch_name: branch.to_string(),
        channel: msg.channel.clone(),
        message_name: msg.message_name.clone(),
        owner_service_id: service_id,
        payload_yaml: msg.payload_yaml.clone(),
    }
}

/// An endpoint as shown in the UI: what was served, where it came from, and how
/// that was decided. Always answers — the caller never has to infer a state from
/// an error, which is what forced the UI to guess before.
#[derive(Debug, serde::Serialize)]
pub struct EndpointView {
    pub state: ResolutionState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub served_branch: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<SemVer>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub yaml: Option<String>,
    pub deprecated: bool,
    pub external: bool,
}

pub async fn get_endpoint_yaml(
    repo: &impl SpecRepository,
    producername: &str,
    branch: &str,
    api_type: ApiType,
    path: &str,
    method: &str,
) -> Result<EndpointView, AppError> {
    let unknown = || EndpointView {
        state: ResolutionState::Unknown,
        served_branch: None,
        version: None,
        yaml: None,
        deprecated: false,
        external: false,
    };

    // Read path: resolve, never create. An unknown producer has no endpoint,
    // which is `Unknown` rather than an error — the view always gets an answer.
    let Some(service_id) = repo.find_service(producername).await? else {
        return Ok(unknown());
    };
    let method_to_use = match api_type {
        ApiType::OpenApi | ApiType::AsyncApi => method.to_uppercase(),
        ApiType::Proto => method.to_string(),
    };

    let resolution = resolve_endpoint(
        repo,
        BranchLookup {
            service_id,
            producername,
            branch,
            api_type,
        },
        path,
        &method_to_use,
        &mut None,
    )
    .await?;

    let Some(endpoint) = resolution.endpoint else {
        return Ok(EndpointView {
            state: resolution.state,
            ..unknown()
        });
    };

    // The version of the branch that actually served it, not the one requested.
    let version = match &resolution.served_branch {
        Some(served) => match repo.find_branch(service_id, served).await? {
            Some(bid) => repo
                .get_spec_version(service_id, bid)
                .await?
                .map(|(v, _)| v),
            None => None,
        },
        None => None,
    };

    Ok(EndpointView {
        state: resolution.state,
        served_branch: resolution.served_branch,
        version,
        yaml: Some(endpoint.yaml_content),
        deprecated: endpoint.deprecated,
        external: endpoint.external,
    })
}

#[instrument(skip_all)]
pub async fn list_producer_endpoints(
    repo: &impl SpecRepository,
    producername: &str,
    branch: &str,
) -> Result<Vec<EndpointRecord>, AppError> {
    // Read path: resolve, never create. This previously called
    // `ensure_service`/`ensure_branch`, so merely browsing a mistyped or stale
    // branch URL minted a permanent empty branch that then showed up in listings
    // and cleanup. An unknown service or branch serves nothing, which is the
    // same empty list callers already handled.
    let Some(service_id) = repo.find_service(producername).await? else {
        return Ok(Vec::new());
    };
    let Some(branch_id) = repo.find_branch(service_id, branch).await? else {
        return Ok(Vec::new());
    };
    Ok(repo.get_endpoints_for_branch(branch_id).await?)
}

/// Reassemble the full spec for a service/branch from its stored per-endpoint
/// specs. OpenAPI endpoints are merged into one document; AsyncAPI/proto are
/// concatenated (matching how `require_bundle` assembles them). Endpoints are
/// ordered deterministically so the output is stable.
pub async fn get_full_spec(
    repo: &impl SpecRepository,
    producername: &str,
    branch: &str,
    api_type: ApiType,
) -> Result<String, AppError> {
    let mut endpoints: Vec<EndpointRecord> = list_producer_endpoints(repo, producername, branch)
        .await?
        .into_iter()
        .filter(|e| e.api_type == api_type)
        .collect();
    if endpoints.is_empty() {
        return Err(AppError::NotFound(format!(
            "No {} endpoints for service '{}' on branch '{}'",
            api_type.as_str(),
            producername,
            branch
        )));
    }
    endpoints.sort_by(|a, b| {
        (a.path.as_str(), a.method.as_str()).cmp(&(b.path.as_str(), b.method.as_str()))
    });
    let yamls: Vec<String> = endpoints.into_iter().map(|e| e.yaml_content).collect();
    match api_type {
        ApiType::OpenApi => openapi::merge_endpoint_yamls(&yamls).map_err(AppError::Internal),
        _ => Ok(yamls.join("\n---\n")),
    }
}

pub async fn get_endpoint_version_history(
    repo: &impl SpecRepository,
    producername: &str,
    branch: &str,
    api_type: ApiType,
    path: &str,
    method: &str,
) -> Result<Vec<EndpointVersion>, AppError> {
    // Read path: resolve, never create. An unknown service has no history,
    // which is the same 404 this produced before creating the row.
    let Some(service_id) = repo.find_service(producername).await? else {
        return Err(AppError::NotFound("Endpoint not found".to_string()));
    };
    let method_to_use = match api_type {
        ApiType::OpenApi | ApiType::AsyncApi => method.to_uppercase(),
        ApiType::Proto => method.to_string(),
    };

    // The branch's own endpoint wins outright — including when it has no
    // recorded versions. Version rows are only written on protected branches, so
    // a feature branch legitimately holds a published endpoint with an empty
    // history; returning that empty history is correct, because the caller then
    // shows the branch's *own* current spec. Falling through to an ancestor here
    // would display another branch's API under this branch's name.
    if let Some(branch_id) = repo.find_branch(service_id, branch).await?
        && let Some(endpoint_id) = repo
            .get_endpoint_id(branch_id, api_type, path, &method_to_use)
            .await?
    {
        return Ok(repo.get_endpoint_versions(endpoint_id).await?);
    }

    // The branch doesn't have the endpoint. If it is authoritative, that is
    // deliberate and there is no history to show from anywhere else.
    if branch_is_authoritative(repo, service_id, branch).await? {
        return Err(AppError::Gone(format!(
            "Branch '{}' publishes a spec that does not include {} {}",
            branch, method_to_use, path
        )));
    }

    // Never published: inherit. Prefer a candidate with real history, then any
    // candidate that merely has the endpoint. The returned versions carry the
    // branch they live on (`branch_name`), so the caller can say where from.
    let mut candidates: Option<Vec<String>> = None;
    let chain = resolve_candidates(repo, service_id, producername, branch, &mut candidates)
        .await?
        .clone();
    let mut first_without_history: Option<i64> = None;
    for candidate in &chain {
        if let Some(candidate_branch_id) = repo.find_branch(service_id, candidate).await?
            && let Some(endpoint_id) = repo
                .get_endpoint_id(candidate_branch_id, api_type, path, &method_to_use)
                .await?
        {
            let versions = repo.get_endpoint_versions(endpoint_id).await?;
            if !versions.is_empty() {
                return Ok(versions);
            }
            first_without_history.get_or_insert(endpoint_id);
        }
    }

    match first_without_history {
        Some(endpoint_id) => Ok(repo.get_endpoint_versions(endpoint_id).await?),
        None => Err(AppError::NotFound("Endpoint not found".to_string())),
    }
}

pub async fn get_audit_timeline(
    repo: &impl SpecRepository,
    limit: u32,
) -> Result<Vec<EndpointVersion>, AppError> {
    Ok(repo.get_global_endpoint_versions(limit).await?)
}

#[derive(Debug)]
pub struct RequireResponse {
    pub yaml: String,
    pub deprecated: bool,
    pub external: bool,
    /// How this was resolved, surfaced to Consumers as `X-Sanshain-Resolution`.
    pub state: ResolutionState,
    /// The branch that actually served the spec, surfaced as
    /// `X-Sanshain-Served-Branch`. For a bundle this is the set of branches the
    /// endpoints came from, joined — usually one.
    pub served_branch: Option<String>,
}

pub async fn require_endpoint(
    repo: &impl SpecRepository,
    notifier: Option<tokio::sync::broadcast::Receiver<()>>,
    params: RequireEndpointParams<'_>,
) -> Result<RequireResponse, AppError> {
    require_endpoint_inner(repo, notifier, params, false).await
}

pub async fn require_endpoint_dry_run(
    repo: &impl SpecRepository,
    notifier: Option<tokio::sync::broadcast::Receiver<()>>,
    params: RequireEndpointParams<'_>,
) -> Result<RequireResponse, AppError> {
    require_endpoint_inner(repo, notifier, params, true).await
}

#[instrument(skip_all)]
async fn require_endpoint_inner(
    repo: &impl SpecRepository,
    mut notifier: Option<tokio::sync::broadcast::Receiver<()>>,
    params: RequireEndpointParams<'_>,
    dry_run: bool,
) -> Result<RequireResponse, AppError> {
    let client_id = if dry_run {
        0
    } else {
        repo.ensure_client(params.consumername).await?
    };
    let service_id = if dry_run {
        repo.find_service(params.producername)
            .await?
            .ok_or_else(|| AppError::NotFound("Service not found".to_string()))?
    } else {
        repo.ensure_service(params.producername).await?
    };

    let method_to_use = match params.api_type {
        ApiType::OpenApi | ApiType::AsyncApi => params.method.to_uppercase(),
        ApiType::Proto => params.method.to_string(),
    };

    // `pull_from_branch` (item #17) is a one-shot, side-effect-free override:
    // when present, skip the sticky `source_protected_branch` hint entirely
    // (never persisted) and bypass all fallback resolution below. Also gated
    // on a hint actually being supplied: `ensure_branch` creates the branch
    // row if missing, and a plain require (no hint) for a nonexistent
    // service/branch must not spuriously create one — that would make an
    // otherwise-unknown "phantom" service/branch show up in the overview.
    if !dry_run
        && params.pull_from_branch.is_none()
        && let Some(hint) = params.source_protected_branch
    {
        let bid = repo.ensure_branch(service_id, params.branch).await?;
        apply_source_protected_branch_hint(
            repo,
            bid,
            params.producername,
            params.branch,
            Some(hint),
        )
        .await?;
    }

    let deadline = params
        .timeout_secs
        .map(|s| std::time::Instant::now() + std::time::Duration::from_secs(s));
    let poll_interval = std::time::Duration::from_millis(500);

    // Filled on the first poll iteration that actually reaches the fallback,
    // then reused for the rest of the long-poll instead of being re-resolved
    // every 500ms.
    let mut candidates: Option<Vec<String>> = None;

    loop {
        let resolution = if let Some(pull_from_branch) = params.pull_from_branch {
            // A one-shot pin: resolve against exactly this branch, no inheritance.
            match repo
                .find_endpoint(
                    service_id,
                    pull_from_branch,
                    params.api_type,
                    params.path,
                    &method_to_use,
                )
                .await?
            {
                Some((id, yaml_content, deprecated, external)) => EndpointResolution::published(
                    pull_from_branch,
                    ResolvedEndpoint {
                        id,
                        yaml_content,
                        deprecated,
                        external,
                    },
                ),
                None => EndpointResolution::unknown(),
            }
        } else {
            resolve_endpoint(
                repo,
                BranchLookup {
                    service_id,
                    producername: params.producername,
                    branch: params.branch,
                    api_type: params.api_type,
                },
                params.path,
                &method_to_use,
                &mut candidates,
            )
            .await?
        };

        // Absent is definitive: the branch published a spec and this endpoint is
        // not in it. Waiting cannot change that, so fail immediately even when a
        // timeout was supplied — only Unknown is worth long-polling on.
        if resolution.state == ResolutionState::Absent {
            // Record the unmet dependency first, exactly as the not-found path
            // below does: a Consumer wanting an endpoint its Producer has
            // deliberately dropped is precisely the breakage the dependency
            // views exist to surface, so it must not vanish from them.
            if !dry_run {
                repo.record_dependency(RecordDependencyParams {
                    client_id,
                    endpoint_id: None,
                    api_type: params.api_type,
                    service_id,
                    branch_name: params.branch,
                    path: params.path,
                    method: &method_to_use,
                })
                .await?;
            }
            return Err(AppError::Gone(format!(
                "Branch '{}' of service '{}' publishes a spec that does not include {} {}. \
                 It was not inherited from another branch because this branch is authoritative.",
                params.branch, params.producername, method_to_use, params.path
            )));
        }

        let served_branch = resolution.served_branch.clone();
        if let Some(ResolvedEndpoint {
            id,
            yaml_content: yaml,
            deprecated,
            external,
        }) = resolution.endpoint
        {
            let _ = &served_branch;
            if !dry_run {
                repo.record_dependency(RecordDependencyParams {
                    client_id,
                    endpoint_id: Some(id),
                    api_type: params.api_type,
                    service_id,
                    branch_name: params.branch,
                    path: params.path,
                    method: &method_to_use,
                })
                .await?;
                tracing::info!(
                    service = params.producername,
                    branch = params.branch,
                    "Client '{}' required {:?} {} {} on branch '{}'",
                    params.consumername,
                    params.api_type,
                    method_to_use,
                    params.path,
                    params.branch
                );
            }
            return Ok(RequireResponse {
                yaml,
                deprecated,
                external,
                state: resolution.state,
                served_branch,
            });
        }

        if let Some(dl) = deadline {
            let now = std::time::Instant::now();
            if now < dl {
                let timeout = dl - now;
                match notifier.as_mut() {
                    Some(rx) => {
                        tokio::select! { _ = tokio::time::sleep(timeout) => {}, _ = rx.recv() => {}, }
                    }
                    None => {
                        tokio::time::sleep(poll_interval.min(timeout)).await;
                    }
                }
                continue;
            }
        }

        if !dry_run {
            repo.record_dependency(RecordDependencyParams {
                client_id,
                endpoint_id: None,
                api_type: params.api_type,
                service_id,
                branch_name: params.branch,
                path: params.path,
                method: &method_to_use,
            })
            .await?;
        }
        return Err(AppError::NotFound(format!(
            "Endpoint not found: {} {} (service: {}, branch: {})",
            method_to_use, params.path, params.producername, params.branch
        )));
    }
}

pub async fn require_bundle(
    repo: &impl SpecRepository,
    notifier: Option<tokio::sync::broadcast::Receiver<()>>,
    params: RequireBundleParams<'_>,
) -> Result<RequireResponse, AppError> {
    require_bundle_inner(repo, notifier, params, false).await
}

pub async fn require_bundle_dry_run(
    repo: &impl SpecRepository,
    notifier: Option<tokio::sync::broadcast::Receiver<()>>,
    params: RequireBundleParams<'_>,
) -> Result<RequireResponse, AppError> {
    require_bundle_inner(repo, notifier, params, true).await
}

pub async fn update_endpoint_manual(
    repo: &impl SpecRepository,
    params: RequireEndpointParams<'_>,
    api_type: ApiType,
    yaml: String,
    deprecated: bool,
    external: bool,
) -> Result<(), AppError> {
    let service_id = repo
        .find_service(params.producername)
        .await?
        .ok_or_else(|| AppError::NotFound("Service not found".to_string()))?;
    let branch_id = repo
        .find_branch(service_id, params.branch)
        .await?
        .ok_or_else(|| AppError::NotFound("Branch not found".to_string()))?;

    repo.update_endpoint(UpdateEndpointParams {
        branch_id,
        api_type,
        path: params.path,
        method: params.method,
        yaml_content: &yaml,
        deprecated,
        external,
    })
    .await?;

    Ok(())
}

#[instrument(skip_all)]
async fn require_bundle_inner(
    repo: &impl SpecRepository,
    mut notifier: Option<tokio::sync::broadcast::Receiver<()>>,
    params: RequireBundleParams<'_>,
    dry_run: bool,
) -> Result<RequireResponse, AppError> {
    if params.endpoints.is_empty() {
        return Err(AppError::BadRequest("No endpoints requested".to_string()));
    }
    let client_id = if dry_run {
        0
    } else {
        repo.ensure_client(params.consumername).await?
    };
    let service_id = if dry_run {
        repo.find_service(params.producername)
            .await?
            .ok_or_else(|| AppError::NotFound("Service not found".to_string()))?
    } else {
        repo.ensure_service(params.producername).await?
    };

    // Same lineage handling as the single-endpoint `/require` (item #17):
    // record the sticky hint on first supply, and skip it entirely when
    // `pull_from_branch` pins this one request to an exact branch. Gated on a
    // hint actually being present so a plain bundle require never creates a
    // phantom branch row for an unknown service/branch.
    if !dry_run
        && params.pull_from_branch.is_none()
        && let Some(hint) = params.source_protected_branch
    {
        let bid = repo.ensure_branch(service_id, params.branch).await?;
        apply_source_protected_branch_hint(
            repo,
            bid,
            params.producername,
            params.branch,
            Some(hint),
        )
        .await?;
    }

    let deadline = params
        .timeout_secs
        .map(|s| std::time::Instant::now() + std::time::Duration::from_secs(s));
    let poll_interval = std::time::Duration::from_millis(500);

    // Lazily filled and reused across poll iterations — see `require_endpoint_inner`.
    let mut candidates: Option<Vec<String>> = None;

    loop {
        let normalized_endpoints: Vec<(String, String)> = params
            .endpoints
            .iter()
            .map(|(p, m)| {
                (
                    p.clone(),
                    match params.api_type {
                        ApiType::OpenApi | ApiType::AsyncApi => m.to_uppercase(),
                        ApiType::Proto => m.to_string(),
                    },
                )
            })
            .collect();

        // Sort endpoints deterministically so bundle output (and thus ETag) is
        // independent of the request order.
        let mut sorted_endpoints = normalized_endpoints.clone();
        sorted_endpoints.sort();

        // `pull_from_branch` pins this request to an exact branch, bypassing
        // fallback resolution entirely (and persisting nothing).
        let resolution = if let Some(pull_from_branch) = params.pull_from_branch {
            let endpoints = repo
                .find_endpoints_bulk(
                    service_id,
                    pull_from_branch,
                    params.api_type,
                    &normalized_endpoints,
                )
                .await?;
            let served = if endpoints.is_empty() {
                Vec::new()
            } else {
                vec![pull_from_branch.to_string()]
            };
            BulkResolution {
                endpoints,
                served_branches: served,
                authoritative: true,
            }
        } else {
            resolve_endpoints_bulk(
                repo,
                BranchLookup {
                    service_id,
                    producername: params.producername,
                    branch: params.branch,
                    api_type: params.api_type,
                },
                &normalized_endpoints,
                &mut candidates,
            )
            .await?
        };
        let found_endpoints = resolution.endpoints;

        // Anything still missing on an authoritative branch is deliberately not
        // part of its API. Definitive, so fail immediately rather than waiting
        // out a timeout that cannot change the answer.
        if found_endpoints.len() != params.endpoints.len() && resolution.authoritative {
            let mut absent = Vec::new();
            for (path, method) in &normalized_endpoints {
                if !found_endpoints.contains_key(&(path.clone(), method.clone())) {
                    absent.push(format!("{} {}", method, path));
                }
            }
            absent.sort();
            return Err(AppError::Gone(format!(
                "Branch '{}' of service '{}' publishes a spec that does not include: {}. \
                 These were not inherited from another branch because this branch is authoritative.",
                params.branch,
                params.producername,
                absent.join(", ")
            )));
        }

        if found_endpoints.len() == params.endpoints.len() {
            let mut yamls = Vec::new();
            let mut bulk_params = Vec::new();
            let mut any_deprecated = false;
            let mut any_external = false;
            for (path, method) in &sorted_endpoints {
                let (id, yaml, deprecated, external) = found_endpoints
                    .get(&(path.clone(), method.clone()))
                    .ok_or_else(|| {
                        AppError::Internal(format!(
                            "endpoint unexpectedly missing: {} {}",
                            method, path
                        ))
                    })?;
                yamls.push(yaml.clone());
                if *deprecated {
                    any_deprecated = true;
                }
                if *external {
                    any_external = true;
                }
                bulk_params.push(RecordDependencyParams {
                    client_id,
                    endpoint_id: Some(*id),
                    api_type: params.api_type,
                    service_id,
                    branch_name: params.branch,
                    path,
                    method,
                });
            }
            if !dry_run {
                repo.record_dependencies_bulk(bulk_params).await?;
            }
            let merged_yaml = match params.api_type {
                ApiType::OpenApi => {
                    openapi::merge_endpoint_yamls(&yamls).map_err(AppError::Internal)?
                }
                _ => yamls.join("\n---\n"),
            };
            // A bundle can legitimately draw from more than one branch, so name
            // them all; the state reflects whether any inheritance happened.
            let inherited = resolution
                .served_branches
                .iter()
                .any(|b| b != params.branch);
            return Ok(RequireResponse {
                yaml: merged_yaml,
                deprecated: any_deprecated,
                external: any_external,
                state: if inherited {
                    ResolutionState::Inherited
                } else {
                    ResolutionState::Published
                },
                served_branch: if resolution.served_branches.is_empty() {
                    None
                } else {
                    Some(resolution.served_branches.join(","))
                },
            });
        }

        if let Some(dl) = deadline {
            let now = std::time::Instant::now();
            if now < dl {
                let timeout = dl - now;
                match notifier.as_mut() {
                    Some(rx) => {
                        tokio::select! { _ = tokio::time::sleep(timeout) => {}, _ = rx.recv() => {}, }
                    }
                    None => {
                        tokio::time::sleep(poll_interval.min(timeout)).await;
                    }
                }
                continue;
            }
        }

        let mut missing_list = Vec::new();
        for (path, method) in params.endpoints {
            let m_use = match params.api_type {
                ApiType::OpenApi | ApiType::AsyncApi => method.to_uppercase(),
                ApiType::Proto => method.to_string(),
            };
            if !found_endpoints.contains_key(&(path.clone(), m_use.clone())) {
                missing_list.push(format!("missing {} {}", m_use, path));
            }
        }
        return Err(AppError::NotFound(missing_list.join(", ")));
    }
}

/// Ordered list of candidate branches to fall back to when `branch` doesn't have
/// what's needed: the branch's own `source_protected_branch` (item #17, if
/// set — the caller-supplied or admin-corrected hint), then the service's
/// configured fallback branch (if set), then every protected branch —
/// excluding `branch` itself, in priority order.
async fn fallback_branch_candidates(
    repo: &impl SpecRepository,
    service_id: i64,
    producername: &str,
    branch: &str,
) -> Result<Vec<String>, RepositoryError> {
    let mut candidates = Vec::new();
    if let Some(branch_id) = repo.find_branch(service_id, branch).await?
        && let Some(spb) = repo.get_source_protected_branch(branch_id).await?
        && spb != branch
    {
        candidates.push(spb);
    }
    if let Ok(Some(sfb)) = repo.get_fallback_branch(producername).await
        && sfb != branch
        && !candidates.contains(&sfb)
    {
        candidates.push(sfb);
    }
    // `list_protected_branches` returns *patterns*, not branch names: a wildcard
    // entry like `release/*` is not itself a branch and can never be resolved
    // against. Expand the patterns over the service's real branches instead, so
    // wildcard-protected branches actually participate in fallback resolution.
    let patterns = repo.list_protected_branches().await?;
    let mut protected: Vec<String> = repo
        .list_branches(producername)
        .await?
        .into_iter()
        .filter(|b| branch_matches_any(b, &patterns))
        .collect();
    protected.sort();
    for pb in protected {
        if pb != branch && !candidates.contains(&pb) {
            candidates.push(pb);
        }
    }
    Ok(candidates)
}

/// Which service/branch (and API flavour) a lookup is resolving against.
/// Grouped so the resolvers stay within a sane argument count.
#[derive(Clone, Copy)]
struct BranchLookup<'a> {
    service_id: i64,
    producername: &'a str,
    branch: &'a str,
    api_type: ApiType,
}

/// Whether a branch's spec is the complete statement of its own API.
///
/// A `/provide` submits a *complete* spec, so once a branch has published
/// anything, an endpoint missing from it is missing by choice. Only a branch
/// that has never published inherits.
///
/// Protection does *not* confer authority. A protected branch that has never
/// published has said nothing about its API, and a Consumer asking for it is in
/// the ordinary first-publish race that long-polling exists to absorb — the
/// producer's first build simply hasn't run yet. Treating it as authoritative
/// would answer `Absent` and fail fast, breaking every new producer's first CI
/// run.
async fn branch_is_authoritative(
    repo: &impl SpecRepository,
    service_id: i64,
    branch: &str,
) -> Result<bool, RepositoryError> {
    let Some(branch_id) = repo.find_branch(service_id, branch).await? else {
        return Ok(false);
    };
    Ok(repo
        .get_spec_version(service_id, branch_id)
        .await?
        .is_some())
}

/// Resolve one endpoint against one branch, reporting *how* it was answered.
///
/// The order matters and is the whole point of the model:
/// 1. the branch has it            → `Published`
/// 2. else the branch is authoritative → `Absent` — stop, never inherit
/// 3. else walk the candidate chain    → `Inherited`, naming the branch
/// 4. else                             → `Unknown`
///
/// `candidates` is a lazily-filled cache, not an input: the chain is resolved
/// only once step 3 is actually reached, and then reused. That keeps the common
/// cases free, while a long-poll that waits out its timeout still resolves the
/// chain once rather than on every 500ms iteration. (It cannot change in a way
/// this caller could observe mid-poll.)
async fn resolve_endpoint(
    repo: &impl SpecRepository,
    lookup: BranchLookup<'_>,
    path: &str,
    method_to_use: &str,
    candidates: &mut Option<Vec<String>>,
) -> Result<EndpointResolution, RepositoryError> {
    let BranchLookup {
        service_id,
        producername,
        branch,
        api_type,
    } = lookup;

    if let Some((id, yaml_content, deprecated, external)) = repo
        .find_endpoint(service_id, branch, api_type, path, method_to_use)
        .await?
    {
        return Ok(EndpointResolution::published(
            branch,
            ResolvedEndpoint {
                id,
                yaml_content,
                deprecated,
                external,
            },
        ));
    }

    if branch_is_authoritative(repo, service_id, branch).await? {
        return Ok(EndpointResolution::absent());
    }

    // Authority and inheritance are separate questions. A protected branch that
    // has not published yet is not authoritative — the answer may still arrive,
    // so a long-poll should wait — but it must never *inherit*: it is a declared
    // release line of its own, and serving another line's diverged API under its
    // name is the wrong-branch-served bug this whole model exists to prevent.
    if repo.is_branch_protected(branch).await? {
        return Ok(EndpointResolution::unknown());
    }

    let candidates = resolve_candidates(repo, service_id, producername, branch, candidates).await?;
    for candidate in candidates.iter() {
        if let Some((id, yaml_content, deprecated, external)) = repo
            .find_endpoint(service_id, candidate, api_type, path, method_to_use)
            .await?
        {
            return Ok(EndpointResolution::inherited(
                candidate,
                ResolvedEndpoint {
                    id,
                    yaml_content,
                    deprecated,
                    external,
                },
            ));
        }
    }
    Ok(EndpointResolution::unknown())
}

/// Fill `cache` with the branch's fallback chain on first use, then hand back
/// the cached list. See `find_endpoint_with_fallback` for why this is lazy.
async fn resolve_candidates<'a>(
    repo: &impl SpecRepository,
    service_id: i64,
    producername: &str,
    branch: &str,
    cache: &'a mut Option<Vec<String>>,
) -> Result<&'a Vec<String>, RepositoryError> {
    if cache.is_none() {
        let resolved = fallback_branch_candidates(repo, service_id, producername, branch).await?;
        *cache = Some(resolved);
    }
    Ok(cache.as_ref().unwrap_or(const { &Vec::new() }))
}

/// What a bulk resolution produced: the endpoints found, the branch each came
/// from, and whether the requested branch is authoritative (in which case
/// anything still missing is `Absent`, not `Unknown`).
struct BulkResolution {
    endpoints: crate::domain::ports::EndpointMap,
    /// Branches that actually served endpoints, in the order they were consulted.
    served_branches: Vec<String>,
    /// True when the requested branch's own spec is the complete answer, so the
    /// caller must report anything missing as deliberately absent.
    authoritative: bool,
}

/// Bulk counterpart of `resolve_endpoint`, applying the identical rule to a set
/// of endpoints. It must resolve the same way — a bundle require that disagreed
/// with the single-endpoint require would hand a Consumer a different branch's
/// API for the same lineage.
async fn resolve_endpoints_bulk(
    repo: &impl SpecRepository,
    lookup: BranchLookup<'_>,
    endpoints: &[(String, String)],
    candidates: &mut Option<Vec<String>>,
) -> Result<BulkResolution, RepositoryError> {
    let BranchLookup {
        service_id,
        producername,
        branch,
        api_type,
    } = lookup;
    let mut results = repo
        .find_endpoints_bulk(service_id, branch, api_type, endpoints)
        .await?;
    let mut served_branches = Vec::new();
    if !results.is_empty() {
        served_branches.push(branch.to_string());
    }
    let mut missing: Vec<(String, String)> = endpoints
        .iter()
        .filter(|e| !results.contains_key(&(e.0.clone(), e.1.clone())))
        .cloned()
        .collect();

    if missing.is_empty() {
        return Ok(BulkResolution {
            endpoints: results,
            served_branches,
            authoritative: true,
        });
    }

    // The branch published its own spec, so what it omits it omits on purpose.
    if branch_is_authoritative(repo, service_id, branch).await? {
        return Ok(BulkResolution {
            endpoints: results,
            served_branches,
            authoritative: true,
        });
    }

    // Not authoritative, but a protected branch still never inherits another
    // release line's API — see `resolve_endpoint`. Anything missing stays
    // missing (Unknown), so a long-poll can wait for its first publish.
    if repo.is_branch_protected(branch).await? {
        return Ok(BulkResolution {
            endpoints: results,
            served_branches,
            authoritative: false,
        });
    }

    let candidates = resolve_candidates(repo, service_id, producername, branch, candidates).await?;
    for candidate in candidates.iter() {
        let fallback_results = repo
            .find_endpoints_bulk(service_id, candidate, api_type, &missing)
            .await?;
        if !fallback_results.is_empty() {
            served_branches.push(candidate.clone());
        }
        for (key, val) in fallback_results {
            results.insert(key.clone(), val);
            missing.retain(|m| m.0 != key.0 || m.1 != key.1);
        }
        if missing.is_empty() {
            break;
        }
    }
    Ok(BulkResolution {
        endpoints: results,
        served_branches,
        authoritative: false,
    })
}

pub fn check_compatibility(
    api_type: ApiType,
    old_yaml: &str,
    new_yaml: &str,
) -> Result<(), String> {
    match api_type {
        ApiType::OpenApi => openapi::check_backward_compatibility(old_yaml, new_yaml),
        ApiType::AsyncApi => crate::asyncapi::check_backward_compatibility(old_yaml, new_yaml),
        ApiType::Proto => crate::proto::check_backward_compatibility(old_yaml, new_yaml),
    }
}

// --- Reviewing held Provides ---

/// Apply a held Provide.
///
/// Replays the submission exactly as it was received, with the refusals
/// suspended. Two things are deliberately skipped:
///
/// - **The stale-base-version check.** It exists to stop a Producer overwriting
///   work it had not seen. An approver applying a specific submission they have
///   just read is a different act, and failing after review would make approval
///   unreliable for reasons the approver cannot see.
/// - **Nothing else.** Everything a normal Provide records — version bump, soft
///   deletes, version history — happens as usual, because the change really is
///   landing.
///
/// The held entry is cleared by the successful Provide itself, on the same rule
/// that clears it when a Producer fixes the problem on its own.
pub async fn apply_pending_spec(
    repo: &impl SpecRepository,
    pending_id: i64,
    actor: Option<&str>,
) -> Result<ProvideResponse, AppError> {
    let pending = repo
        .get_pending_spec(pending_id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("No held spec #{}", pending_id)))?;

    let response = provide_spec_inner(
        repo,
        ProvideInternalParams {
            producername: &pending.producer,
            branch: &pending.branch,
            api_type: pending.api_type,
            content: &pending.content,
            dry_run: false,
            extra_tags: &[],
            base_version: None,
            force: false,
            username: actor,
            source_protected_branch: None,
            author: Some(&pending.submitted_by),
            waived: true,
        },
    )
    .await?;

    record_review(
        repo,
        actor,
        "ACCEPTED_PENDING",
        &pending,
        &format!(
            "Applied held {:?} spec #{} for service '{}' on branch '{}': {}",
            pending.api_type, pending.id, pending.producer, pending.branch, pending.reason
        ),
    )
    .await;

    Ok(response)
}

/// Discard a held Provide without applying it.
///
/// The Producer's next Provide is evaluated fresh, so this is a decision about
/// one submission rather than a standing refusal.
pub async fn reject_pending_spec(
    repo: &impl SpecRepository,
    pending_id: i64,
    actor: Option<&str>,
) -> Result<(), AppError> {
    let pending = repo
        .get_pending_spec(pending_id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("No held spec #{}", pending_id)))?;

    repo.delete_pending_spec(pending_id).await?;
    record_review(
        repo,
        actor,
        "REJECTED_PENDING",
        &pending,
        &format!(
            "Discarded held {:?} spec #{} for service '{}' on branch '{}'",
            pending.api_type, pending.id, pending.producer, pending.branch
        ),
    )
    .await;
    Ok(())
}

pub async fn list_pending_specs(repo: &impl SpecRepository) -> Result<Vec<PendingSpec>, AppError> {
    Ok(repo.list_pending_specs().await?)
}

pub async fn get_pending_spec(
    repo: &impl SpecRepository,
    pending_id: i64,
) -> Result<PendingSpec, AppError> {
    repo.get_pending_spec(pending_id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("No held spec #{}", pending_id)))
}

/// Record a review decision. Best effort, like the other audit writes on this
/// path: losing the entry must not undo the decision.
async fn record_review(
    repo: &impl SpecRepository,
    actor: Option<&str>,
    action: &str,
    pending: &PendingSpec,
    details: &str,
) {
    if let Err(e) = repo
        .insert_audit_log(
            actor.unwrap_or("DevMode/Anonymous"),
            NewAuditLog {
                action,
                details,
                service: Some(&pending.producer),
                branch: Some(&pending.branch),
                action_type: Some("ADMIN"),
                diff: None,
            },
        )
        .await
    {
        tracing::warn!("Could not record the review decision: {}", e);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::mock_repo::MockRepo;

    // A parse failure logs a compact fingerprint, not the whole submitted spec,
    // so a large submission cannot flood the in-memory (admin-viewable) log
    // buffer. (Spec content is public by design; this is log hygiene, not secrecy.)
    #[test]
    fn parse_error_does_not_log_submitted_content() {
        use std::io::Write;
        use std::sync::{Arc, Mutex};
        use tracing_subscriber::fmt::MakeWriter;

        #[derive(Clone)]
        struct BufMakeWriter(Arc<Mutex<Vec<u8>>>);
        struct BufGuard(Arc<Mutex<Vec<u8>>>);
        impl Write for BufGuard {
            fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
                self.0.lock().unwrap().extend_from_slice(data);
                Ok(data.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        impl<'a> MakeWriter<'a> for BufMakeWriter {
            type Writer = BufGuard;
            fn make_writer(&'a self) -> Self::Writer {
                BufGuard(self.0.clone())
            }
        }

        let buf = Arc::new(Mutex::new(Vec::<u8>::new()));
        let subscriber = tracing_subscriber::fmt()
            .with_writer(BufMakeWriter(buf.clone()))
            .with_max_level(tracing::Level::WARN)
            .with_ansi(false)
            .finish();

        let marker = "UNIQUE_CONTENT_MARKER_9f8e7d6c";
        // Unclosed flow sequence -> guaranteed YAML/parse failure.
        let bad_spec = format!("openapi: \"3.0.0\"\npaths: [unclosed\nmarker: {marker}\n");

        tracing::subscriber::with_default(subscriber, || {
            let result = parse_spec_endpoints(ApiType::OpenApi, &bad_spec, "billing", "main");
            assert!(result.is_err(), "malformed spec should fail to parse");
        });

        let logged = String::from_utf8(buf.lock().unwrap().clone()).unwrap();
        assert!(
            logged.contains("Failed to split OpenAPI"),
            "expected a parse-failure warning to be logged, got: {logged:?}"
        );
        assert!(
            !logged.contains(marker),
            "parse-failure log dumped the full submitted spec: {logged:?}"
        );
    }

    const SIMPLE_OPENAPI: &str = r#"openapi: "3.0.0"
info:
  title: Test
  version: "1.0"
paths:
  /hello:
    get:
      summary: Hello
      responses:
        "200":
          description: OK
"#;

    const UPDATED_OPENAPI: &str = r#"openapi: "3.0.0"
info:
  title: Test
  version: "1.1"
paths:
  /hello:
    get:
      summary: Hello updated
      responses:
        "200":
          description: OK
  /world:
    post:
      summary: World
      responses:
        "201":
          description: Created
"#;

    #[tokio::test]
    async fn test_provide_spec_insert() {
        let repo = MockRepo::new();
        let resp = provide_spec(
            &repo,
            "svc",
            "main",
            ApiType::OpenApi,
            SIMPLE_OPENAPI,
            None,
            false,
        )
        .await
        .unwrap();
        assert_eq!(resp.version, SemVer::new(1, 0, 0));
        assert_eq!(resp.changes.inserts, 1);
        assert_eq!(resp.changes.updates, 0);
        assert_eq!(resp.changes.deletes, 0);
    }

    #[tokio::test]
    async fn test_provide_spec_semver_increment() {
        let repo = MockRepo::new();
        // 1. Initial version
        let r1 = provide_spec(
            &repo,
            "svc",
            "main",
            ApiType::OpenApi,
            SIMPLE_OPENAPI,
            None,
            false,
        )
        .await
        .unwrap();
        assert_eq!(r1.version, SemVer::new(1, 0, 0));

        // 2. Patch version (minor change like description)
        let patch_spec = SIMPLE_OPENAPI.replace("summary: Hello", "summary: Hello Patch");
        let r2 = provide_spec(
            &repo,
            "svc",
            "main",
            ApiType::OpenApi,
            &patch_spec,
            None,
            false,
        )
        .await
        .unwrap();
        assert_eq!(r2.version, SemVer::new(1, 0, 1));

        // 3. Minor version (addition)
        let r3 = provide_spec(
            &repo,
            "svc",
            "main",
            ApiType::OpenApi,
            UPDATED_OPENAPI,
            None,
            false,
        )
        .await
        .unwrap();
        assert_eq!(r3.version, SemVer::new(1, 1, 0));

        // 4. Major version (breaking change)
        let breaking_spec = SIMPLE_OPENAPI.replace("/hello:", "/hi:");
        let _r4 = provide_spec(
            &repo,
            "svc",
            "feat-breaking", // Use non-protected branch for breaking change
            ApiType::OpenApi,
            &breaking_spec,
            None,
            true, // force because it's breaking
        )
        .await
        .unwrap();
        // Since it's a new branch, it starts at 1.0.0?
        // No, if it's a new branch, it should start at 1.0.0.

        // Wait, if I want to test MAJOR increment, I should do it on the same branch.
        // So I'll unprotect the branch first.
        repo.remove_protected_branch("main").await.unwrap();
        let r4 = provide_spec(
            &repo,
            "svc",
            "main",
            ApiType::OpenApi,
            &breaking_spec,
            None,
            true,
        )
        .await
        .unwrap();
        assert_eq!(r4.version, SemVer::new(2, 0, 0));
    }

    #[tokio::test]
    async fn test_provide_spec_no_change_skip() {
        let repo = MockRepo::new();
        let r1 = provide_spec(
            &repo,
            "svc",
            "main",
            ApiType::OpenApi,
            SIMPLE_OPENAPI,
            None,
            false,
        )
        .await
        .unwrap();
        let r2 = provide_spec(
            &repo,
            "svc",
            "main",
            ApiType::OpenApi,
            SIMPLE_OPENAPI,
            None,
            false,
        )
        .await
        .unwrap();
        assert_eq!(r1.version, r2.version);
        assert_eq!(r2.changes.inserts, 0);
        assert_eq!(r2.changes.updates, 0);
        assert_eq!(r2.changes.deletes, 0);
    }

    #[tokio::test]
    async fn test_list_endpoints_on_feature_branch() {
        let repo = MockRepo::new();
        // 1. Provide initial spec to main
        provide_spec(
            &repo,
            "svc",
            "main",
            ApiType::OpenApi,
            SIMPLE_OPENAPI,
            None,
            false,
        )
        .await
        .unwrap();

        // 2. Provide modified spec to a feature branch
        provide_spec(
            &repo,
            "svc",
            "feat",
            ApiType::OpenApi,
            UPDATED_OPENAPI,
            None,
            false,
        )
        .await
        .unwrap();

        // 3. List endpoints on the feature branch
        let endpoints = list_producer_endpoints(&repo, "svc", "feat").await.unwrap();
        assert!(endpoints.iter().any(|e| e.path == "/hello"));
        assert!(endpoints.iter().any(|e| e.path == "/world"));
    }

    // Feature branches accept breaking changes without `force`; protected
    // branches remain the only compatibility gate.
    #[tokio::test]
    async fn test_breaking_change_allowed_on_feature_branch_without_force() {
        let repo = MockRepo::new();
        // Establish the spec on the protected branch first, so the service
        // is not a "new service" special case.
        provide_spec(
            &repo,
            "svc",
            "main",
            ApiType::OpenApi,
            SIMPLE_OPENAPI,
            None,
            false,
        )
        .await
        .unwrap();
        provide_spec(
            &repo,
            "svc",
            "feat",
            ApiType::OpenApi,
            SIMPLE_OPENAPI,
            None,
            false,
        )
        .await
        .unwrap();

        // Breaking change (removes the endpoint) on the feature branch: accepted.
        let breaking = r#"
openapi: 3.0.0
info: { title: T, version: 2.0.0 }
paths:
  /renamed:
    get:
      responses:
        '200': { description: OK }
"#;
        let resp = provide_spec(
            &repo,
            "svc",
            "feat",
            ApiType::OpenApi,
            breaking,
            None,
            false,
        )
        .await
        .unwrap();
        assert_eq!(resp.changes.inserts, 1);
        assert_eq!(resp.changes.deletes, 1);
    }

    #[tokio::test]
    async fn test_provide_spec_dry_run() {
        let repo = MockRepo::new();
        let resp = provide_spec_dry_run(
            &repo,
            "svc",
            "main",
            ApiType::OpenApi,
            SIMPLE_OPENAPI,
            false,
        )
        .await
        .unwrap();
        assert_eq!(resp.changes.inserts, 1);
        // Dry run should not persist
        let services = repo.list_producers().await.unwrap();
        assert!(services.is_empty());
    }

    #[tokio::test]
    async fn test_check_compatibility_openapi_pass() {
        let result = check_compatibility(ApiType::OpenApi, SIMPLE_OPENAPI, SIMPLE_OPENAPI);
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_check_compatibility_asyncapi_detects_breaking_change() {
        let old = r#"
asyncapi: 2.6.0
info: { title: T, version: 1.0.0 }
channels:
  user-created:
    publish:
      message:
        payload:
          type: object
          properties:
            id: { type: string }
"#;
        let new = old.replace("id: { type: string }", "id: { type: integer }");
        assert_eq!(check_compatibility(ApiType::AsyncApi, old, old), Ok(()));
        let err = check_compatibility(ApiType::AsyncApi, old, &new).unwrap_err();
        assert!(
            err.contains("changed type from 'string' to 'integer'"),
            "{err}"
        );
    }

    #[tokio::test]
    async fn test_check_compatibility_proto_detects_breaking_change() {
        let old = r#"
syntax = "proto3";
message Req { string id = 1; }
message Res { string name = 1; }
service S { rpc Get (Req) returns (Res); }
"#;
        let new = old.replace("string id = 1;", "string id = 2;");
        assert_eq!(check_compatibility(ApiType::Proto, old, old), Ok(()));
        let err = check_compatibility(ApiType::Proto, old, &new).unwrap_err();
        assert!(err.contains("changed number from 1 to 2"), "{err}");
    }

    #[test]
    fn test_parse_spec_endpoints_openapi() {
        let endpoints =
            parse_spec_endpoints(ApiType::OpenApi, SIMPLE_OPENAPI, "svc", "main").unwrap();
        assert_eq!(endpoints.len(), 1);
        assert_eq!(endpoints[0].method, "GET");
    }

    #[test]
    fn test_parse_spec_endpoints_invalid() {
        let result = parse_spec_endpoints(ApiType::OpenApi, "not valid yaml {{", "svc", "main");
        assert!(result.is_err());
    }

    // --- item #20: message-level channel contracts through the provide flow ---

    /// AsyncAPI 2.x document publishing a single named message on `orders`,
    /// with the payload properties spliced in.
    fn asyncapi_pub(props: &str) -> String {
        format!(
            r#"asyncapi: 2.6.0
info:
  title: T
  version: "1.0.0"
channels:
  orders:
    publish:
      message:
        name: OrderPlaced
        payload:
          type: object
          properties:
{props}
"#
        )
    }

    async fn provide_async(
        repo: &MockRepo,
        service: &str,
        branch: &str,
        content: &str,
    ) -> Result<ProvideResponse, AppError> {
        provide_spec(
            repo,
            service,
            branch,
            ApiType::AsyncApi,
            content,
            None,
            false,
        )
        .await
    }

    #[tokio::test]
    async fn asyncapi_provide_registers_contract() {
        let repo = MockRepo::new();
        let a = repo.ensure_service("producer-a").await.unwrap();
        provide_async(
            &repo,
            "producer-a",
            "main",
            &asyncapi_pub("            id: { type: string }"),
        )
        .await
        .unwrap();

        let contracts = repo.list_channel_message_contracts("main").await.unwrap();
        assert_eq!(contracts.len(), 1);
        assert_eq!(contracts[0].channel, "orders");
        assert_eq!(contracts[0].message_name, "OrderPlaced");
        assert_eq!(contracts[0].owner_service_id, a);
    }

    #[tokio::test]
    async fn asyncapi_owner_may_widen_message_minor_bump() {
        let repo = MockRepo::new();
        repo.ensure_service("producer-a").await.unwrap();
        let r1 = provide_async(
            &repo,
            "producer-a",
            "feature",
            &asyncapi_pub("            id: { type: string }"),
        )
        .await
        .unwrap();
        assert_eq!(r1.version, SemVer::new(1, 0, 0));

        let r2 = provide_async(
            &repo,
            "producer-a",
            "feature",
            &asyncapi_pub("            id: { type: string }\n            name: { type: string }"),
        )
        .await
        .unwrap();
        // Additive change → minor bump; stored payload is updated.
        assert_eq!(r2.version, SemVer::new(1, 1, 0));
        let contracts = repo
            .list_channel_message_contracts("feature")
            .await
            .unwrap();
        assert!(contracts[0].payload_yaml.contains("name"));
    }

    #[tokio::test]
    async fn asyncapi_owner_breaking_change_rejected_on_feature_branch() {
        // Message contracts are enforced on all branches, including unprotected
        // ones where endpoint-level breaking changes are otherwise allowed.
        let repo = MockRepo::new();
        repo.ensure_service("producer-a").await.unwrap();
        provide_async(
            &repo,
            "producer-a",
            "feature",
            &asyncapi_pub("            id: { type: string }"),
        )
        .await
        .unwrap();

        let err = provide_async(
            &repo,
            "producer-a",
            "feature",
            &asyncapi_pub("            id: { type: integer }"),
        )
        .await
        .unwrap_err();
        assert!(matches!(err, AppError::BreakingChange(_)), "{err:?}");
    }

    #[tokio::test]
    async fn asyncapi_second_producer_identical_schema_accepted() {
        let repo = MockRepo::new();
        let a = repo.ensure_service("producer-a").await.unwrap();
        repo.ensure_service("producer-b").await.unwrap();
        let doc = asyncapi_pub("            id: { type: string }");
        provide_async(&repo, "producer-a", "main", &doc)
            .await
            .unwrap();
        provide_async(&repo, "producer-b", "main", &doc)
            .await
            .unwrap();

        // Ownership is unchanged and there is still exactly one contract row.
        let contracts = repo.list_channel_message_contracts("main").await.unwrap();
        assert_eq!(contracts.len(), 1);
        assert_eq!(contracts[0].owner_service_id, a);
    }

    #[tokio::test]
    async fn asyncapi_second_producer_divergent_schema_rejected_naming_owner() {
        let repo = MockRepo::new();
        repo.ensure_service("producer-a").await.unwrap();
        repo.ensure_service("producer-b").await.unwrap();
        provide_async(
            &repo,
            "producer-a",
            "main",
            &asyncapi_pub("            id: { type: string }"),
        )
        .await
        .unwrap();

        let err = provide_async(
            &repo,
            "producer-b",
            "main",
            &asyncapi_pub("            id: { type: integer }"),
        )
        .await
        .unwrap_err();
        match err {
            AppError::Conflict(msg) => {
                assert!(msg.contains("producer-a"), "{msg}");
                assert!(msg.contains("OrderPlaced"), "{msg}");
            }
            other => panic!("expected Conflict, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn asyncapi_owner_dropping_message_clears_contract() {
        let repo = MockRepo::new();
        repo.ensure_service("producer-a").await.unwrap();
        provide_async(
            &repo,
            "producer-a",
            "feature",
            &asyncapi_pub("            id: { type: string }"),
        )
        .await
        .unwrap();
        assert_eq!(
            repo.list_channel_message_contracts("feature")
                .await
                .unwrap()
                .len(),
            1
        );

        // A later provide without the message drops it (allowed on a feature
        // branch); the contract row is cleared.
        let empty = r#"asyncapi: 2.6.0
info:
  title: T
  version: "1.0.0"
channels:
  status:
    publish:
      message:
        name: Heartbeat
        payload: { type: object }
"#;
        provide_async(&repo, "producer-a", "feature", empty)
            .await
            .unwrap();
        let contracts = repo
            .list_channel_message_contracts("feature")
            .await
            .unwrap();
        assert_eq!(contracts.len(), 1);
        assert_eq!(contracts[0].message_name, "Heartbeat");
    }

    #[tokio::test]
    async fn asyncapi_unnamed_message_registers_no_contract() {
        let repo = MockRepo::new();
        repo.ensure_service("producer-a").await.unwrap();
        let unnamed = r#"asyncapi: 2.6.0
info:
  title: T
  version: "1.0.0"
channels:
  orders:
    publish:
      message:
        payload: { type: object }
"#;
        provide_async(&repo, "producer-a", "main", unnamed)
            .await
            .unwrap();
        assert!(
            repo.list_channel_message_contracts("main")
                .await
                .unwrap()
                .is_empty()
        );
    }
}
