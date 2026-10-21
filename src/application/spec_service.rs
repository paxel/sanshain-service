//! Read/query use-cases of the version-line model (ADR-0003): endpoint views,
//! version listing, diffing, endpoint history and the provide timeline, plus
//! spec-parsing/stream helpers shared with the Provide and Require use-cases.
//!
//! The Provide use case lives in `provide_service`, Require in
//! `require_service`; both are re-exported through `services` (and the Provide
//! entry points are also re-exported here for compatibility).

use super::require_service::{method_for, resolve_pin};
use super::text_diff::{DiffHunk, DiffOptions, blame_lines, diff_lines, group_blame};
use crate::domain::models::*;
use crate::domain::ports::{RewriteSpecText, SpecRepository};
use crate::openapi;
use sha2::{Digest, Sha256};
use std::borrow::Cow;
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

/// The one normalization every spec document goes through on arrival, before
/// it is hashed, validated, split or stored (ADR-0008): a leading UTF-8 BOM is
/// dropped, and CRLF and lone CR line endings become LF. The same document
/// therefore hashes the same from every platform, and a Windows checkout can
/// no longer turn an identical GA re-provide into an immutability conflict.
/// Borrows when there is nothing to change.
pub fn normalize_spec_text(content: &str) -> Cow<'_, str> {
    let without_bom = content.strip_prefix('\u{feff}').unwrap_or(content);
    if !without_bom.contains('\r') {
        return Cow::Borrowed(without_bom);
    }
    Cow::Owned(without_bom.replace("\r\n", "\n").replace('\r', "\n"))
}

/// Settings key recording that the one-time normalization of stored spec text
/// (ADR-0008) has run, so it never runs again.
pub const SPEC_TEXT_NORMALIZED_SETTING: &str = "spec_text_normalized_at";

/// What the one-time normalization rewrote.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct SpecTextNormalization {
    /// True when an earlier run was recorded and nothing was looked at.
    pub already_done: bool,
    pub versions: usize,
    pub endpoints: usize,
    pub contracts: usize,
}

/// Bring every stored spec text in line with [`normalize_spec_text`], once:
/// documents (with their `content_hash` recomputed), endpoint snippets and
/// channel-message contract payloads. Without it a document stored with CRLF
/// before normalization existed would answer a CRLF re-provide of its GA with
/// a 409 forever. Rows keep their timestamps and attribution — this is a
/// storage fix, not a provide. Recorded under
/// [`SPEC_TEXT_NORMALIZED_SETTING`] only after everything succeeded; a failed
/// run is simply repeated on the next start (every step is idempotent).
#[instrument(skip_all)]
pub async fn normalize_stored_spec_text(
    repo: &impl SpecRepository,
) -> Result<SpecTextNormalization, AppError> {
    if repo
        .get_setting(SPEC_TEXT_NORMALIZED_SETTING)
        .await?
        .is_some()
    {
        return Ok(SpecTextNormalization {
            already_done: true,
            ..Default::default()
        });
    }
    let mut summary = SpecTextNormalization::default();
    for (_, meta, _) in repo.list_all_spec_versions().await? {
        let Some(content) = repo.get_spec_content(meta.id).await? else {
            continue;
        };
        let normalized = normalize_spec_text(&content);
        let hash = content_hash(&normalized);
        let endpoint_texts: Vec<(i64, String)> = repo
            .get_endpoints_for_version(meta.id)
            .await?
            .into_iter()
            .filter_map(|e| {
                let id = e.id?;
                let text = normalize_spec_text(&e.yaml_content);
                (text != e.yaml_content).then(|| (id, text.into_owned()))
            })
            .collect();
        if normalized == content && hash == meta.content_hash && endpoint_texts.is_empty() {
            continue;
        }
        repo.rewrite_spec_text(RewriteSpecText {
            spec_version_id: meta.id,
            content: &normalized,
            content_hash: &hash,
            endpoint_texts: &endpoint_texts,
        })
        .await?;
        summary.versions += 1;
        summary.endpoints += endpoint_texts.len();
    }
    for mut contract in repo.list_channel_message_contracts().await? {
        let payload = normalize_spec_text(&contract.payload_yaml);
        if payload != contract.payload_yaml {
            contract.payload_yaml = payload.into_owned();
            repo.upsert_channel_message_contract(&contract).await?;
            summary.contracts += 1;
        }
    }
    repo.set_setting(SPEC_TEXT_NORMALIZED_SETTING, &super::now_iso())
        .await?;
    tracing::info!(
        "Normalized stored spec text (line endings, BOM): {} version(s), {} snippet(s), {} contract(s) rewritten",
        summary.versions,
        summary.endpoints,
        summary.contracts
    );
    Ok(summary)
}

