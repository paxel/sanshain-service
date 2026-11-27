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
use std::collections::HashMap;

/// Whole-document backward-compatibility verdict for one API type.
pub fn check_compatibility(api_type: ApiType, old: &str, new: &str) -> Result<(), String> {
    match api_type {
        ApiType::OpenApi => openapi::check_backward_compatibility(old, new),
        ApiType::AsyncApi => asyncapi::check_backward_compatibility(old, new),
        ApiType::Proto => crate::proto::check_backward_compatibility(old, new),
    }
}

/// The number a refusal proposes instead of `version` (san-1020): the highest
/// version of `version`'s major in `line` (or `version` itself when higher),
/// bumped by one minor; a breaking change goes to the highest major of the
/// whole line plus one. Never a patch: an assistant publishes the proposal as
/// it is, and a patch rarely says what changed. Landing above everything in
/// the major keeps the proposal out of gaps left by older lines of work, and
/// staying in the major keeps a maintenance line from being sent to a major
/// whose GA it would break.
pub(crate) fn propose_version(line: &[SpecVersionMeta], version: SemVer, impact: Impact) -> SemVer {
    if impact == Impact::Major {
        let highest_major = line
            .iter()
            .map(|v| v.version.major)
            .fold(version.major, u32::max);
        return SemVer::new(highest_major.saturating_add(1), 0, 0);
    }
    let highest_in_major = line
        .iter()
        .map(|v| v.version)
        .filter(|v| v.major == version.major)
        .fold(version, SemVer::max);
    highest_in_major.increment(Impact::Minor)
}

/// The number a refusal proposes on a maintenance branch (san-1030): a side
/// line of semver. The proposal stays in the refused `MAJOR.MINOR`, one patch
/// above its highest version (`2.6.0` refused gives `2.6.1`, not `2.23.0`),
/// for an additive change as for one that keeps the shape. A breaking change
/// still gets a new major: Sanshain never proposes a number it would refuse
/// with `breaking_without_major`.
pub(crate) fn propose_maintenance_version(
    line: &[SpecVersionMeta],
    version: SemVer,
    impact: Impact,
) -> SemVer {
    if impact == Impact::Major {
        return propose_version(line, version, Impact::Major);
    }
    line.iter()
        .map(|v| v.version)
        .filter(|v| v.major == version.major && v.minor == version.minor)
        .fold(version, SemVer::max)
        .increment(Impact::Patch)
}

/// The GA the level of a proposal is classified against: the highest GA of
/// `version`'s major. The proposal lands above it, so a change that is
/// additive against the conflicting entry but drops what that GA added is
/// classified as breaking, not sent into `breaking_without_major`.
pub(crate) fn highest_ga_in_major(
    line: &[SpecVersionMeta],
    version: SemVer,
) -> Option<SpecVersionMeta> {
    line.iter()
        .filter(|v| v.stability == Stability::Ga && v.version.major == version.major)
        .max_by_key(|v| v.version)
        .cloned()
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
    let mut inserts = 0usize;
    let mut updates = 0usize;
    for endpoint in new {
        match old_map.remove(&(endpoint.normalized_path.clone(), endpoint.method.clone())) {
            Some(existing) => {
                if existing.yaml_content != endpoint.yaml_content
                    || existing.path != endpoint.path
                    || existing.deprecated != endpoint.deprecated
                {
                    updates = updates.saturating_add(1);
                }
            }
            None => inserts = inserts.saturating_add(1),
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
