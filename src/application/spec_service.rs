use crate::domain::models::*;
use crate::domain::ports::{RecordDependencyParams, RepositoryError, SpecRepository};
use crate::openapi;
use sha2::{Digest, Sha256};
use std::collections::HashMap;

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

pub async fn provide_spec(
    repo: &impl SpecRepository,
    servicename: &str,
    branch: &str,
    api_type: ApiType,
    content: &str,
    base_version: Option<i32>,
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
        },
    )
    .await
}

pub async fn provide_spec_with_tags(
    repo: &impl SpecRepository,
    servicename: &str,
    branch: &str,
    api_type: ApiType,
    content: &str,
    tags: &[String],
    base_version: Option<i32>,
) -> Result<ProvideResponse, AppError> {
    provide_spec_inner(
        repo,
        ProvideInternalParams {
            servicename,
            branch,
            api_type,
            content,
            dry_run: false,
            extra_tags: tags,
            base_version,
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
    pub base_version: Option<i32>,
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
            })
            .collect()),
    }
}

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
            None => (0, String::new()),
        }
    } else {
        (0, String::new())
    };

    if let Some(base_v) = base_version
        && base_v != current_version
    {
        return Err(AppError::Conflict(format!(
            "Outdated spec version: your base version is {}, but current version is {}. Pull latest changes.",
            base_v, current_version
        )));
    }

    let is_protected = repo.is_branch_protected(branch).await?;

    if is_protected && api_type == ApiType::OpenApi {
        let existing = repo.get_endpoints_for_branch(bid).await?;
        if !existing.is_empty() {
            let existing_yamls: Vec<String> = existing
                .into_iter()
                .filter(|e| e.api_type == ApiType::OpenApi)
                .map(|e| e.yaml_content)
                .collect();

            if !existing_yamls.is_empty() {
                let old_full_yaml =
                    openapi::merge_endpoint_yamls(&existing_yamls).map_err(|e| {
                        AppError::Internal(format!(
                            "Failed to merge existing endpoints for compatibility check: {}",
                            e
                        ))
                    })?;

                if let Err(reason) = openapi::check_backward_compatibility(&old_full_yaml, content)
                {
                    tracing::warn!(
                        "Rejected update for service '{}' branch '{}': breaking changes: {}",
                        servicename,
                        branch,
                        reason
                    );
                    return Err(AppError::Conflict(format!(
                        "Breaking changes detected on protected branch '{}' of service '{}': {}",
                        branch, servicename, reason
                    )));
                }
                tracing::info!(
                    "Spec update is backward-compatible for protected branch '{}'",
                    branch
                );
            }
        }
    }

    let existing = repo.get_endpoints_for_branch(bid).await?;
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

    for endpoint in endpoints {
        if !is_protected && !dry_run {
            let shared = repo
                .get_shared_contract(
                    branch,
                    api_type,
                    &endpoint.normalized_path,
                    &endpoint.method,
                )
                .await?;
            match shared {
                None => {
                    repo.upsert_shared_contract(SharedContract {
                        branch_name: branch.to_string(),
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
                            return Err(AppError::Conflict(format!(
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
            });
            inserts += 1;
        }
    }

    let mut deletes = 0;
    for ((old_api_type, _norm_path, method), (path, _)) in existing_map {
        if old_api_type != api_type {
            continue;
        }
        if !is_protected
            && !dry_run
            && let Some(mut entry) = repo
                .get_shared_contract(branch, old_api_type, &_norm_path, &method)
                .await?
            && entry.owner_service_id == Some(sid)
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

    repo.apply_spec_changes(bid, changes, is_protected).await?;
    let new_version = repo.increment_spec_version(sid, bid, &content_hash).await?;

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
    endpoint.map(|(_, yaml)| yaml).ok_or_else(|| {
        AppError::NotFound(format!("Endpoint not found: {} {}", method_to_use, path))
    })
}

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

pub async fn require_endpoint(
    repo: &impl SpecRepository,
    notifier: Option<tokio::sync::broadcast::Receiver<()>>,
    params: RequireEndpointParams<'_>,
) -> Result<String, AppError> {
    require_endpoint_inner(repo, notifier, params, false).await
}

pub async fn require_endpoint_dry_run(
    repo: &impl SpecRepository,
    notifier: Option<tokio::sync::broadcast::Receiver<()>>,
    params: RequireEndpointParams<'_>,
) -> Result<String, AppError> {
    require_endpoint_inner(repo, notifier, params, true).await
}

async fn require_endpoint_inner(
    repo: &impl SpecRepository,
    mut notifier: Option<tokio::sync::broadcast::Receiver<()>>,
    params: RequireEndpointParams<'_>,
    dry_run: bool,
) -> Result<String, AppError> {
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

        if let Some((id, yaml)) = endpoint {
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
            return Ok(yaml);
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
            "Endpoint not found: {} {}",
            method_to_use, params.path
        )));
    }
}

pub async fn require_bundle(
    repo: &impl SpecRepository,
    notifier: Option<tokio::sync::broadcast::Receiver<()>>,
    params: RequireBundleParams<'_>,
) -> Result<String, AppError> {
    require_bundle_inner(repo, notifier, params, false).await
}

pub async fn require_bundle_dry_run(
    repo: &impl SpecRepository,
    notifier: Option<tokio::sync::broadcast::Receiver<()>>,
    params: RequireBundleParams<'_>,
) -> Result<String, AppError> {
    require_bundle_inner(repo, notifier, params, true).await
}

async fn require_bundle_inner(
    repo: &impl SpecRepository,
    mut notifier: Option<tokio::sync::broadcast::Receiver<()>>,
    params: RequireBundleParams<'_>,
    dry_run: bool,
) -> Result<String, AppError> {
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
            for (path, method) in params.endpoints {
                let m_use = match params.api_type {
                    ApiType::OpenApi | ApiType::AsyncApi => method.to_uppercase(),
                    ApiType::Proto => method.to_string(),
                };
                let (id, yaml) = found_endpoints
                    .get(&(path.clone(), m_use.clone()))
                    .ok_or_else(|| {
                        AppError::Internal(format!(
                            "endpoint unexpectedly missing: {} {}",
                            m_use, path
                        ))
                    })?;
                yamls.push(yaml.clone());
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
            return match params.api_type {
                ApiType::OpenApi => {
                    openapi::merge_endpoint_yamls(&yamls).map_err(AppError::Internal)
                }
                _ => Ok(yamls.join("\n---\n")),
            };
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
) -> Result<Option<(i64, String)>, RepositoryError> {
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
        let resp = provide_spec(&repo, "svc", "main", ApiType::OpenApi, SIMPLE_OPENAPI, None)
            .await
            .unwrap();
        assert_eq!(resp.version, 1);
        assert_eq!(resp.changes.inserts, 1);
        assert_eq!(resp.changes.updates, 0);
        assert_eq!(resp.changes.deletes, 0);
    }

    #[tokio::test]
    async fn test_provide_spec_update() {
        let repo = MockRepo::new();
        provide_spec(&repo, "svc", "main", ApiType::OpenApi, SIMPLE_OPENAPI, None)
            .await
            .unwrap();
        let resp = provide_spec(
            &repo,
            "svc",
            "main",
            ApiType::OpenApi,
            UPDATED_OPENAPI,
            None,
        )
        .await
        .unwrap();
        assert_eq!(resp.version, 2);
        assert!(resp.changes.inserts > 0 || resp.changes.updates > 0);
    }

    #[tokio::test]
    async fn test_provide_spec_no_change_skip() {
        let repo = MockRepo::new();
        let r1 = provide_spec(&repo, "svc", "main", ApiType::OpenApi, SIMPLE_OPENAPI, None)
            .await
            .unwrap();
        let r2 = provide_spec(&repo, "svc", "main", ApiType::OpenApi, SIMPLE_OPENAPI, None)
            .await
            .unwrap();
        assert_eq!(r1.version, r2.version);
        assert_eq!(r2.changes.inserts, 0);
        assert_eq!(r2.changes.updates, 0);
        assert_eq!(r2.changes.deletes, 0);
    }

    #[tokio::test]
    async fn test_provide_spec_dry_run() {
        let repo = MockRepo::new();
        let resp = provide_spec_dry_run(&repo, "svc", "main", ApiType::OpenApi, SIMPLE_OPENAPI)
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