/// Resolve the declared stream (ADR-0004/0005). The graph is informational
/// and never fails a build: `trunk` with `tag`, or a tag naming no existing
/// sanshain-branch, records the build as if no stream had been declared (dev
/// only) and carries a warning instead of an error. Branches are still never
/// auto-created, so a typo'd pipeline cannot mint a half-empty release graph.
/// Applies to dry-runs too: a misconfigured pipeline must hear it.
pub(crate) async fn resolve_stream(
    repo: &impl SpecRepository,
    trunk: bool,
    tag: Option<&str>,
) -> Result<ResolvedStream, AppError> {
    let dev_only = |warning: String| ResolvedStream {
        trunk: false,
        branch: None,
        warnings: vec![warning],
    };
    match tag {
        Some(_) if trunk => Ok(dev_only(
            "trunk and tag are mutually exclusive - this build was recorded in neither".to_string(),
        )),
        Some(name) => match repo.find_branch(name).await? {
            Some(branch) => Ok(ResolvedStream {
                trunk: false,
                branch: Some(branch),
                warnings: Vec::new(),
            }),
            None => Ok(dev_only(format!(
                "no sanshain-branch '{name}' - this build was not recorded in a release graph"
            ))),
        },
        None => Ok(ResolvedStream {
            trunk,
            branch: None,
            warnings: Vec::new(),
        }),
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

/// An endpoint of a list with the one line the Producer wrote about it.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ListedEndpoint {
    #[serde(flatten)]
    pub endpoint: EndpointRecord,
    /// Derived from the snippet: the operation's `summary` or the first
    /// sentence of its `description` (message summary, rpc comment).
    pub summary: Option<String>,
}

/// The endpoints of one version-line entry with their one-line descriptions.
pub async fn list_producer_endpoints_described(
    repo: &impl SpecRepository,
    producername: &str,
    api_type: ApiType,
    version: SemVer,
) -> Result<Vec<ListedEndpoint>, AppError> {
    Ok(
        list_producer_endpoints(repo, producername, api_type, version)
            .await?
            .into_iter()
            .map(|endpoint| ListedEndpoint {
                summary: crate::descriptions::endpoint_summary(
                    endpoint.api_type,
                    &endpoint.yaml_content,
                ),
                endpoint,
            })
            .collect(),
    )
}

/// The full provided document of one version-line entry — what the Producer
/// submitted, with line endings and a leading BOM normalized on provide.
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

/// What to diff: two versions of one line, either whole stored documents or
/// one endpoint's snippet (`endpoint` = path and method).
#[derive(Debug, Clone)]
pub struct DiffRequest<'a> {
    pub api_type: ApiType,
    pub from: SemVer,
    pub to: SemVer,
    pub endpoint: Option<(&'a str, &'a str)>,
    pub options: DiffOptions,
}

/// The line diff between two versions, oldest side first.
#[derive(Debug, serde::Serialize)]
pub struct VersionDiff {
    pub from: SemVer,
    pub to: SemVer,
    pub hunks: Vec<DiffHunk>,
}

/// Diff two versions of a line with the service's `similar` diff. Snapshots
/// participate as a single entry ("what's coming" vs the last GA). In
/// endpoint mode a version without the endpoint diffs as an empty document,
/// which shows the removal or introduction.
#[instrument(skip_all)]
pub async fn diff_versions(
    repo: &impl SpecRepository,
    producername: &str,
    request: &DiffRequest<'_>,
) -> Result<VersionDiff, AppError> {
    let from_content = diff_side(repo, producername, request, request.from).await?;
    let to_content = diff_side(repo, producername, request, request.to).await?;
    let options = request.options;
    let hunks =
        super::run_cpu_bound(move || Ok(diff_lines(&from_content, &to_content, options))).await?;
    Ok(VersionDiff {
        from: request.from,
        to: request.to,
        hunks,
    })
}

async fn diff_side(
    repo: &impl SpecRepository,
    producername: &str,
    request: &DiffRequest<'_>,
    version: SemVer,
) -> Result<String, AppError> {
    let (_sid, entry) = resolve_pin(repo, producername, request.api_type, version).await?;
    version_text(repo, entry.id, request.api_type, request.endpoint).await
}

