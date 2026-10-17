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
use crate::{asyncapi, openapi};
use std::collections::HashSet;

/// The shape a require answers in — the `format` query parameter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequireFormat {
    /// The stored snippets in their own API type (no `format`).
    Native,
    /// `format=openapi`: an AsyncAPI require answered as an OpenAPI 3.0.3
    /// document holding the kept messages' payload schemas.
    OpenApi,
}

impl RequireFormat {
    /// Read the `format` parameter of a require for `api_type`. Only
    /// `openapi`, and only for AsyncAPI, is supported; anything else is a
    /// `400` naming what is.
    pub fn parse(api_type: ApiType, format: Option<&str>) -> Result<Self, AppError> {
        match format {
            None => Ok(RequireFormat::Native),
            Some("openapi") if api_type == ApiType::AsyncApi => Ok(RequireFormat::OpenApi),
            Some("openapi") => Err(AppError::BadRequest(format!(
                "format=openapi is only supported for apiType asyncapi, not {}",
                api_type.as_str()
            ))),
            Some(other) => Err(AppError::BadRequest(format!(
                "Unknown format '{}': the only supported format is 'openapi' (for apiType asyncapi)",
                other
            ))),
        }
    }

    /// The name keying this format's ETag apart from the native answer's.
    pub fn as_str(self) -> &'static str {
        match self {
            RequireFormat::Native => "native",
            RequireFormat::OpenApi => "openapi",
        }
    }
}

/// Convert resolved AsyncAPI snippets to the `format=openapi` document.
/// Called before any Pin is recorded, so a spec that cannot be converted
/// records nothing.
async fn to_openapi(
    producername: &str,
    version: SemVer,
    snippets: Vec<String>,
) -> Result<String, AppError> {
    let service = producername.to_string();
    let version = version.to_string();
    // Parses and walks every snippet — CPU work, off the async worker.
    super::run_cpu_bound(move || {
        asyncapi::to_openapi_schemas(&snippets, &service, &version).map_err(AppError::BadRequest)
    })
    .await
}

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
    /// The shape of the answer (see [`RequireFormat`]).
    pub format: RequireFormat,
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
    /// See [`RequireEndpointParams::format`].
    pub format: RequireFormat,
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
    /// The format `yaml` is in — keys the ETag apart per format.
    pub format: RequireFormat,
    /// The stream the pins were recorded in, and the warnings when the
    /// declared one could not be honoured (`Sanshain-Warning` headers).
    pub stream: ResolvedStream,
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
        format,
    } = params;
    // Dev only when the declared stream cannot be honoured (ADR-0005).
    let stream = resolve_stream(repo, trunk, tag).await?;
    let trunk = stream.trunk;

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
    let yaml = match format {
        RequireFormat::Native => yaml,
        RequireFormat::OpenApi => to_openapi(producername, version, vec![yaml]).await?,
    };

    if !dry_run {
        record_pins(
            repo,
            consumername,
            &entry,
            &[(path.to_string(), method.clone())],
            trunk,
            stream.branch.as_ref().map(|b| b.id),
        )
        .await?;
        if let Some(branch) = &stream.branch {
            count_branch_update(&branch.name);
        }
    }

    Ok(RequireResponse {
        yaml,
        deprecated,
        state: ResolutionState::Served,
        version,
        stability: entry.stability,
        format,
        stream,
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
        format,
    } = params;
    // Dev only when the declared stream cannot be honoured (ADR-0005).
    let stream = resolve_stream(repo, trunk, tag).await?;
    let trunk = stream.trunk;

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

    let yaml = match (format, api_type) {
        (RequireFormat::OpenApi, _) => to_openapi(producername, version, yamls).await?,
        // The merge re-parses every snippet — CPU work, off the async worker.
        (RequireFormat::Native, ApiType::OpenApi) => {
            super::run_cpu_bound(move || {
                openapi::merge_endpoint_yamls(&yamls).map_err(AppError::Internal)
            })
            .await?
        }
        (RequireFormat::Native, _) => yamls.join("\n---\n"),
    };

    if !dry_run {
        record_pins(
            repo,
            consumername,
            &entry,
            &normalized,
            trunk,
            stream.branch.as_ref().map(|b| b.id),
        )
        .await?;
        if let Some(branch) = &stream.branch {
            count_branch_update(&branch.name);
        }
    }

    Ok(RequireResponse {
        yaml,
        deprecated: any_deprecated,
        state: ResolutionState::Served,
        version,
        stability: entry.stability,
        format,
        stream,
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
    repo.record_dependencies_bulk(deps, Vec::new()).await?;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn require_format_defaults_to_native_and_accepts_openapi_for_asyncapi() {
        assert_eq!(
            RequireFormat::parse(ApiType::OpenApi, None).ok(),
            Some(RequireFormat::Native)
        );
        assert_eq!(
            RequireFormat::parse(ApiType::AsyncApi, Some("openapi")).ok(),
            Some(RequireFormat::OpenApi)
        );
        assert_eq!(RequireFormat::Native.as_str(), "native");
        assert_eq!(RequireFormat::OpenApi.as_str(), "openapi");
    }

    #[test]
    fn require_format_rejects_other_api_types_and_unknown_formats() {
        for api_type in [ApiType::OpenApi, ApiType::Proto] {
            match RequireFormat::parse(api_type, Some("openapi")) {
                Err(AppError::BadRequest(msg)) => {
                    assert!(msg.contains("only supported for apiType asyncapi"), "{msg}")
                }
                other => panic!("expected 400, got {other:?}"),
            }
        }
        match RequireFormat::parse(ApiType::AsyncApi, Some("json")) {
            Err(AppError::BadRequest(msg)) => {
                assert!(msg.contains("Unknown format 'json'"), "{msg}")
            }
            other => panic!("expected 400, got {other:?}"),
        }
    }
}
