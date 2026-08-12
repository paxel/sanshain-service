//! The Require use case (ADR-0003): resolve a Pin and hand back exactly the
//! endpoint asked for, recording the dependency it implies.
//!
//! Split out of `spec_service` so the read-side flow is not interleaved with
//! the publish-side rules it has nothing to do with — a Require never
//! validates a version, it only resolves one that already exists.

use super::now_iso;
use super::spec_service::resolve_stream;
use crate::domain::models::*;
use crate::domain::ports::{RecordDependencyParams, RecordTrunkPinParams, SpecRepository};
use crate::openapi;
use std::collections::HashSet;

pub struct RequireEndpointParams<'a> {
    pub consumername: &'a str,
    pub producername: &'a str,
    pub version: SemVer,
    pub api_type: ApiType,
    pub path: &'a str,
    pub method: &'a str,
    /// ADR-0004: this build belongs to the trunk stream — the pin is also
    /// recorded in the append-only trunk store (last-write-wins view).
    pub trunk: bool,
    /// ADR-0005: the pin updates the named sanshain-branch's graph instead.
    /// Mutually exclusive with `trunk`.
    pub tag: Option<&'a str>,
}

pub struct RequireBundleParams<'a> {
    pub consumername: &'a str,
    pub producername: &'a str,
    pub version: SemVer,
    pub api_type: ApiType,
    pub endpoints: &'a [(String, String)],
    /// See [`RequireEndpointParams::trunk`].
    pub trunk: bool,
    /// See [`RequireEndpointParams::tag`].
    pub tag: Option<&'a str>,
}

/// Resolve a Producer's version-line entry for a Pin, or fail the way the
/// contract promises: `404` when the version does not exist in either
/// stability. Never creates anything.
pub(crate) async fn resolve_pin(
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

pub(crate) fn method_for(api_type: ApiType, method: &str) -> String {
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
        trunk,
        tag,
    } = params;
    let tag_branch = resolve_stream(repo, trunk, tag).await?;

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
            trunk,
            tag_branch.as_ref().map(|b| b.id),
        )
        .await?;
        if let Some(branch) = &tag_branch {
            count_branch_update(&branch.name);
        }
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
        trunk,
        tag,
    } = params;
    let tag_branch = resolve_stream(repo, trunk, tag).await?;

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
        // The merge re-parses every snippet — CPU work, off the async worker.
        ApiType::OpenApi => {
            super::spec_service::run_cpu_bound(move || {
                openapi::merge_endpoint_yamls(&yamls).map_err(AppError::Internal)
            })
            .await?
        }
        _ => yamls.join("\n---\n"),
    };

    if !dry_run {
        record_pins(
            repo,
            consumername,
            &entry,
            &normalized,
            trunk,
            tag_branch.as_ref().map(|b| b.id),
        )
        .await?;
        if let Some(branch) = &tag_branch {
            count_branch_update(&branch.name);
        }
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
    trunk: bool,
    tag_branch_id: Option<i64>,
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
    // A trunk build additionally maintains the append-only trunk pin set
    // (ADR-0004); a tagged build updates its branch's graph instead
    // (ADR-0005) — both recorded by version value, never by row id.
    if trunk || tag_branch_id.is_some() {
        let now = now_iso();
        // The pin store keys on `(normalized_path, method)`, so two bundle
        // entries that differ only in a path-parameter name are one pin. Left
        // in, the second would hit the store's refresh branch and vanish while
        // the dev graph kept both — the two views would then disagree about
        // the same require call. Collapsed here instead, keeping the first
        // spelling for display.
        let mut seen: HashSet<(&str, &str)> = HashSet::new();
        let pins: Vec<RecordTrunkPinParams> = endpoints
            .iter()
            .zip(normalized.iter())
            .filter(|((_, method), normalized_path)| {
                seen.insert((normalized_path.as_str(), method.as_str()))
            })
            .map(|((path, method), normalized_path)| RecordTrunkPinParams {
                client_id,
                service_id: entry.service_id,
                api_type: entry.api_type,
                version: entry.version,
                path,
                normalized_path,
                method,
                now_iso: &now,
            })
            .collect();
        match tag_branch_id {
            Some(branch_id) => repo.record_branch_pins(branch_id, pins).await?,
            None => repo.record_trunk_pins(pins).await?,
        }
    }
    Ok(())
}

/// One branch update happened (tagged provide or require) — the counter the
/// observability surface graphs per branch.
pub(crate) fn count_branch_update(branch: &str) {
    metrics::counter!("sanshain_branch_updates_total", "branch" => branch.to_string()).increment(1);
}
