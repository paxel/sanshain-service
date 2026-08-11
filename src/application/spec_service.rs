//! Read/query use-cases of the version-line model (ADR-0003): endpoint views,
//! version listing, diffing, endpoint history and the provide timeline, plus
//! spec-parsing/stream helpers shared with the Provide and Require use-cases.
//!
//! The Provide use case lives in `provide_service`, Require in
//! `require_service`; both are re-exported through `services` (and the Provide
//! entry points are also re-exported here for compatibility).

use super::require_service::{method_for, resolve_pin};
use crate::domain::models::*;
use crate::domain::ports::SpecRepository;
use crate::openapi;
use sha2::{Digest, Sha256};
use std::collections::HashMap;

// Compat re-exports: the Provide use case moved to provide_service (#16);
// existing `spec_service::…` call sites (handlers via services, and tests)
// keep resolving through here.
pub use super::provide_service::{ProvideSpecParams, promote_version, provide_spec};

use tracing::instrument;

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

pub(crate) fn content_hash(content: &str) -> String {
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
pub(crate) fn extract_spec_version(api_type: ApiType, content: &str) -> Result<SemVer, AppError> {
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

pub(crate) fn parse_spec_endpoints(
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
    // These exercise the pure version-rule helpers (now in version_rules).
    use super::super::version_rules::{classify_change, diff_endpoints, ga_baseline, propose_free};

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
        let changes = diff_endpoints(&old, &new);
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