/// The text a version contributes to a diff or blame: its stored document,
/// or one endpoint's snippet (empty when the version lacks the endpoint).
async fn version_text(
    repo: &impl SpecRepository,
    entry_id: i64,
    api_type: ApiType,
    endpoint: Option<(&str, &str)>,
) -> Result<String, AppError> {
    match endpoint {
        None => Ok(repo.get_spec_content(entry_id).await?.unwrap_or_default()),
        Some((path, method)) => {
            let method = method_for(api_type, method);
            let normalized = openapi::lookup_path(api_type, path);
            Ok(repo
                .get_endpoints_for_version(entry_id)
                .await?
                .into_iter()
                .find(|e| e.normalized_path == normalized && e.method == method)
                .map(|e| e.yaml_content)
                .unwrap_or_default())
        }
    }
}

/// Consecutive lines of a version last changed in the same version, with
/// that version's attribution shown once for the block.
#[derive(Debug, serde::Serialize)]
pub struct BlameBlock {
    pub version: SemVer,
    pub stability: Stability,
    pub provided_by: String,
    pub updated_at: String,
    /// 1-based number of the block's first line.
    pub start_line: usize,
    pub lines: Vec<String>,
}

/// Blame for one version of a line: its text split into blocks of lines
/// last changed in the same version. The line is walked oldest-first up to
/// `version` with the same `similar` diff as the diff views; in endpoint mode
/// a version without the endpoint is an empty text, so a returning endpoint
/// is blamed on the version that brought it back.
#[instrument(skip_all)]
pub async fn blame_version(
    repo: &impl SpecRepository,
    producername: &str,
    api_type: ApiType,
    version: SemVer,
    endpoint: Option<(&str, &str)>,
) -> Result<Vec<BlameBlock>, AppError> {
    let (service_id, _target) = resolve_pin(repo, producername, api_type, version).await?;
    let mut line: Vec<SpecVersionMeta> = repo
        .list_spec_versions(service_id)
        .await?
        .into_iter()
        .filter(|v| v.api_type == api_type && v.version <= version)
        .collect();
    line.sort_by_key(|v| v.version);
    let mut texts = Vec::with_capacity(line.len());
    for meta in &line {
        texts.push(version_text(repo, meta.id, api_type, endpoint).await?);
    }

    let (owners, texts) = super::run_cpu_bound(move || {
        let refs: Vec<&str> = texts.iter().map(String::as_str).collect();
        let owners = blame_lines(&refs);
        Ok((owners, texts))
    })
    .await?;
    let last: Vec<&str> = texts
        .last()
        .map(|t| t.lines().collect())
        .unwrap_or_default();
    Ok(group_blame(&owners)
        .into_iter()
        .filter_map(|run| {
            let meta = line.get(run.owner)?;
            Some(BlameBlock {
                version: meta.version,
                stability: meta.stability,
                provided_by: meta.provided_by.clone(),
                updated_at: meta.updated_at.clone(),
                start_line: run.start + 1,
                lines: last
                    .iter()
                    .skip(run.start)
                    .take(run.len)
                    .map(|l| l.to_string())
                    .collect(),
            })
        })
        .collect())
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

/// Cap for concurrent `/validate` parses. The endpoint is deliberately
/// unauthenticated (a public validator), so this cap — not authentication —
/// is what bounds how much parsing CPU anonymous callers can occupy at once.
/// Excess callers wait their turn instead of being refused.
static VALIDATE_PERMITS: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(4);

/// [`validate_spec`], off the async worker thread and capped in concurrency.
pub async fn validate_spec_async(
    api_type: ApiType,
    content: String,
) -> Result<ValidationReport, AppError> {
    let _permit = VALIDATE_PERMITS
        .acquire()
        .await
        .map_err(|e| AppError::Internal(format!("Validator unavailable: {}", e)))?;
    super::run_cpu_bound(move || Ok(validate_spec(api_type, &content))).await
}

/// Validate a pasted document against the exact provide-side pipeline:
/// version extraction (strict semver) and splitting. No producer context, no
/// version-line rules — those need a Provide.
pub fn validate_spec(api_type: ApiType, content: &str) -> ValidationReport {
    let content = normalize_spec_text(content);
    let content: &str = &content;
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
    use super::super::version_rules::{
        classify_change, diff_endpoints, ga_baseline, propose_free, propose_free_minor,
    };
    use crate::application::mock_repo::MockRepo;

    // The blocking-pool wrapper must give the same verdict as the sync
    // validator, for both a valid and an invalid document.
    #[tokio::test]
    async fn validate_spec_async_matches_sync_verdicts() {
        let valid = "openapi: 3.0.3\ninfo:\n  title: T\n  version: 1.0.0\npaths:\n  /a:\n    get:\n      responses:\n        '200':\n          description: OK\n";
        let report = validate_spec_async(ApiType::OpenApi, valid.to_string())
            .await
            .unwrap();
        assert!(report.valid);
        assert_eq!(report.endpoint_count, Some(1));

        let report = validate_spec_async(ApiType::OpenApi, "not: yaml: spec".to_string())
            .await
            .unwrap();
        assert!(!report.valid);
        assert!(report.error.is_some());
    }

    #[test]
    fn normalize_spec_text_converts_crlf_and_lone_cr_to_lf() {
        assert_eq!(normalize_spec_text("a\r\nb\rc\n"), "a\nb\nc\n");
        // A CR immediately before a CRLF is a lone CR followed by a line end.
        assert_eq!(normalize_spec_text("a\r\r\nb"), "a\n\nb");
    }

    #[test]
    fn normalize_spec_text_strips_only_a_leading_bom() {
        assert_eq!(normalize_spec_text("\u{feff}x: 1\r\n"), "x: 1\n");
        assert_eq!(normalize_spec_text("\u{feff}x: 1\n"), "x: 1\n");
        // A BOM anywhere else is content, not an encoding marker.
        assert_eq!(normalize_spec_text("x: \u{feff}\n"), "x: \u{feff}\n");
    }

    #[test]
    fn normalize_spec_text_borrows_when_nothing_changes() {
        let lf = "openapi: 3.0.3\ninfo: {}\n";
        assert!(matches!(normalize_spec_text(lf), Cow::Borrowed(s) if s == lf));
        assert!(matches!(
            normalize_spec_text("\u{feff}a\n"),
            Cow::Borrowed("a\n")
        ));
        assert!(matches!(normalize_spec_text("a\r\n"), Cow::Owned(_)));
    }

    #[test]
    fn normalized_documents_hash_the_same_from_every_platform() {
        let lf = "openapi: 3.0.3\ninfo:\n  version: 1.0.0\n";
        let windows = format!("\u{feff}{}", lf.replace('\n', "\r\n"));
        let classic_mac = lf.replace('\n', "\r");
        assert_eq!(
            content_hash(&normalize_spec_text(&windows)),
            content_hash(lf)
        );
        assert_eq!(
            content_hash(&normalize_spec_text(&classic_mac)),
            content_hash(lf)
        );
    }

    #[test]
    fn validate_spec_accepts_bom_and_crlf_documents() {
        let lf = "openapi: 3.0.3\ninfo:\n  title: T\n  version: 1.0.0\npaths:\n  /a:\n    get:\n      responses:\n        '200':\n          description: OK\n";
        let windows = format!("\u{feff}{}", lf.replace('\n', "\r\n"));
        let report = validate_spec(ApiType::OpenApi, &windows);
        assert!(report.valid, "got: {:?}", report.error);
        assert_eq!(report.version.map(|v| v.to_string()), Some("1.0.0".into()));
        assert_eq!(report.endpoints, vec!["GET /a".to_string()]);
    }

    async fn seed(repo: &MockRepo, service: &str, content: &str, snippet: &str) -> i64 {
        let sid = repo.ensure_service(service).await.unwrap();
        repo.upsert_spec_version(crate::domain::ports::UpsertSpecVersion {
            outbox: Vec::new(),
            service_id: sid,
            api_type: ApiType::OpenApi,
            version: "1.0.0".parse().unwrap(),
            stability: Stability::Ga,
            content,
            content_hash: "sha256:raw",
            provided_by: "alice",
            expected_prior_hash: None,
            now_iso: "2026-01-01T00:00:00Z",
            endpoints: vec![EndpointRecord {
                id: None,
                api_type: ApiType::OpenApi,
                path: "/a".into(),
                normalized_path: "/a".into(),
                method: "GET".into(),
                yaml_content: snippet.into(),
                deprecated: false,
            }],
        })
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn normalize_stored_spec_text_rewrites_once_and_records_it() {
        let repo = MockRepo::new();
        let crlf_id = seed(&repo, "crlf", "\u{feff}a: 1\r\nb: 2\r\n", "x\r\n").await;
        let lf_id = seed(&repo, "lf", "a: 1\n", "x\n").await;
        let lf_hash_before = content_hash("a: 1\n");
        // The LF entry's hash is already right except for the seed's dummy.
        repo.rewrite_spec_text(RewriteSpecText {
            spec_version_id: lf_id,
            content: "a: 1\n",
            content_hash: &lf_hash_before,
            endpoint_texts: &[],
        })
        .await
        .unwrap();
        repo.upsert_channel_message_contract(&ChannelMessageContract {
            channel: "orders".into(),
            message_name: "Created".into(),
            owner_service_id: 1,
            payload_yaml: "type: object\r\n".into(),
        })
        .await
        .unwrap();

        let summary = normalize_stored_spec_text(&repo).await.unwrap();
        assert_eq!(
            summary,
            SpecTextNormalization {
                already_done: false,
                versions: 1,
                endpoints: 1,
                contracts: 1,
            }
        );
        assert_eq!(
            repo.get_spec_content(crlf_id).await.unwrap().as_deref(),
            Some("a: 1\nb: 2\n")
        );
        let line = repo.list_spec_versions(1).await.unwrap();
        assert_eq!(line[0].content_hash, content_hash("a: 1\nb: 2\n"));
        assert_eq!(line[0].provided_by, "alice");
        assert_eq!(
            repo.get_endpoints_for_version(crlf_id).await.unwrap()[0].yaml_content,
            "x\n"
        );
        assert_eq!(
            repo.list_channel_message_contracts().await.unwrap()[0].payload_yaml,
            "type: object\n"
        );
        assert!(
            repo.get_setting(SPEC_TEXT_NORMALIZED_SETTING)
                .await
                .unwrap()
                .is_some()
        );

        // Recorded: the next start does not look again, even at new CRs.
        seed(&repo, "late", "c: 3\r\n", "y\r\n").await;
        let again = normalize_stored_spec_text(&repo).await.unwrap();
        assert!(again.already_done);
        assert_eq!(again.versions, 0);
    }

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
    fn propose_free_minor_skips_taken_minors() {
        let line = vec![
            meta("1.3.0", Stability::Snapshot),
            meta("1.4.0", Stability::Ga),
            meta("1.5.0", Stability::Snapshot),
        ];
        assert_eq!(
            propose_free_minor(&line, "1.3.0".parse().unwrap()).to_string(),
            "1.6.0"
        );
        assert_eq!(
            propose_free_minor(&line, "2.0.0".parse().unwrap()).to_string(),
            "2.1.0"
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

    #[tokio::test]
    async fn resolve_stream_honours_trunk_and_an_existing_tag() {
        let repo = MockRepo::new();
        let id = repo
            .insert_branch(
                "R",
                "2026-01-01T00:00:00Z",
                "rel",
                "trunk",
                "2026-01-01T00:00:00Z",
                Vec::new(),
            )
            .await
            .unwrap();

        let plain = resolve_stream(&repo, false, None).await.unwrap();
        assert!(!plain.trunk && plain.branch.is_none() && plain.warnings.is_empty());
        assert_eq!(plain.label(), None);

        let trunk = resolve_stream(&repo, true, None).await.unwrap();
        assert!(trunk.trunk && trunk.branch.is_none() && trunk.warnings.is_empty());
        assert_eq!(trunk.label(), Some("trunk"));

        let tagged = resolve_stream(&repo, false, Some("R")).await.unwrap();
        assert!(!tagged.trunk && tagged.warnings.is_empty());
        assert_eq!(tagged.branch.as_ref().map(|b| b.id), Some(id));
        assert_eq!(tagged.label(), Some("R"));
    }

    #[tokio::test]
    async fn resolve_stream_records_dev_only_and_warns_instead_of_failing() {
        let repo = MockRepo::new();
        repo.insert_branch(
            "R",
            "2026-01-01T00:00:00Z",
            "rel",
            "trunk",
            "2026-01-01T00:00:00Z",
            Vec::new(),
        )
        .await
        .unwrap();

        let unknown = resolve_stream(&repo, false, Some("Ghost")).await.unwrap();
        assert!(!unknown.trunk && unknown.branch.is_none());
        assert_eq!(unknown.label(), None);
        assert_eq!(
            unknown.warnings,
            vec![
                "no sanshain-branch 'Ghost' - this build was not recorded in a release graph"
                    .to_string()
            ]
        );
        // Never auto-created.
        assert!(repo.find_branch("Ghost").await.unwrap().is_none());

        // Both declared: neither recorded — even when the tag exists.
        for tag in ["R", "Ghost"] {
            let both = resolve_stream(&repo, true, Some(tag)).await.unwrap();
            assert!(!both.trunk && both.branch.is_none(), "{tag}");
            assert_eq!(
                both.warnings,
                vec![
                    "trunk and tag are mutually exclusive - this build was recorded in neither"
                        .to_string()
                ],
                "{tag}"
            );
        }
    }
}
