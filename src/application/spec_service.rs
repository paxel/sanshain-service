use crate::domain::models::*;
use crate::domain::ports::{
    RecordDependencyParams, RepositoryError, SpecRepository, UpdateEndpointParams,
};
use crate::openapi;
use sha2::{Digest, Sha256};
use std::collections::HashMap;

use tracing::instrument;

pub struct RequireEndpointParams<'a> {
    pub clientname: &'a str,
    pub servicename: &'a str,
    pub branch: &'a str,
    pub api_type: ApiType,
    pub path: &'a str,
    pub method: &'a str,
    pub timeout_secs: Option<u64>,
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
                "Failed to split OpenAPI for service '{}' branch '{}': {}. Input content: \n{}",
                servicename,
                branch,
                e,
                content
            );
            AppError::BadRequest(e)
        }),
        ApiType::AsyncApi => {
            let all_specs = crate::asyncapi::split_asyncapi(content).map_err(|e| {
                tracing::warn!(
                    "Failed to split AsyncAPI for service '{}' branch '{}': {}. Input content: \n{}",
                    servicename,
                    branch,
                    e,
                    content
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
                    deprecated: false,
                })
                .collect())
        }
        ApiType::Proto => Ok(crate::proto::split_proto(content)
            .map_err(|e| {
                tracing::warn!(
                    "Failed to split Proto for service '{}' branch '{}': {}. Input content: \n{}",
                    servicename,
                    branch,
                    e,
                    content
                );
                AppError::BadRequest(e)
            })?
            .into_iter()
            .map(|s| openapi::EndpointSpec {
                normalized_path: s.service.clone(),
                path: s.service,
                method: s.method,
                yaml_content: s.content,
                deprecated: false,
            })
            .collect()),
    }
}

