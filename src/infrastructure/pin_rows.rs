//! The row shape every pin-set query selects, and its one mapping into the
//! domain's [`TrunkPinInfo`] — shared by the SQLite and Postgres repositories.

use crate::domain::models::{SemVer, TrunkPinInfo};
use crate::domain::ports::RepositoryError;

/// `(client, service, api_type, major, minor, patch, path, method,
/// valid_from, last_required_at)` — generic over the integer width because
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
        method,
        valid_from,
        last_required_at,
        dangling: false,
    })
}
