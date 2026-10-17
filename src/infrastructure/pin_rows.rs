//! The row shape every pin-set query selects, and its one mapping into the
//! domain's [`TrunkPinInfo`] — shared by the SQLite and Postgres repositories.

use crate::domain::models::{SemVer, TrunkPinInfo};
use crate::domain::ports::RepositoryError;

/// `(client, service, api_type, major, minor, patch, path, normalized_path,
/// method, valid_from, last_required_at)` — generic over the integer width because
/// SQLite decodes INTEGER as `i64` while Postgres decodes INT4 as `i32`.
pub(crate) type PinRow<I> = (
    String,
    String,
    String,
    I,
    I,
    I,
    String,
    String,
    String,
    String,
    String,
);

/// `dangling` starts `false`: whether a pin's version still exists is decided
/// in the application layer (`mark_dangling`), never by the row itself.
pub(crate) fn pin_row_to_info<I: Into<i64>>(
    row: PinRow<I>,
) -> Result<TrunkPinInfo, RepositoryError> {
    let (
        client,
        service,
        api_type,
        major,
        minor,
        patch,
        path,
        normalized_path,
        method,
        valid_from,
        last_required_at,
    ) = row;
    let (major, minor, patch): (i64, i64, i64) = (major.into(), minor.into(), patch.into());
    Ok(TrunkPinInfo {
        client,
        service,
        api_type: api_type.parse().map_err(RepositoryError::Internal)?,
        version: SemVer::new(major as u32, minor as u32, patch as u32),
        path,
        normalized_path,
        method,
        valid_from,
        last_required_at,
        dangling: false,
    })
}

/// How a periphery set (#46) is addressed in `peripheral_dependencies`: the
/// `track` column and the `branch_id` the open-row index compares as
/// `COALESCE(branch_id, 0)`.
pub(crate) fn periphery_track_key(
    track: crate::domain::models::PeripheryTrack,
) -> (&'static str, Option<i64>) {
    use crate::domain::models::PeripheryTrack;
    match track {
        PeripheryTrack::Dev => ("dev", None),
        PeripheryTrack::Trunk => ("trunk", None),
        PeripheryTrack::Branch(id) => ("branch", Some(id)),
    }
}

/// `(system_name, owner_kind, owner_name, description, source_location,
/// links_json)` — one `system_context` row (#48), shared by both engines.
pub(crate) type SystemContextRow = (
    String,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
    String,
);

/// A `system_context` row into the domain's [`SystemContext`] and its name.
pub(crate) fn system_context_from_row(
    row: SystemContextRow,
) -> Result<(String, crate::domain::models::SystemContext), RepositoryError> {
    use crate::domain::models::{SystemContext, SystemOwner};
    let (system, owner_kind, owner_name, description, source_location, links_json) = row;
    let links = serde_json::from_str(&links_json).map_err(|e| {
        RepositoryError::Internal(format!("links of system '{system}' are not JSON: {e}"))
    })?;
    let owner = match (owner_kind, owner_name) {
        (Some(kind), Some(name)) => Some(SystemOwner { kind, name }),
        _ => None,
    };
    Ok((
        system,
        SystemContext {
            owner,
            description,
            source_location,
            links,
        },
    ))
}

/// The `links_json` column value of a context.
pub(crate) fn links_json(
    context: &crate::domain::models::SystemContext,
) -> Result<String, RepositoryError> {
    serde_json::to_string(&context.links).map_err(|e| RepositoryError::Internal(e.to_string()))
}