async fn has_protected_branch_endpoints(
    repo: &impl SpecRepository,
    service_id: i64,
) -> Result<bool, AppError> {
    let protected_branches = repo.list_protected_branches().await?;
    for pb in &protected_branches {
        if let Some(bid) = repo.find_branch(service_id, pb).await? {
            let endpoints = repo.get_endpoints_for_branch(bid).await?;
            if !endpoints.is_empty() {
                return Ok(true);
            }
        }
    }
    Ok(false)
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

    let mut existing_map: HashMap<(ApiType, String, String), (String, String)> = existing
        .into_iter()
        .map(|e| {
            (
                (e.api_type, e.normalized_path, e.method),
                (e.path, e.yaml_content),
            )
        })
        .collect();

    let mut changes = Vec::new();
    let mut inserts = 0;
    let mut updates = 0;

    let skip_compat = if !is_protected && !dry_run {
        force || !has_protected_branch_endpoints(repo, sid).await?
    } else {
        false
    };

    for endpoint in endpoints {
        if !is_protected && !dry_run {
            if skip_compat {
                repo.upsert_shared_contract(SharedContract {
                    branch_name: branch.to_string(),
                    service_id: sid,
                    api_type,
                    path: endpoint.normalized_path.clone(),
                    method: endpoint.method.clone(),
                    source_yaml: endpoint.yaml_content.clone(),
                    current_yaml: endpoint.yaml_content.clone(),
                    owner_service_id: None,
                })
                .await?;
            } else {
                let shared = repo
                    .get_shared_contract(
                        branch,
                        sid,
                        api_type,
                        &endpoint.normalized_path,
                        &endpoint.method,
                    )
                    .await?;
                match shared {
                    None => {
                        repo.upsert_shared_contract(SharedContract {
                            branch_name: branch.to_string(),
                            service_id: sid,
                            api_type,
                            path: endpoint.normalized_path.clone(),
                            method: endpoint.method.clone(),
                            source_yaml: endpoint.yaml_content.clone(),
                            current_yaml: endpoint.yaml_content.clone(),
                            owner_service_id: None,
                        })
                        .await?;
                    }
                    Some(mut entry) => {
                        if endpoint.yaml_content == entry.source_yaml {
                            if entry.owner_service_id == Some(sid) {
                                entry.current_yaml = entry.source_yaml.clone();
                                entry.owner_service_id = None;
                                repo.upsert_shared_contract(entry).await?;
                            }
                        } else if endpoint.yaml_content != entry.current_yaml {
                            let is_owner = entry.owner_service_id == Some(sid);
                            let check_against = if is_owner || entry.owner_service_id.is_none() {
                                &entry.source_yaml
                            } else {
                                &entry.current_yaml
                            };

                            if let Err(reason) =
                                check_compatibility(api_type, check_against, &endpoint.yaml_content)
                            {
                                tracing::warn!("Rejected shared endpoint modification: {}", reason);
                                let owner_desc = if is_owner || entry.owner_service_id.is_none() {
                                    "source"
                                } else {
                                    "current owner's"
                                };
                                return Err(AppError::BreakingChange(format!(
                                    "Breaking change for shared endpoint {} {} on branch '{}' (compared to {} version): {}",
                                    endpoint.method, endpoint.path, branch, owner_desc, reason
                                )));
                            }

                            entry.current_yaml = endpoint.yaml_content.clone();
                            entry.owner_service_id = Some(sid);
                            repo.upsert_shared_contract(entry).await?;
                        }
                    }
                }
            }
        }

        let key = (
            api_type,
            endpoint.normalized_path.clone(),
            endpoint.method.clone(),
        );
        if let Some((old_path, existing_yaml)) = existing_map.remove(&key) {
            if existing_yaml != endpoint.yaml_content || old_path != endpoint.path {
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
    for ((old_api_type, _norm_path, method), (path, _)) in existing_map {
        if old_api_type != api_type {
            continue;
        }

        if is_protected && !dry_run {
            return Err(AppError::BreakingChange(format!(
                "Removing endpoint {} {} is a breaking change on a protected branch",
                method, path
            )));
        }

        if !is_protected
            && !dry_run
            && let Some(mut entry) = repo
                .get_shared_contract(branch, sid, old_api_type, &_norm_path, &method)
                .await?
        {
            entry.current_yaml = entry.source_yaml.clone();
            entry.owner_service_id = None;
            repo.upsert_shared_contract(entry).await?;
        }
        changes.push(SpecChange::Delete {
            api_type,
            path,
            method,
            soft_delete: is_protected,
        });
        deletes += 1;
    }

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

    let impact = if let Some(old_yaml) = old_full_yaml {
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

pub async fn get_endpoint_version_history(
    repo: &impl SpecRepository,
    servicename: &str,
    branch: &str,
    api_type: ApiType,
    path: &str,
    method: &str,
) -> Result<Vec<EndpointVersion>, AppError> {
    let service_id = repo.ensure_service(servicename).await?;
    let branch_id = repo.ensure_branch(service_id, branch).await?;
    let method_to_use = match api_type {
        ApiType::OpenApi | ApiType::AsyncApi => method.to_uppercase(),
        ApiType::Proto => method.to_string(),
    };

    let endpoint_id = repo
        .get_endpoint_id(branch_id, api_type, path, &method_to_use)
        .await?
        .ok_or_else(|| AppError::NotFound("Endpoint not found".to_string()))?;

    Ok(repo.get_endpoint_versions(endpoint_id).await?)
}

pub async fn get_audit_timeline(
    repo: &impl SpecRepository,
    limit: u32,
) -> Result<Vec<EndpointVersion>, AppError> {
    Ok(repo.get_global_endpoint_versions(limit).await?)
}

pub async fn get_shared_contract_info(
    repo: &impl SpecRepository,
    servicename: &str,
    branch: &str,
    api_type: ApiType,
    path: &str,
    method: &str,
) -> Result<Option<SharedContractInfo>, AppError> {
    let service_id = match repo.find_service(servicename).await? {
        Some(id) => id,
        None => return Ok(None),
    };

    let method_to_use = match api_type {
        ApiType::OpenApi | ApiType::AsyncApi => method.to_uppercase(),
        ApiType::Proto => method.to_string(),
    };

    let contract = repo
        .get_shared_contract(branch, service_id, api_type, path, &method_to_use)
        .await?;

    match contract {
        None => Ok(None),
        Some(c) => {
            let owner_service = match c.owner_service_id {
                Some(owner_id) => repo.get_service_name_by_id(owner_id).await?,
                None => None,
            };
            let has_changes = c.source_yaml != c.current_yaml;
            Ok(Some(SharedContractInfo {
                source_yaml: c.source_yaml,
                current_yaml: c.current_yaml,
                owner_service,
                has_changes,
            }))
        }
    }
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

    let deadline = params
        .timeout_secs
        .map(|s| std::time::Instant::now() + std::time::Duration::from_secs(s));
    let poll_interval = std::time::Duration::from_millis(500);

    loop {
        let endpoint = find_endpoint_with_fallback(
            repo,
            service_id,
            params.servicename,
            params.branch,
            params.api_type,
            params.path,
            &method_to_use,
        )
        .await?;

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

    if let Ok(Some(sfb)) = repo.get_fallback_branch(servicename).await
        && sfb != branch
        && let Some(ep) = repo
            .find_endpoint(service_id, &sfb, api_type, path, method_to_use)
            .await?
    {
        return Ok(Some(ep));
    }

    let protected = repo.list_protected_branches().await?;
    for pb in &protected {
        if pb != branch
            && let Some(ep) = repo
                .find_endpoint(service_id, pb, api_type, path, method_to_use)
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
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::mock_repo::MockRepo;

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
    async fn test_list_endpoints_with_changes() {
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

        // 3. Simulate shared contract with changes for /hello
        let service_id = repo.find_service("svc").await.unwrap().unwrap();
        repo.upsert_shared_contract(SharedContract {
            branch_name: "feat".to_string(),
            service_id,
            api_type: ApiType::OpenApi,
            path: "/hello".to_string(),
            method: "GET".to_string(),
            source_yaml: "old".to_string(),
            current_yaml: "new".to_string(),
            owner_service_id: None,
        })
        .await
        .unwrap();

        // 4. List endpoints and verify has_changes
        let endpoints = list_service_endpoints(&repo, "svc", "feat").await.unwrap();
        let hello = endpoints.iter().find(|e| e.path == "/hello").unwrap();
        assert!(hello.has_changes);

        let world = endpoints.iter().find(|e| e.path == "/world").unwrap();
        assert!(!world.has_changes);
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
    async fn test_check_compatibility_asyncapi_always_ok() {
        let result = check_compatibility(ApiType::AsyncApi, "old", "new");
        assert!(result.is_ok());
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
}
