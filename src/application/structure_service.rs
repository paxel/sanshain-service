//! The structure view of a version: its endpoints as trees of parameters,
//! bodies, responses and messages, each node carrying the version of the line
//! that last changed it (see `crate::structure`).

use super::require_service::method_for;
use super::spec_service::version_line;
use crate::domain::models::{ApiType, AppError, EndpointRecord, SemVer};
use crate::domain::ports::SpecRepository;
use crate::openapi;
use crate::structure::{self, Node};
use moka::future::Cache;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::{Arc, LazyLock};
use tracing::instrument;

/// Built trees by the hash of the snippet they were built from: a version's
/// stored snippets never change in place, and a re-provided snapshot stores
/// new ones, so the key alone says whether a tree is still right. Only the
/// bare trees are kept; the version annotations depend on the whole line and
/// are computed per request. Weighed in nodes, since one inlined tree can be
/// a thousand times another.
static TREES: LazyLock<Cache<String, Arc<Node>>> = LazyLock::new(|| {
    Cache::builder()
        .max_capacity(TREE_NODE_BUDGET)
        .weigher(|_key: &String, tree: &Arc<Node>| {
            u32::try_from(structure::node_count(tree)).unwrap_or(u32::MAX)
        })
        .build()
});

/// How many nodes the cached trees may hold together.
const TREE_NODE_BUDGET: u64 = 400_000;

#[derive(Debug, serde::Serialize)]
pub struct StructureView {
    pub service: String,
    pub api_type: ApiType,
    pub version: SemVer,
    /// One endpoint in endpoint mode; every endpoint, without children, for
    /// the whole document.
    pub endpoints: Vec<Node>,
}

#[derive(Debug, Clone, Copy)]
pub struct StructureRequest<'a> {
    pub api_type: ApiType,
    pub version: SemVer,
    /// One endpoint (`path`, `method`), or the whole document.
    pub endpoint: Option<(&'a str, &'a str)>,
    /// Give every body an example (built from the schema when the spec has
    /// none). Endpoint mode only; the whole document answers no bodies.
    pub with_examples: bool,
}

/// The structure of `version`, annotated against the line's earlier versions.
#[instrument(skip_all, fields(producer = producername))]
pub async fn structure_version(
    repo: &impl SpecRepository,
    producername: &str,
    request: StructureRequest<'_>,
) -> Result<StructureView, AppError> {
    let api_type = request.api_type;
    if api_type == ApiType::Proto {
        return Err(AppError::BadRequest(
            "the structure view covers OpenAPI and AsyncAPI; a proto reads as text".to_string(),
        ));
    }
    let line = version_line(repo, producername, api_type, request.version).await?;
    let target = request.endpoint.map(|(path, method)| {
        (
            openapi::lookup_path(api_type, path),
            method_for(api_type, method),
        )
    });

    let mut history: Vec<(SemVer, Vec<EndpointRecord>)> = Vec::with_capacity(line.len());
    for meta in &line {
        let mut rows: Vec<EndpointRecord> = repo
            .get_endpoints_for_version(meta.id)
            .await?
            .into_iter()
            .filter(|row| match &target {
                Some((path, method)) => &row.normalized_path == path && &row.method == method,
                None => true,
            })
            .collect();
        rows.sort_by(|a, b| (&a.path, &a.method).cmp(&(&b.path, &b.method)));
        history.push((meta.version, rows));
    }
    if let (Some((path, method)), Some((_, rows))) = (request.endpoint, history.last())
        && rows.is_empty()
    {
        return Err(AppError::NotFound(format!(
            "{} version {} of '{}' has no endpoint {} {}",
            api_type.as_str(),
            request.version,
            producername,
            method.to_uppercase(),
            path
        )));
    }

    let trees = trees_for(api_type, &history).await?;
    let whole_document = request.endpoint.is_none();
    let with_examples = request.with_examples && !whole_document;
    let endpoints = super::run_cpu_bound(move || {
        let annotated: Vec<(SemVer, Vec<Node>)> = history
            .into_iter()
            .map(|(version, rows)| {
                let nodes = rows
                    .iter()
                    .filter_map(|row| trees.get(&tree_key(api_type, row)))
                    .map(|tree| tree.as_ref().clone())
                    .collect();
                (version, nodes)
            })
            .collect();
        let mut endpoints = structure::annotate(annotated);
        if with_examples {
            endpoints.iter_mut().for_each(structure::fill_examples);
        }
        if whole_document {
            structure::headers_only(&mut endpoints);
        }
        Ok(endpoints)
    })
    .await?;

    Ok(StructureView {
        service: producername.to_string(),
        api_type,
        version: request.version,
        endpoints,
    })
}

/// The bare tree of every snippet in `history`, from the cache or built.
async fn trees_for(
    api_type: ApiType,
    history: &[(SemVer, Vec<EndpointRecord>)],
) -> Result<HashMap<String, Arc<Node>>, AppError> {
    let mut trees: HashMap<String, Arc<Node>> = HashMap::new();
    let mut missing: HashMap<String, EndpointRecord> = HashMap::new();
    for row in history.iter().flat_map(|(_, rows)| rows) {
        let key = tree_key(api_type, row);
        if trees.contains_key(&key) || missing.contains_key(&key) {
            continue;
        }
        match TREES.get(&key).await {
            Some(tree) => {
                trees.insert(key, tree);
            }
            None => {
                missing.insert(key, row.clone());
            }
        }
    }
    let built = super::run_cpu_bound(move || {
        Ok(missing
            .into_iter()
            .map(|(key, row)| {
                let tree = structure::endpoint_node(
                    api_type,
                    &row.method,
                    &row.path,
                    &row.normalized_path,
                    &row.yaml_content,
                )
                .unwrap_or_else(|reason| {
                    structure::unreadable_endpoint(
                        &row.method,
                        &row.path,
                        &row.normalized_path,
                        reason,
                    )
                });
                (key, Arc::new(tree))
            })
            .collect::<Vec<_>>())
    })
    .await?;
    for (key, tree) in built {
        TREES.insert(key.clone(), tree.clone()).await;
        trees.insert(key, tree);
    }
    Ok(trees)
}

fn tree_key(api_type: ApiType, row: &EndpointRecord) -> String {
    let mut hasher = Sha256::new();
    for part in [
        api_type.as_str(),
        &row.method,
        &row.path,
        &row.normalized_path,
        &row.yaml_content,
    ] {
        hasher.update(part.as_bytes());
        hasher.update([0u8]);
    }
    hex::encode(hasher.finalize())
}
