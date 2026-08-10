//! The version rules of ADR-0003, as pure functions.
//!
//! Split out of `spec_service` because they are the one part of the provide
//! path that needs no repository and no I/O: given a version line and a
//! candidate, they decide whether the candidate is honest and what to propose
//! instead. Keeping them apart makes that decidable without a database, and
//! leaves the provide flow readable as a flow.

use crate::asyncapi;
use crate::domain::models::*;
use crate::openapi;
use std::collections::{HashMap, HashSet};

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
pub(crate) fn propose_free(line: &[SpecVersionMeta], candidate: SemVer) -> SemVer {
    let taken: HashSet<SemVer> = line.iter().map(|v| v.version).collect();
    let mut proposal = candidate;
    while taken.contains(&proposal) {
        proposal = proposal.increment(Impact::Patch);
    }
    proposal
}

/// Classify the change between two documents for the bump proposal:
/// breaking → major, additive → minor, shape-identical → patch.
pub(crate) fn classify_change(
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
pub(crate) fn diff_endpoints(
    old: &[EndpointRecord],
    new: &[openapi::EndpointSpec],
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
    ProvideChanges {
        inserts,
        updates,
        deletes,
    }
}

/// The GA entry the mislabel check compares against: the highest GA of the
/// line strictly below the incoming version.
pub(crate) fn ga_baseline(line: &[SpecVersionMeta], below: SemVer) -> Option<SpecVersionMeta> {
    line.iter()
        .filter(|v| v.stability == Stability::Ga && v.version < below)
        .max_by_key(|v| v.version)
        .cloned()
}
