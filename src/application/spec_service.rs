use crate::asyncapi;
use crate::domain::models::*;
use crate::domain::ports::{
    RecordDependencyParams, RepositoryError, SpecRepository, UpdateEndpointParams,
};
use crate::openapi;
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};

use tracing::instrument;

pub struct RequireEndpointParams<'a> {
    pub clientname: &'a str,
    pub servicename: &'a str,
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
    pub clientname: &'a str,
    pub servicename: &'a str,
    pub branch: &'a str,
    pub api_type: ApiType,
    pub endpoints: &'a [(String, String)],
    pub timeout_secs: Option<u64>,
}

#[instrument(skip_all)]
pub async fn provide_spec(
    repo: &impl SpecRepository,
    servicename: &str,
    branch: &str,
    api_type: ApiType,
    content: &str,
    base_version: Option<String>,
    force: bool,
) -> Result<ProvideResponse, AppError> {
    provide_spec_inner(
        repo,
        ProvideInternalParams {
            servicename,
            branch,
            api_type,
            content,
            dry_run: false,
            extra_tags: &[],
            base_version,
            force,
            username: None,
            source_protected_branch: None,
        },
    )
    .await
}

pub async fn provide_spec_dry_run(
    repo: &impl SpecRepository,
    servicename: &str,
    branch: &str,
    api_type: ApiType,
    content: &str,
    force: bool,
) -> Result<ProvideResponse, AppError> {
    provide_spec_inner(
        repo,
        ProvideInternalParams {
            servicename,
            branch,
            api_type,
            content,
            dry_run: true,
            extra_tags: &[],
            base_version: None,
            force,
            username: None,
            source_protected_branch: None,
        },
    )
    .await
}

/// Inputs describing the spec a service provides for a branch. Grouped so the
/// public provide entry points stay within a sane argument count.
pub struct ProvideSpecParams<'a> {
    pub servicename: &'a str,
    pub branch: &'a str,
    pub api_type: ApiType,
    pub content: &'a str,
    pub base_version: Option<String>,
    pub force: bool,
    /// Sticky hint (item #17): see `RequireQuery::source_protected_branch`.
    pub source_protected_branch: Option<&'a str>,
}

pub async fn provide_spec_with_tags(
    repo: &impl SpecRepository,
    params: ProvideSpecParams<'_>,
    tags: &[String],
) -> Result<ProvideResponse, AppError> {
    provide_spec_inner(
        repo,
        ProvideInternalParams {
            servicename: params.servicename,
            branch: params.branch,
            api_type: params.api_type,
            content: params.content,
            dry_run: false,
            extra_tags: tags,
            base_version: params.base_version,
            force: params.force,
            username: None,
            source_protected_branch: params.source_protected_branch,
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
            servicename: params.servicename,
            branch: params.branch,
            api_type: params.api_type,
            content: params.content,
            dry_run: false,
            extra_tags: &[],
            base_version: params.base_version,
            force: params.force,
            username,
            source_protected_branch: params.source_protected_branch,
        },
    )
    .await
}

