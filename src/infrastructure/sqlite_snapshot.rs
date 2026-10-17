//! A consistent copy of the SQLite database, produced by the engine itself
//! (`VACUUM INTO`) rather than by copying the live file, which can be
//! inconsistent while the write-ahead log is open (#50).
//!
//! The copy is written to a temporary file next to the database and removed
//! when the [`SqliteSnapshot`] is dropped — after streaming, or as soon as the
//! response is abandoned. Only one snapshot runs at a time; the
//! [`SnapshotLease`] carries that right and gives it back on drop.

use crate::domain::ports::RepositoryError;
use sqlx::SqlitePool;
use sqlx::sqlite::SqliteConnectOptions;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio_stream::StreamExt;
use tokio_util::io::ReaderStream;

/// The exclusive right to produce a snapshot. Released on drop, so holding
/// it inside the response stream keeps a second snapshot out until the
/// first download has ended one way or the other.
pub struct SnapshotLease {
    flag: Arc<AtomicBool>,
}

impl SnapshotLease {
    /// `None` when a snapshot is already in progress.
    pub fn acquire(flag: &Arc<AtomicBool>) -> Option<Self> {
        flag.compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .ok()
            .map(|_| Self {
                flag: Arc::clone(flag),
            })
    }
}

impl Drop for SnapshotLease {
    fn drop(&mut self) {
        self.flag.store(false, Ordering::Release);
    }
}

/// The directory the database file lives in, read from the connection URL.
///
/// An in-memory database has no directory, so the system temporary directory
/// stands in; a bare relative file name means the working directory.
pub fn data_dir(db_url: &str) -> PathBuf {
    // Checked on the URL: sqlx gives an in-memory database a synthetic file
    // name, so the parsed options cannot tell.
    if db_url.contains(":memory:") || db_url.contains("mode=memory") {
        return std::env::temp_dir();
    }
    let filename = SqliteConnectOptions::from_str(db_url)
        .ok()
        .map(|options| options.get_filename().to_path_buf());
    match filename.as_deref().and_then(Path::parent) {
        Some(parent) if !parent.as_os_str().is_empty() => parent.to_path_buf(),
        _ => PathBuf::from("."),
    }
}

/// A file system path as the path part of a SQLite `file:` URI: the three
/// characters that would be read as URI syntax are percent-encoded.
fn uri_path(path: &str) -> String {
    path.replace('%', "%25")
        .replace('?', "%3F")
        .replace('#', "%23")
}

/// A finished snapshot on disk. Removes its file on drop.
pub struct SqliteSnapshot {
    path: PathBuf,
    size: u64,
    _lease: SnapshotLease,
}

impl SqliteSnapshot {
    /// Run `VACUUM INTO` a fresh temporary file in `dir`.
    ///
    /// The file name is unique per attempt: `VACUUM INTO` refuses an existing
    /// target, so a leftover from an interrupted run can never block the next.
    #[tracing::instrument(skip_all)]
    pub async fn create(
        pool: &SqlitePool,
        dir: &Path,
        lease: SnapshotLease,
    ) -> Result<Self, RepositoryError> {
        let path = dir.join(format!("sanshain-snapshot-{}.db.tmp", uuid::Uuid::new_v4()));
        // As a URI: `VACUUM INTO` attaches its target with the connection's own
        // open flags, and for an in-memory source that includes
        // `SQLITE_OPEN_MEMORY` — the copy would never reach disk. `mode=rwc`
        // in the URI replaces the mode flags for the target alone.
        let target = format!(
            "file:{}?mode=rwc",
            uri_path(path.to_str().ok_or_else(|| RepositoryError::Internal(
                "snapshot path is not UTF-8".to_string()
            ))?)
        );
        // Owned before the VACUUM runs, so a failure part-way still removes
        // whatever was written.
        let mut snapshot = Self {
            path,
            size: 0,
            _lease: lease,
        };
        sqlx::query("VACUUM INTO ?")
            .bind(target)
            .execute(pool)
            .await
            .map_err(|e| RepositoryError::Internal(format!("VACUUM INTO failed: {e}")))?;
        snapshot.size = tokio::fs::metadata(&snapshot.path)
            .await
            .map_err(|e| RepositoryError::Internal(format!("snapshot file: {e}")))?
            .len();
        Ok(snapshot)
    }

    pub fn size(&self) -> u64 {
        self.size
    }

    /// The file's bytes as a stream that owns the snapshot: the temporary file
    /// and the lease go when the stream does, whether it was read to the end
    /// or dropped with the connection.
    pub async fn into_stream(
        self,
    ) -> Result<
        impl tokio_stream::Stream<Item = std::io::Result<tokio_util::bytes::Bytes>>,
        RepositoryError,
    > {
        let file = tokio::fs::File::open(&self.path)
            .await
            .map_err(|e| RepositoryError::Internal(format!("open snapshot: {e}")))?;
        let guard = self;
        Ok(ReaderStream::new(file).map(move |chunk| {
            let _held = &guard;
            chunk
        }))
    }
}

impl Drop for SqliteSnapshot {
    fn drop(&mut self) {
        if let Err(e) = std::fs::remove_file(&self.path)
            && e.kind() != std::io::ErrorKind::NotFound
        {
            tracing::warn!(path = %self.path.display(), "could not remove snapshot file: {e}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn data_dir_is_the_database_files_directory() {
        assert_eq!(
            data_dir("sqlite:/data/sanshain.db?mode=rwc"),
            PathBuf::from("/data")
        );
        assert_eq!(data_dir("sqlite:sanshain.db?mode=rwc"), PathBuf::from("."));
        assert_eq!(data_dir("sqlite::memory:"), std::env::temp_dir());
        assert_eq!(
            data_dir("sqlite:file:x?mode=memory&cache=shared"),
            std::env::temp_dir()
        );
    }

    #[test]
    fn uri_path_escapes_what_a_uri_would_misread() {
        assert_eq!(uri_path("/data/a b"), "/data/a b");
        assert_eq!(uri_path("/d%?#"), "/d%25%3F%23");
    }

    #[test]
    fn the_lease_is_exclusive_and_returned_on_drop() {
        let flag = Arc::new(AtomicBool::new(false));
        let first = SnapshotLease::acquire(&flag).expect("free");
        assert!(SnapshotLease::acquire(&flag).is_none());
        drop(first);
        assert!(SnapshotLease::acquire(&flag).is_some());
    }

    #[tokio::test]
    async fn a_dropped_snapshot_removes_its_file() {
        // One connection: every pooled connection to `:memory:` would be a
        // database of its own, and the VACUUM must see the seeded table.
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("pool");
        sqlx::query("CREATE TABLE t (x INTEGER); INSERT INTO t VALUES (1)")
            .execute(&pool)
            .await
            .expect("seed");
        let flag = Arc::new(AtomicBool::new(false));
        let lease = SnapshotLease::acquire(&flag).expect("free");
        let snapshot = SqliteSnapshot::create(&pool, &std::env::temp_dir(), lease)
            .await
            .expect("snapshot");
        let path = snapshot.path.clone();
        assert!(snapshot.size() > 0);
        assert!(path.is_file());
        drop(snapshot);
        assert!(!path.exists());
        assert!(!flag.load(Ordering::Acquire), "the lease is returned");
    }
}