struct ProvideInternalParams<'a> {
    pub servicename: &'a str,
    pub branch: &'a str,
    pub api_type: ApiType,
    pub content: &'a str,
    pub dry_run: bool,
    pub extra_tags: &'a [String],
    pub base_version: Option<String>,
    pub force: bool,
    pub username: Option<&'a str>,
    pub source_protected_branch: Option<&'a str>,
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
    servicename: &str,
    branch: &str,
) -> Result<Vec<openapi::EndpointSpec>, AppError> {
    match api_type {
        ApiType::OpenApi => openapi::split_openapi(content).map_err(|e| {
            tracing::warn!(
                "Failed to split OpenAPI for service '{}' branch '{}' ({}): {}",
                servicename,
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
                    servicename,
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
                    servicename,
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
                    servicename,
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
        servicename,
        branch,
        api_type,
        content,
        dry_run,
        extra_tags,
        base_version,
        force,
        username,
        source_protected_branch,
    } = params;

    tracing::debug!(
        "Providing {:?} for service '{}' branch '{}' (dry_run: {}, base_version: {:?})",
        api_type,
        servicename,
        branch,
        dry_run,
        base_version
    );

    let content_hash = {
        let mut hasher = Sha256::new();
        hasher.update(content.as_bytes());
        format!("sha256:{}", hex::encode(hasher.finalize()))
    };

    let endpoints = parse_spec_endpoints(api_type, content, servicename, branch)?;

    tracing::debug!(
        "Successfully split {:?} into {} endpoints for service '{}'",
        api_type,
        endpoints.len(),
        servicename
    );

    let (sid, bid) = if dry_run {
        match repo.find_service(servicename).await? {
            Some(sid) => match repo.find_branch(sid, branch).await? {
                Some(bid) => (sid, bid),
                None => (sid, 0),
            },
            None => (0, 0),
        }
    } else {
        let sid = repo.ensure_service(servicename).await?;
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
        apply_source_protected_branch_hint(repo, bid, servicename, branch, source_protected_branch)
            .await?;
    }

    let (current_version, last_hash) = if sid != 0 && bid != 0 {
        match repo.get_spec_version(sid, bid).await? {
            Some((v, h)) => (v, h),
            None => (SemVer::default(), String::new()),
        }
    } else {
        (SemVer::default(), String::new())
    };

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

    if force && is_protected {
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

    if let (Some(old_yaml), true) = (&old_full_yaml, is_protected && api_type == ApiType::OpenApi) {
        if let Err(reason) = openapi::check_backward_compatibility(old_yaml, content) {
            tracing::warn!(
                "Rejected update for service '{}' branch '{}': breaking changes: {}",
                servicename,
                branch,
                reason
            );
            return Err(AppError::BreakingChange(format!(
                "Breaking changes detected on protected branch '{}' of service '{}': {}",
                branch, servicename, reason
            )));
        }
        tracing::info!(
            "Spec update is backward-compatible for protected branch '{}'",
            branch
        );
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
    // Aggregated SemVer impact for AsyncAPI provides (OpenAPI uses the merged
    // whole-spec `analyze_impact` below; proto keeps hash-based classification).
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
                    tracing::warn!(
                        "Rejected update for service '{}' branch '{}': breaking changes: {}",
                        servicename,
                        branch,
                        reason
                    );
                    return Err(AppError::BreakingChange(format!(
                        "Breaking changes detected on protected branch '{}' of service '{}': {}",
                        branch, servicename, reason
                    )));
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
            if is_protected
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
            return Err(AppError::BreakingChange(format!(
                "Removing non-deprecated endpoint {} {} is a breaking change on a protected branch; mark it deprecated first",
                method, path
            )));
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

    if inserts == 0 && updates == 0 && deletes == 0 && content_hash == last_hash {
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

    repo.apply_spec_changes(bid, changes, is_protected, username, Some(branch))
        .await?;

    // Apply the planned channel-message-contract mutations after the endpoint
    // changes commit (endpoints are the source of truth).
    for op in contract_ops {
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

    let impact = if api_type == ApiType::AsyncApi {
        async_impact
    } else if let Some(old_yaml) = old_full_yaml {
        openapi::analyze_impact(&old_yaml, content)
    } else if !last_hash.is_empty() {
        if content_hash != last_hash {
            Impact::Patch
        } else {
            Impact::None
        }
    } else {
        Impact::None
    };

    let new_version = repo
        .increment_spec_version(sid, bid, &content_hash, impact)
        .await?;

    tracing::info!(
        service = servicename,
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
    servicename: &str,
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
            service = servicename,
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

pub async fn get_endpoint_yaml(
    repo: &impl SpecRepository,
    servicename: &str,
    branch: &str,
    api_type: ApiType,
    path: &str,
    method: &str,
) -> Result<String, AppError> {
    let service_id = repo.ensure_service(servicename).await?;
    let method_to_use = match api_type {
        ApiType::OpenApi | ApiType::AsyncApi => method.to_uppercase(),
        ApiType::Proto => method.to_string(),
    };

    let endpoint = find_endpoint_with_fallback(
        repo,
        service_id,
        servicename,
        branch,
        api_type,
        path,
        &method_to_use,
    )
    .await?;
    endpoint.map(|(_, yaml, _, _)| yaml).ok_or_else(|| {
        AppError::NotFound(format!(
            "Endpoint not found: {} {} (service: {}, branch: {})",
            method_to_use, path, servicename, branch
        ))
    })
}

#[instrument(skip_all)]
pub async fn list_service_endpoints(
    repo: &impl SpecRepository,
    servicename: &str,
    branch: &str,
) -> Result<Vec<EndpointRecord>, AppError> {
    let service_id = repo.ensure_service(servicename).await?;
    let branch_id = repo.ensure_branch(service_id, branch).await?;
    Ok(repo.get_endpoints_for_branch(branch_id).await?)
}

/// Reassemble the full spec for a service/branch from its stored per-endpoint
/// specs. OpenAPI endpoints are merged into one document; AsyncAPI/proto are
/// concatenated (matching how `require_bundle` assembles them). Endpoints are
/// ordered deterministically so the output is stable.
pub async fn get_full_spec(
    repo: &impl SpecRepository,
    servicename: &str,
    branch: &str,
    api_type: ApiType,
) -> Result<String, AppError> {
    let mut endpoints: Vec<EndpointRecord> = list_service_endpoints(repo, servicename, branch)
        .await?
        .into_iter()
        .filter(|e| e.api_type == api_type)
        .collect();
    if endpoints.is_empty() {
        return Err(AppError::NotFound(format!(
            "No {} endpoints for service '{}' on branch '{}'",
            api_type.as_str(),
            servicename,
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
    servicename: &str,
    branch: &str,
    api_type: ApiType,
    path: &str,
    method: &str,
) -> Result<Vec<EndpointVersion>, AppError> {
    let service_id = repo.ensure_service(servicename).await?;
    let method_to_use = match api_type {
        ApiType::OpenApi | ApiType::AsyncApi => method.to_uppercase(),
        ApiType::Proto => method.to_string(),
    };

    // Prefer the requested branch's own history, but only if it actually has any:
    // version rows are only written on protected branches (see
    // `apply_spec_changes`), so a feature branch can genuinely hold the endpoint
    // (e.g. branched from `master` with identical content) yet have zero version
    // rows of its own. Checking existence alone (as `find_endpoint_with_fallback`
    // does, and would do again below) would stop right there and hand back that
    // same empty history.
    if let Some(branch_id) = repo.find_branch(service_id, branch).await?
        && let Some(endpoint_id) = repo
            .get_endpoint_id(branch_id, api_type, path, &method_to_use)
            .await?
    {
        let versions = repo.get_endpoint_versions(endpoint_id).await?;
        if !versions.is_empty() {
            return Ok(versions);
        }
    }

    // No local history — look for real history on the branch's source_protected_branch
    // hint (item #17), then the service's configured fallback branch, then any
    // protected branch (in priority order), explicitly skipping the branch
    // already checked above (existence there isn't enough).
    for candidate in fallback_branch_candidates(repo, service_id, servicename, branch).await? {
        if let Some(candidate_branch_id) = repo.find_branch(service_id, &candidate).await?
            && let Some(endpoint_id) = repo
                .get_endpoint_id(candidate_branch_id, api_type, path, &method_to_use)
                .await?
        {
            let versions = repo.get_endpoint_versions(endpoint_id).await?;
            if !versions.is_empty() {
                return Ok(versions);
            }
        }
    }

    // Nobody has real history — fall back to whatever endpoint exists at all
    // (e.g. a client-required branch the server never published, or the local
    // branch's own content with no recorded history). The returned versions
    // carry the branch they actually live on (`branch_name`), so the caller can
    // tell a fallback happened.
    let endpoint_id = find_endpoint_with_fallback(
        repo,
        service_id,
        servicename,
        branch,
        api_type,
        path,
        &method_to_use,
    )
    .await?
    .map(|(id, _, _, _)| id)
    .ok_or_else(|| AppError::NotFound("Endpoint not found".to_string()))?;

    Ok(repo.get_endpoint_versions(endpoint_id).await?)
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
        repo.ensure_client(params.clientname).await?
    };
    let service_id = if dry_run {
        repo.find_service(params.servicename)
            .await?
            .ok_or_else(|| AppError::NotFound("Service not found".to_string()))?
    } else {
        repo.ensure_service(params.servicename).await?
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
            params.servicename,
            params.branch,
            Some(hint),
        )
        .await?;
    }

    let deadline = params
        .timeout_secs
        .map(|s| std::time::Instant::now() + std::time::Duration::from_secs(s));
    let poll_interval = std::time::Duration::from_millis(500);

    loop {
        let endpoint = if let Some(pull_from_branch) = params.pull_from_branch {
            repo.find_endpoint(
                service_id,
                pull_from_branch,
                params.api_type,
                params.path,
                &method_to_use,
            )
            .await?
        } else {
            find_endpoint_with_fallback(
                repo,
                service_id,
                params.servicename,
                params.branch,
                params.api_type,
                params.path,
                &method_to_use,
            )
            .await?
        };

        if let Some((id, yaml, deprecated, external)) = endpoint {
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
                    service = params.servicename,
                    branch = params.branch,
                    "Client '{}' required {:?} {} {} on branch '{}'",
                    params.clientname,
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
            method_to_use, params.path, params.servicename, params.branch
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
        .find_service(params.servicename)
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
        repo.ensure_client(params.clientname).await?
    };
    let service_id = if dry_run {
        repo.find_service(params.servicename)
            .await?
            .ok_or_else(|| AppError::NotFound("Service not found".to_string()))?
    } else {
        repo.ensure_service(params.servicename).await?
    };

    let deadline = params
        .timeout_secs
        .map(|s| std::time::Instant::now() + std::time::Duration::from_secs(s));
    let poll_interval = std::time::Duration::from_millis(500);

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

        let found_endpoints = find_endpoints_bulk_with_fallback(
            repo,
            service_id,
            params.servicename,
            params.branch,
            params.api_type,
            &normalized_endpoints,
        )
        .await?;

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
            return Ok(RequireResponse {
                yaml: merged_yaml,
                deprecated: any_deprecated,
                external: any_external,
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
    servicename: &str,
    branch: &str,
) -> Result<Vec<String>, RepositoryError> {
    let mut candidates = Vec::new();
    if let Some(branch_id) = repo.find_branch(service_id, branch).await?
        && let Some(spb) = repo.get_source_protected_branch(branch_id).await?
        && spb != branch
    {
        candidates.push(spb);
    }
    if let Ok(Some(sfb)) = repo.get_fallback_branch(servicename).await
        && sfb != branch
        && !candidates.contains(&sfb)
    {
        candidates.push(sfb);
    }
    for pb in repo.list_protected_branches().await? {
        if pb != branch && !candidates.contains(&pb) {
            candidates.push(pb);
        }
    }
    Ok(candidates)
}

async fn find_endpoint_with_fallback(
    repo: &impl SpecRepository,
    service_id: i64,
    servicename: &str,
    branch: &str,
    api_type: ApiType,
    path: &str,
    method_to_use: &str,
) -> Result<Option<(i64, String, bool, bool)>, RepositoryError> {
    let endpoint = repo
        .find_endpoint(service_id, branch, api_type, path, method_to_use)
        .await?;
    if endpoint.is_some() || repo.is_branch_protected(branch).await? {
        return Ok(endpoint);
    }

    for candidate in fallback_branch_candidates(repo, service_id, servicename, branch).await? {
        if let Some(ep) = repo
            .find_endpoint(service_id, &candidate, api_type, path, method_to_use)
            .await?
        {
            return Ok(Some(ep));
        }
    }
    Ok(None)
}

async fn find_endpoints_bulk_with_fallback(
    repo: &impl SpecRepository,
    service_id: i64,
    servicename: &str,
    branch: &str,
    api_type: ApiType,
    endpoints: &[(String, String)],
) -> Result<crate::domain::ports::EndpointMap, RepositoryError> {
    let mut results = repo
        .find_endpoints_bulk(service_id, branch, api_type, endpoints)
        .await?;
    let mut missing: Vec<(String, String)> = endpoints
        .iter()
        .filter(|e| !results.contains_key(&(e.0.clone(), e.1.clone())))
        .cloned()
        .collect();
    if missing.is_empty() || repo.is_branch_protected(branch).await? {
        return Ok(results);
    }

    if let Some(sfb) = repo
        .get_fallback_branch(servicename)
        .await
        .ok()
        .flatten()
        .filter(|b| b != branch)
    {
        let fallback_results = repo
            .find_endpoints_bulk(service_id, &sfb, api_type, &missing)
            .await?;
        for (key, val) in fallback_results {
            results.insert(key.clone(), val);
            missing.retain(|m| m.0 != key.0 || m.1 != key.1);
        }
    }

    if missing.is_empty() {
        return Ok(results);
    }

    let protected = repo.list_protected_branches().await?;
    for pb in &protected {
        if pb == branch {
            continue;
        }
        let fallback_results = repo
            .find_endpoints_bulk(service_id, pb, api_type, &missing)
            .await?;
        for (key, val) in fallback_results {
            results.insert(key.clone(), val);
            missing.retain(|m| m.0 != key.0 || m.1 != key.1);
        }
        if missing.is_empty() {
            break;
        }
    }
    Ok(results)
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
        let endpoints = list_service_endpoints(&repo, "svc", "feat").await.unwrap();
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
        let services = repo.list_services().await.unwrap();
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
