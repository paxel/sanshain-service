use sqlx::{Row, SqlitePool};
use std::collections::HashMap;
use std::str::FromStr;
use tracing::instrument;

/// Row shape for `spec_versions` metadata queries (id, service_id, api_type,
/// major, minor, patch, stability, content_hash, provided_by, created_at,
/// updated_at, last_required_at, trunk_provided_at) — everything but the
/// document.
type SpecVersionRow = (
    i64,
    i64,
    String,
    i64,
    i64,
    i64,
    String,
    String,
    String,
    String,
    String,
    Option<String>,
    Option<String>,
);

fn spec_version_from_row(row: SpecVersionRow) -> Result<SpecVersionMeta, RepositoryError> {
    let (
        id,
        service_id,
        api_type,
        major,
        minor,
        patch,
        stability,
        content_hash,
        provided_by,
        created_at,
        updated_at,
        last_required_at,
        trunk_provided_at,
    ) = row;
    Ok(SpecVersionMeta {
        id,
        service_id,
        api_type: api_type
            .parse()
            .map_err(|e: String| RepositoryError::Internal(e))?,
        version: SemVer::new(major as u32, minor as u32, patch as u32),
        stability: stability
            .parse()
            .map_err(|e: String| RepositoryError::Internal(e))?,
        content_hash,
        provided_by,
        created_at,
        updated_at,
        last_required_at,
        trunk_provided_at,
    })
}

/// Row shape for audit-log queries (id, timestamp, username, action, details,
/// service, version, action_type, diff, stream).
type AuditLogRow = (
    i64,
    String,
    String,
    String,
    String,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
);

/// Hashes a session token with SHA-256 so that only the hash is stored at rest.
/// The raw token is returned to the client; lookups hash the incoming token.
fn hash_session_token(token: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(token.as_bytes());
    hex::encode(hasher.finalize())
}

use super::pin_rows::{PinRow, pin_row_to_info};
use crate::domain::models::*;
use crate::domain::ports::{
    EndpointMap, NewAuditLog, RecordDependencyParams, RecordTrunkPinParams, RepositoryError,
    SpecRepository, UpsertSpecVersion,
};

struct ApiTokenRow {
    id: String,
    user_id: i64,
    name: String,
    token_hash: String,
    created_at: String,
    expires_at: String,
    last_used_at: Option<String>,
}

impl<'r> sqlx::FromRow<'r, sqlx::sqlite::SqliteRow> for ApiTokenRow {
    fn from_row(row: &'r sqlx::sqlite::SqliteRow) -> Result<Self, sqlx::Error> {
        Ok(Self {
            id: row.try_get("id")?,
            user_id: row.try_get("user_id")?,
            name: row.try_get("name")?,
            token_hash: row.try_get("token_hash")?,
            created_at: row.try_get("created_at")?,
            expires_at: row.try_get("expires_at")?,
            last_used_at: row.try_get("last_used_at")?,
        })
    }
}

impl From<ApiTokenRow> for ApiToken {
    fn from(row: ApiTokenRow) -> Self {
        Self {
            id: row.id,
            user_id: row.user_id,
            name: row.name,
            token_hash: row.token_hash,
            created_at: row.created_at,
            expires_at: row.expires_at,
            last_used_at: row.last_used_at,
        }
    }
}

#[derive(Clone)]
pub struct SqliteSpecRepository {
    pub pool: SqlitePool,
}

impl SqliteSpecRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn run_migrations(&self) -> Result<(), sqlx::migrate::MigrateError> {
        tracing::info!("Running SQLite migrations...");
        let migrator = sqlx::migrate::Migrator::new(std::path::Path::new(
            "src/infrastructure/migrations/sqlite",
        ))
        .await?;
        migrator.run(&self.pool).await?;
        Ok(())
    }
}

impl SpecRepository for SqliteSpecRepository {
    async fn ping(&self) -> Result<(), RepositoryError> {
        sqlx::query("SELECT 1")
            .execute(&self.pool)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(())
    }

    #[instrument(skip_all)]
    async fn upsert_spec_version(
        &self,
        params: UpsertSpecVersion<'_>,
    ) -> Result<i64, RepositoryError> {
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;

        // Insert-or-overwrite keeps the row's identity and `created_at` on an
        // overwrite/promotion; only the content, stability, attribution and
        // `updated_at` move.
        //
        // A compare-and-set caller never inserts: the row it read must still
        // exist and carry the expected hash, so it gets a plain conditional
        // UPDATE — the same statement shape as the Postgres twin (numbered
        // placeholders, one bind per value, so the two can be diffed
        // side by side). SQLite's single writer makes the race the Postgres
        // comment describes impossible here, but the two backends must enforce
        // the same contract.
        let upsert = if let Some(expected) = params.expected_prior_hash {
            sqlx::query(
                r#"
                UPDATE spec_versions SET
                    stability = ?6,
                    content = ?7,
                    content_hash = ?8,
                    provided_by = ?9,
                    updated_at = ?10
                 WHERE service_id = ?1 AND api_type = ?2 AND major = ?3 AND minor = ?4 AND patch = ?5
                   AND stability != 'ga'
                   AND content_hash = ?11
                "#,
            )
            .bind(params.service_id)
            .bind(params.api_type.as_str())
            .bind(params.version.major as i64)
            .bind(params.version.minor as i64)
            .bind(params.version.patch as i64)
            .bind(params.stability.as_str())
            .bind(params.content)
            .bind(params.content_hash)
            .bind(params.provided_by)
            .bind(params.now_iso)
            .bind(expected)
        } else {
            sqlx::query(
                r#"
                INSERT INTO spec_versions
                  (service_id, api_type, major, minor, patch, stability, content, content_hash, provided_by, created_at, updated_at)
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
                ON CONFLICT(service_id, api_type, major, minor, patch) DO UPDATE SET
                    stability = excluded.stability,
                    content = excluded.content,
                    content_hash = excluded.content_hash,
                    provided_by = excluded.provided_by,
                    updated_at = excluded.updated_at
                WHERE spec_versions.stability != 'ga'
                "#,
            )
            .bind(params.service_id)
            .bind(params.api_type.as_str())
            .bind(params.version.major as i64)
            .bind(params.version.minor as i64)
            .bind(params.version.patch as i64)
            .bind(params.stability.as_str())
            .bind(params.content)
            .bind(params.content_hash)
            .bind(params.provided_by)
            .bind(params.now_iso)
            .bind(params.now_iso)
        }
        .execute(&mut *tx)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        // Zero rows written means the guard refused — the last line of defense
        // against racing writers: for a CAS caller the row vanished, was
        // released, or no longer carries the expected hash; for a plain provide
        // a GA row is immutable even against a racing writer.
        if upsert.rows_affected() == 0 {
            return Err(RepositoryError::Conflict);
        }

        let (id,): (i64,) = sqlx::query_as(
            "SELECT id FROM spec_versions WHERE service_id = ? AND api_type = ? AND major = ? AND minor = ? AND patch = ?",
        )
        .bind(params.service_id)
        .bind(params.api_type.as_str())
        .bind(params.version.major as i64)
        .bind(params.version.minor as i64)
        .bind(params.version.patch as i64)
        .fetch_one(&mut *tx)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;

        // A version's endpoint set is complete by definition — replace it
        // wholesale rather than diffing in SQL.
        sqlx::query("DELETE FROM endpoints WHERE spec_version_id = ?")
            .bind(id)
            .execute(&mut *tx)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        for endpoint in &params.endpoints {
            sqlx::query(
                "INSERT INTO endpoints (spec_version_id, api_type, path, normalized_path, method, yaml_content, deprecated) VALUES (?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(id)
            .bind(endpoint.api_type.as_str())
            .bind(&endpoint.path)
            .bind(&endpoint.normalized_path)
            .bind(&endpoint.method)
            .bind(&endpoint.yaml_content)
            .bind(endpoint.deprecated)
            .execute(&mut *tx)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        }

        tx.commit()
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(id)
    }

    async fn find_spec_version(
        &self,
        service_id: i64,
        api_type: ApiType,
        version: SemVer,
    ) -> Result<Option<SpecVersionMeta>, RepositoryError> {
        let row: Option<SpecVersionRow> = sqlx::query_as(
            "SELECT id, service_id, api_type, major, minor, patch, stability, content_hash, provided_by, created_at, updated_at, last_required_at, trunk_provided_at \
             FROM spec_versions WHERE service_id = ? AND api_type = ? AND major = ? AND minor = ? AND patch = ?",
        )
        .bind(service_id)
        .bind(api_type.as_str())
        .bind(version.major as i64)
        .bind(version.minor as i64)
        .bind(version.patch as i64)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        row.map(spec_version_from_row).transpose()
    }

    async fn list_spec_versions(
        &self,
        service_id: i64,
    ) -> Result<Vec<SpecVersionMeta>, RepositoryError> {
        let rows: Vec<SpecVersionRow> = sqlx::query_as(
            "SELECT id, service_id, api_type, major, minor, patch, stability, content_hash, provided_by, created_at, updated_at, last_required_at, trunk_provided_at \
             FROM spec_versions WHERE service_id = ? ORDER BY api_type, major, minor, patch",
        )
        .bind(service_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        rows.into_iter().map(spec_version_from_row).collect()
    }

    async fn list_all_spec_versions(
        &self,
    ) -> Result<Vec<(String, SpecVersionMeta, i64)>, RepositoryError> {
        type Row = (
            String,
            i64,
            i64,
            String,
            i64,
            i64,
            i64,
            String,
            String,
            String,
            String,
            String,
            Option<String>,
            Option<String>,
            i64,
        );
        let rows: Vec<Row> = sqlx::query_as(
            r#"
            SELECT s.name, v.id, v.service_id, v.api_type, v.major, v.minor, v.patch, v.stability,
                   v.content_hash, v.provided_by, v.created_at, v.updated_at, v.last_required_at,
                   v.trunk_provided_at,
                   (SELECT COUNT(*) FROM endpoints e WHERE e.spec_version_id = v.id) AS endpoint_count
            FROM spec_versions v
            JOIN services s ON s.id = v.service_id
            ORDER BY s.name, v.api_type, v.major, v.minor, v.patch
            "#,
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;

        rows.into_iter()
            .map(|row| {
                let (
                    name,
                    id,
                    service_id,
                    api_type,
                    major,
                    minor,
                    patch,
                    stability,
                    content_hash,
                    provided_by,
                    created_at,
                    updated_at,
                    last_required_at,
                    trunk_provided_at,
                    endpoint_count,
                ) = row;
                let meta = spec_version_from_row((
                    id,
                    service_id,
                    api_type,
                    major,
                    minor,
                    patch,
                    stability,
                    content_hash,
                    provided_by,
                    created_at,
                    updated_at,
                    last_required_at,
                    trunk_provided_at,
                ))?;
                Ok((name, meta, endpoint_count))
            })
            .collect()
    }

    async fn get_spec_content(
        &self,
        spec_version_id: i64,
    ) -> Result<Option<String>, RepositoryError> {
        let row: Option<(String,)> =
            sqlx::query_as("SELECT content FROM spec_versions WHERE id = ?")
                .bind(spec_version_id)
                .fetch_optional(&self.pool)
                .await
                .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(row.map(|r| r.0))
    }

    async fn delete_spec_version(&self, spec_version_id: i64) -> Result<bool, RepositoryError> {
        let result = sqlx::query("DELETE FROM spec_versions WHERE id = ?")
            .bind(spec_version_id)
            .execute(&self.pool)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(result.rows_affected() > 0)
    }

    async fn touch_spec_version_required(
        &self,
        spec_version_id: i64,
        now_iso: &str,
    ) -> Result<(), RepositoryError> {
        sqlx::query("UPDATE spec_versions SET last_required_at = ? WHERE id = ?")
            .bind(now_iso)
            .bind(spec_version_id)
            .execute(&self.pool)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(())
    }

    async fn touch_spec_version_provided(
        &self,
        spec_version_id: i64,
        now_iso: &str,
    ) -> Result<(), RepositoryError> {
        sqlx::query("UPDATE spec_versions SET updated_at = ? WHERE id = ?")
            .bind(now_iso)
            .bind(spec_version_id)
            .execute(&self.pool)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(())
    }

    async fn touch_spec_version_trunk(
        &self,
        spec_version_id: i64,
        now_iso: &str,
    ) -> Result<(), RepositoryError> {
        sqlx::query("UPDATE spec_versions SET trunk_provided_at = ? WHERE id = ?")
            .bind(now_iso)
            .bind(spec_version_id)
            .execute(&self.pool)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(())
    }

    async fn insert_branch(
        &self,
        name: &str,
        created_at: &str,
        created_by: &str,
        source: &str,
        as_of: &str,
    ) -> Result<i64, RepositoryError> {
        let res = sqlx::query(
            "INSERT INTO sanshain_branches (name, created_at, created_by, source, as_of) VALUES (?, ?, ?, ?, ?)",
        )
        .bind(name)
        .bind(created_at)
        .bind(created_by)
        .bind(source)
        .bind(as_of)
        .execute(&self.pool)
        .await;
        match res {
            Ok(done) => Ok(done.last_insert_rowid()),
            Err(sqlx::Error::Database(e)) if e.is_unique_violation() => {
                Err(RepositoryError::Conflict)
            }
            Err(e) => Err(RepositoryError::Internal(e.to_string())),
        }
    }

    async fn find_branch(&self, name: &str) -> Result<Option<BranchInfo>, RepositoryError> {
        let row: Option<(i64, String, String, String, String, String)> = sqlx::query_as(
            "SELECT id, name, created_at, created_by, source, as_of FROM sanshain_branches WHERE name = ?",
        )
        .bind(name)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(row.map(
            |(id, name, created_at, created_by, source, as_of)| BranchInfo {
                id,
                name,
                created_at,
                created_by,
                source,
                as_of,
            },
        ))
    }

    async fn list_branches(&self) -> Result<Vec<BranchInfo>, RepositoryError> {
        let rows: Vec<(i64, String, String, String, String, String)> = sqlx::query_as(
            "SELECT id, name, created_at, created_by, source, as_of FROM sanshain_branches ORDER BY created_at DESC, id DESC",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(rows
            .into_iter()
            .map(
                |(id, name, created_at, created_by, source, as_of)| BranchInfo {
                    id,
                    name,
                    created_at,
                    created_by,
                    source,
                    as_of,
                },
            )
            .collect())
    }

    async fn copy_trunk_graph_to_branch(
        &self,
        branch_id: i64,
        as_of: &str,
        now_iso: &str,
    ) -> Result<(), RepositoryError> {
        sqlx::query(
            "INSERT INTO branch_dependencies (branch_id, client_id, service_id, api_type, path, normalized_path, method, major, minor, patch, valid_from, last_required_at) \
             SELECT ?, client_id, service_id, api_type, path, normalized_path, method, major, minor, patch, ?, ? \
             FROM trunk_dependencies WHERE valid_from <= ? AND (valid_to IS NULL OR valid_to > ?)",
        )
        .bind(branch_id)
        .bind(now_iso)
        .bind(now_iso)
        .bind(as_of)
        .bind(as_of)
        .execute(&self.pool)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(())
    }

    async fn copy_branch_graph_to_branch(
        &self,
        target_branch_id: i64,
        source_branch_id: i64,
        as_of: &str,
        now_iso: &str,
    ) -> Result<(), RepositoryError> {
        sqlx::query(
            "INSERT INTO branch_dependencies (branch_id, client_id, service_id, api_type, path, normalized_path, method, major, minor, patch, valid_from, last_required_at) \
             SELECT ?, client_id, service_id, api_type, path, normalized_path, method, major, minor, patch, ?, ? \
             FROM branch_dependencies WHERE branch_id = ? AND valid_from <= ? AND (valid_to IS NULL OR valid_to > ?)",
        )
        .bind(target_branch_id)
        .bind(now_iso)
        .bind(now_iso)
        .bind(source_branch_id)
        .bind(as_of)
        .bind(as_of)
        .execute(&self.pool)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(())
    }

    async fn list_branch_pins(
        &self,
        branch_id: i64,
        at: Option<&str>,
    ) -> Result<Vec<TrunkPinInfo>, RepositoryError> {
        let base = "SELECT c.name, s.name, t.api_type, t.major, t.minor, t.patch, t.path, t.method, t.valid_from, t.last_required_at \
             FROM branch_dependencies t \
             JOIN clients c ON c.id = t.client_id \
             JOIN services s ON s.id = t.service_id \
             WHERE t.branch_id = ?";
        let rows: Vec<PinRow<i64>> = match at {
            Some(at) => {
                sqlx::query_as(&format!(
                    "{base} AND t.valid_from <= ? AND (t.valid_to IS NULL OR t.valid_to > ?) ORDER BY c.name, s.name, t.path, t.method"
                ))
                .bind(branch_id)
                .bind(at)
                .bind(at)
                .fetch_all(&self.pool)
                .await
            }
            None => {
                sqlx::query_as(&format!(
                    "{base} AND t.valid_to IS NULL ORDER BY c.name, s.name, t.path, t.method"
                ))
                .bind(branch_id)
                .fetch_all(&self.pool)
                .await
            }
        }
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        rows.into_iter().map(pin_row_to_info).collect()
    }

    async fn list_trunk_pins_at(&self, at: &str) -> Result<Vec<TrunkPinInfo>, RepositoryError> {
        let rows: Vec<PinRow<i64>> = sqlx::query_as(
            "SELECT c.name, s.name, t.api_type, t.major, t.minor, t.patch, t.path, t.method, t.valid_from, t.last_required_at \
             FROM trunk_dependencies t \
             JOIN clients c ON c.id = t.client_id \
             JOIN services s ON s.id = t.service_id \
             WHERE t.valid_from <= ? AND (t.valid_to IS NULL OR t.valid_to > ?) \
             ORDER BY c.name, s.name, t.path, t.method",
        )
        .bind(at)
        .bind(at)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        rows.into_iter().map(pin_row_to_info).collect()
    }

    async fn list_graph_change_dates(
        &self,
        branch_id: Option<i64>,
    ) -> Result<Vec<String>, RepositoryError> {
        let rows: Vec<(String,)> = match branch_id {
            None => {
                sqlx::query_as(
                    "SELECT DISTINCT d FROM (SELECT valid_from AS d FROM trunk_dependencies UNION SELECT valid_to AS d FROM trunk_dependencies WHERE valid_to IS NOT NULL) ORDER BY d",
                )
                .fetch_all(&self.pool)
                .await
            }
            Some(bid) => {
                sqlx::query_as(
                    "SELECT DISTINCT d FROM (SELECT valid_from AS d FROM branch_dependencies WHERE branch_id = ?1 UNION SELECT valid_to AS d FROM branch_dependencies WHERE branch_id = ?1 AND valid_to IS NOT NULL) ORDER BY d",
                )
                .bind(bid)
                .fetch_all(&self.pool)
                .await
            }
        }
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(rows.into_iter().map(|(d,)| d).collect())
    }

    async fn list_branch_memberships_for_service(
        &self,
        service_id: i64,
    ) -> Result<Vec<BranchMembership>, RepositoryError> {
        type Row = (String, i64, i64, i64, String);
        let rows: Vec<Row> = sqlx::query_as(
            "SELECT api_type, major, minor, patch, name FROM ( \
                 SELECT d.api_type, d.major, d.minor, d.patch, b.name \
                 FROM branch_dependencies d JOIN sanshain_branches b ON b.id = d.branch_id \
                 WHERE d.service_id = ? AND d.valid_to IS NULL \
                 UNION \
                 SELECT m.api_type, m.major, m.minor, m.patch, b.name \
                 FROM branch_member_versions m JOIN sanshain_branches b ON b.id = m.branch_id \
                 WHERE m.service_id = ? AND m.valid_to IS NULL \
             ) ORDER BY name, api_type, major, minor, patch",
        )
        .bind(service_id)
        .bind(service_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        rows.into_iter()
            .map(|(api_type, major, minor, patch, branch)| {
                Ok(BranchMembership {
                    api_type: api_type
                        .parse()
                        .map_err(|e: String| RepositoryError::Internal(e))?,
                    version: SemVer::new(major as u32, minor as u32, patch as u32),
                    branch,
                })
            })
            .collect()
    }

    async fn list_branches_referencing(
        &self,
        service_id: i64,
        api_type: ApiType,
        version: SemVer,
    ) -> Result<Vec<String>, RepositoryError> {
        let rows: Vec<(String,)> = sqlx::query_as(
            "SELECT DISTINCT b.name FROM sanshain_branches b \
             WHERE EXISTS (SELECT 1 FROM branch_dependencies d WHERE d.branch_id = b.id AND d.valid_to IS NULL \
                           AND d.service_id = ?1 AND d.api_type = ?2 AND d.major = ?3 AND d.minor = ?4 AND d.patch = ?5) \
                OR EXISTS (SELECT 1 FROM branch_member_versions m WHERE m.branch_id = b.id AND m.valid_to IS NULL \
                           AND m.service_id = ?1 AND m.api_type = ?2 AND m.major = ?3 AND m.minor = ?4 AND m.patch = ?5) \
             ORDER BY b.name",
        )
        .bind(service_id)
        .bind(api_type.as_str())
        .bind(version.major)
        .bind(version.minor)
        .bind(version.patch)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(rows.into_iter().map(|(n,)| n).collect())
    }

    async fn close_expired_trunk_data(
        &self,
        cutoff_iso: &str,
        now_iso: &str,
    ) -> Result<u64, RepositoryError> {
        let closed = sqlx::query(
            "UPDATE trunk_dependencies SET valid_to = ? WHERE valid_to IS NULL AND last_required_at < ?",
        )
        .bind(now_iso)
        .bind(cutoff_iso)
        .execute(&self.pool)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        sqlx::query(
            "UPDATE spec_versions SET trunk_provided_at = NULL WHERE trunk_provided_at IS NOT NULL AND trunk_provided_at < ?",
        )
        .bind(cutoff_iso)
        .execute(&self.pool)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(closed.rows_affected())
    }

    async fn rename_branch(&self, branch_id: i64, new_name: &str) -> Result<(), RepositoryError> {
        let res = sqlx::query("UPDATE sanshain_branches SET name = ? WHERE id = ?")
            .bind(new_name)
            .bind(branch_id)
            .execute(&self.pool)
            .await;
        match res {
            Ok(_) => Ok(()),
            Err(sqlx::Error::Database(e)) if e.is_unique_violation() => {
                Err(RepositoryError::Conflict)
            }
            Err(e) => Err(RepositoryError::Internal(e.to_string())),
        }
    }

    async fn delete_branch(&self, branch_id: i64) -> Result<(), RepositoryError> {
        // The FK cascade needs sqlite's pragma; delete children explicitly so
        // the behavior does not depend on connection settings. One transaction,
        // because a failure between the children and the branch row would free
        // a branch's graph while leaving the branch — and its name — behind.
        // Postgres gets the same atomicity from the declared cascade.
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        for table in ["branch_dependencies", "branch_member_versions"] {
            sqlx::query(&format!("DELETE FROM {table} WHERE branch_id = ?"))
                .bind(branch_id)
                .execute(&mut *tx)
                .await
                .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        }
        sqlx::query("DELETE FROM sanshain_branches WHERE id = ?")
            .bind(branch_id)
            .execute(&mut *tx)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        tx.commit()
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(())
    }

    async fn record_branch_pins(
        &self,
        branch_id: i64,
        pins: Vec<RecordTrunkPinParams<'_>>,
    ) -> Result<(), RepositoryError> {
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        for p in pins {
            let key = "branch_id = ? AND client_id = ? AND service_id = ? AND api_type = ? AND normalized_path = ? AND method = ? AND valid_to IS NULL";
            sqlx::query(&format!(
                "UPDATE branch_dependencies SET valid_to = ? WHERE {key} AND NOT (major = ? AND minor = ? AND patch = ?)"
            ))
            .bind(p.now_iso)
            .bind(branch_id)
            .bind(p.client_id)
            .bind(p.service_id)
            .bind(p.api_type.as_str())
            .bind(p.normalized_path)
            .bind(p.method)
            .bind(p.version.major)
            .bind(p.version.minor)
            .bind(p.version.patch)
            .execute(&mut *tx)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
            let refreshed = sqlx::query(&format!(
                "UPDATE branch_dependencies SET last_required_at = ? WHERE {key} AND major = ? AND minor = ? AND patch = ?"
            ))
            .bind(p.now_iso)
            .bind(branch_id)
            .bind(p.client_id)
            .bind(p.service_id)
            .bind(p.api_type.as_str())
            .bind(p.normalized_path)
            .bind(p.method)
            .bind(p.version.major)
            .bind(p.version.minor)
            .bind(p.version.patch)
            .execute(&mut *tx)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
            if refreshed.rows_affected() == 0 {
                sqlx::query(
                    "INSERT INTO branch_dependencies (branch_id, client_id, service_id, api_type, path, normalized_path, method, major, minor, patch, valid_from, last_required_at) \
                     VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                )
                .bind(branch_id)
                .bind(p.client_id)
                .bind(p.service_id)
                .bind(p.api_type.as_str())
                .bind(p.path)
                .bind(p.normalized_path)
                .bind(p.method)
                .bind(p.version.major)
                .bind(p.version.minor)
                .bind(p.version.patch)
                .bind(p.now_iso)
                .bind(p.now_iso)
                .execute(&mut *tx)
                .await
                .map_err(|e| RepositoryError::Internal(e.to_string()))?;
            }
        }
        tx.commit()
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(())
    }

    async fn record_branch_member_version(
        &self,
        branch_id: i64,
        service_id: i64,
        api_type: ApiType,
        version: SemVer,
        now_iso: &str,
    ) -> Result<(), RepositoryError> {
        // Close-then-insert is one act: a failure between the two would leave
        // the branch with no open member version at all, hiding the producer
        // from the membership lookups until a later tagged provide repairs it.
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        let key = "branch_id = ? AND service_id = ? AND api_type = ? AND valid_to IS NULL";
        sqlx::query(&format!(
            "UPDATE branch_member_versions SET valid_to = ? WHERE {key} AND NOT (major = ? AND minor = ? AND patch = ?)"
        ))
        .bind(now_iso)
        .bind(branch_id)
        .bind(service_id)
        .bind(api_type.as_str())
        .bind(version.major)
        .bind(version.minor)
        .bind(version.patch)
        .execute(&mut *tx)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        sqlx::query(&format!(
            "INSERT INTO branch_member_versions (branch_id, service_id, api_type, major, minor, patch, valid_from) \
             SELECT ?, ?, ?, ?, ?, ?, ? \
             WHERE NOT EXISTS (SELECT 1 FROM branch_member_versions WHERE {key} AND major = ? AND minor = ? AND patch = ?)"
        ))
        .bind(branch_id)
        .bind(service_id)
        .bind(api_type.as_str())
        .bind(version.major)
        .bind(version.minor)
        .bind(version.patch)
        .bind(now_iso)
        .bind(branch_id)
        .bind(service_id)
        .bind(api_type.as_str())
        .bind(version.major)
        .bind(version.minor)
        .bind(version.patch)
        .execute(&mut *tx)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        tx.commit()
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(())
    }

    async fn record_trunk_pins(
        &self,
        pins: Vec<RecordTrunkPinParams<'_>>,
    ) -> Result<(), RepositoryError> {
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        for p in pins {
            let key = "client_id = ? AND service_id = ? AND api_type = ? AND normalized_path = ? AND method = ? AND valid_to IS NULL";
            // Append semantics: a different version closes the open record…
            sqlx::query(&format!(
                "UPDATE trunk_dependencies SET valid_to = ? WHERE {key} AND NOT (major = ? AND minor = ? AND patch = ?)"
            ))
            .bind(p.now_iso)
            .bind(p.client_id)
            .bind(p.service_id)
            .bind(p.api_type.as_str())
            .bind(p.normalized_path)
            .bind(p.method)
            .bind(p.version.major)
            .bind(p.version.minor)
            .bind(p.version.patch)
            .execute(&mut *tx)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
            // …the same version only refreshes the open record…
            let refreshed = sqlx::query(&format!(
                "UPDATE trunk_dependencies SET last_required_at = ? WHERE {key} AND major = ? AND minor = ? AND patch = ?"
            ))
            .bind(p.now_iso)
            .bind(p.client_id)
            .bind(p.service_id)
            .bind(p.api_type.as_str())
            .bind(p.normalized_path)
            .bind(p.method)
            .bind(p.version.major)
            .bind(p.version.minor)
            .bind(p.version.patch)
            .execute(&mut *tx)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
            // …and a new pin key or a closed record gets a fresh open one.
            if refreshed.rows_affected() == 0 {
                sqlx::query(
                    "INSERT INTO trunk_dependencies (client_id, service_id, api_type, path, normalized_path, method, major, minor, patch, valid_from, last_required_at) \
                     VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                )
                .bind(p.client_id)
                .bind(p.service_id)
                .bind(p.api_type.as_str())
                .bind(p.path)
                .bind(p.normalized_path)
                .bind(p.method)
                .bind(p.version.major)
                .bind(p.version.minor)
                .bind(p.version.patch)
                .bind(p.now_iso)
                .bind(p.now_iso)
                .execute(&mut *tx)
                .await
                .map_err(|e| RepositoryError::Internal(e.to_string()))?;
            }
        }
        tx.commit()
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(())
    }

    async fn list_current_trunk_pins(&self) -> Result<Vec<TrunkPinInfo>, RepositoryError> {
        let rows: Vec<PinRow<i64>> = sqlx::query_as(
            "SELECT c.name, s.name, t.api_type, t.major, t.minor, t.patch, t.path, t.method, t.valid_from, t.last_required_at \
             FROM trunk_dependencies t \
             JOIN clients c ON c.id = t.client_id \
             JOIN services s ON s.id = t.service_id \
             WHERE t.valid_to IS NULL \
             ORDER BY c.name, s.name, t.path, t.method",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        rows.into_iter().map(pin_row_to_info).collect()
    }

    async fn delete_expired_snapshots(&self, cutoff_iso: &str) -> Result<u64, RepositoryError> {
        // Use-based: a snapshot survives if it was provided (updated_at) OR
        // required (last_required_at) since the cutoff. GA never expires.
        let result = sqlx::query(
            r#"
            DELETE FROM spec_versions
            WHERE stability = 'snapshot'
              AND updated_at < ?
              AND (last_required_at IS NULL OR last_required_at < ?)
            "#,
        )
        .bind(cutoff_iso)
        .bind(cutoff_iso)
        .execute(&self.pool)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(result.rows_affected())
    }

    async fn list_version_dependents(
        &self,
        spec_version_id: i64,
    ) -> Result<Vec<String>, RepositoryError> {
        let rows: Vec<(String,)> = sqlx::query_as(
            r#"
            SELECT DISTINCT c.name
            FROM dependencies d
            JOIN clients c ON c.id = d.client_id
            WHERE d.spec_version_id = ?
            ORDER BY c.name
            "#,
        )
        .bind(spec_version_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(rows.into_iter().map(|r| r.0).collect())
    }

    async fn find_service(&self, name: &str) -> Result<Option<i64>, RepositoryError> {
        let row: Option<(i64,)> = sqlx::query_as("SELECT id FROM services WHERE name = ?")
            .bind(name)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(row.map(|r| r.0))
    }
    async fn get_service_name_by_id(
        &self,
        service_id: i64,
    ) -> Result<Option<String>, RepositoryError> {
        let row: Option<(String,)> = sqlx::query_as("SELECT name FROM services WHERE id = ?")
            .bind(service_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(row.map(|r| r.0))
    }
    async fn ensure_service(&self, name: &str) -> Result<i64, RepositoryError> {
        sqlx::query("INSERT OR IGNORE INTO services (name) VALUES (?)")
            .bind(name)
            .execute(&self.pool)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;

        let row: (i64,) = sqlx::query_as("SELECT id FROM services WHERE name = ?")
            .bind(name)
            .fetch_one(&self.pool)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;

        Ok(row.0)
    }

    async fn get_endpoints_for_version(
        &self,
        spec_version_id: i64,
    ) -> Result<Vec<EndpointRecord>, RepositoryError> {
        type EndpointRow = (i64, String, String, String, String, String, bool);
        let rows: Vec<EndpointRow> = sqlx::query_as(
            "SELECT e.id, e.api_type, e.path, e.normalized_path, e.method, e.yaml_content, \
             e.deprecated \
             FROM endpoints e \
             WHERE e.spec_version_id = ?",
        )
        .bind(spec_version_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;

        Ok(rows
            .into_iter()
            .map(
                |(id, api_type, path, normalized_path, method, yaml_content, deprecated)| {
                    EndpointRecord {
                        id: Some(id),
                        api_type: ApiType::from_str(&api_type).unwrap_or_default(),
                        path,
                        normalized_path,
                        method,
                        yaml_content,
                        deprecated,
                    }
                },
            )
            .collect())
    }

    async fn ensure_client(&self, name: &str) -> Result<i64, RepositoryError> {
        sqlx::query("INSERT OR IGNORE INTO clients (name) VALUES (?)")
            .bind(name)
            .execute(&self.pool)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;

        let row: (i64,) = sqlx::query_as("SELECT id FROM clients WHERE name = ?")
            .bind(name)
            .fetch_one(&self.pool)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;

        Ok(row.0)
    }

    async fn find_endpoint(
        &self,
        spec_version_id: i64,
        api_type: ApiType,
        path: &str,
        method: &str,
    ) -> Result<Option<(i64, String, bool)>, RepositoryError> {
        let normalized_path = crate::openapi::lookup_path(api_type, path);
        let row: Option<(i64, String, bool)> = sqlx::query_as(
            r#"
            SELECT e.id, e.yaml_content, e.deprecated
            FROM endpoints e
            WHERE e.spec_version_id = ? AND e.api_type = ? AND e.normalized_path = ? AND e.method = ?
            "#,
        )
        .bind(spec_version_id)
        .bind(api_type.as_str())
        .bind(normalized_path)
        .bind(method)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;

        Ok(row)
    }

    #[instrument(skip_all)]
    async fn find_endpoints_bulk(
        &self,
        spec_version_id: i64,
        api_type: ApiType,
        endpoints: &[(String, String)],
    ) -> Result<EndpointMap, RepositoryError> {
        let mut result = EndpointMap::new();
        if endpoints.is_empty() {
            return Ok(result);
        }

        let mut query_builder = sqlx::QueryBuilder::new(
            r#"
            SELECT e.id, e.path, e.normalized_path, e.method, e.yaml_content, e.deprecated
            FROM endpoints e
            WHERE e.spec_version_id = "#,
        );
        query_builder.push_bind(spec_version_id);
        query_builder.push(" AND e.api_type = ");
        query_builder.push_bind(api_type.as_str());
        query_builder.push(" AND (");

        for (i, (path, method)) in endpoints.iter().enumerate() {
            if i > 0 {
                query_builder.push(" OR ");
            }
            query_builder.push("(e.normalized_path = ");
            query_builder.push_bind(crate::openapi::lookup_path(api_type, path));
            query_builder.push(" AND e.method = ");
            query_builder.push_bind(method);
            query_builder.push(")");
        }
        query_builder.push(")");

        type BulkRow = (i64, String, String, String, String, bool);
        let rows: Vec<BulkRow> = query_builder
            .build_query_as()
            .fetch_all(&self.pool)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;

        // Key the result by what the caller *asked for*, matched via the
        // normalized path, so lenient path matching works for bundles too.
        let by_normalized: HashMap<(String, String), (i64, String, bool)> = rows
            .into_iter()
            .map(|(id, _path, normalized, method, yaml, deprecated)| {
                ((normalized, method), (id, yaml, deprecated))
            })
            .collect();
        for (path, method) in endpoints {
            let normalized = crate::openapi::lookup_path(api_type, path);
            if let Some(details) = by_normalized.get(&(normalized, method.clone())) {
                result.insert((path.clone(), method.clone()), details.clone());
            }
        }

        Ok(result)
    }

    async fn record_dependency(
        &self,
        params: RecordDependencyParams<'_>,
    ) -> Result<(), RepositoryError> {
        self.record_dependencies_bulk(vec![params]).await
    }

    #[instrument(skip_all)]
    async fn record_dependencies_bulk(
        &self,
        params: Vec<RecordDependencyParams<'_>>,
    ) -> Result<(), RepositoryError> {
        if params.is_empty() {
            return Ok(());
        }

        let now = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();

        let mut query_builder = sqlx::QueryBuilder::new(
            "INSERT INTO dependencies (client_id, spec_version_id, api_type, path, normalized_path, method, last_seen_at) ",
        );
        query_builder.push_values(params, |mut b, p| {
            b.push_bind(p.client_id)
                .push_bind(p.spec_version_id)
                .push_bind(p.api_type.as_str())
                .push_bind(p.path)
                .push_bind(p.normalized_path)
                .push_bind(p.method)
                .push_bind(&now);
        });
        query_builder.push(
            " ON CONFLICT(client_id, spec_version_id, api_type, path, method) DO UPDATE SET last_seen_at = excluded.last_seen_at",
        );
        query_builder
            .build()
            .execute(&self.pool)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;

        Ok(())
    }

    #[instrument(skip_all)]
    async fn get_report(&self) -> Result<DependencyReport, RepositoryError> {
        let mut conn = self
            .pool
            .acquire()
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;

        // Every recorded Pin, with the pinned version's current stability and
        // whether the endpoint still exists in that version (a snapshot
        // overwrite can drop an endpoint under a recorded dependency).
        type DepRow = (
            String,
            String,
            String,
            i64,
            i64,
            i64,
            String,
            String,
            String,
            bool,
            bool,
        );
        let dependency_rows: Vec<DepRow> = sqlx::query_as(
            r#"
            SELECT c.name, d.api_type, s.name, v.major, v.minor, v.patch, v.stability,
                   d.path, d.method,
                   COALESCE(e.deprecated, FALSE),
                   (e.id IS NOT NULL)
            FROM dependencies d
            JOIN clients c ON d.client_id = c.id
            JOIN spec_versions v ON d.spec_version_id = v.id
            JOIN services s ON v.service_id = s.id
            LEFT JOIN endpoints e ON e.spec_version_id = v.id AND e.api_type = d.api_type
                 AND e.normalized_path = d.normalized_path AND e.method = d.method
            "#,
        )
        .fetch_all(&mut *conn)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;

        let mut dependency_graph = Vec::new();
        let mut missing_endpoints = Vec::new();
        for (
            client,
            api_type,
            service,
            major,
            minor,
            patch,
            stability,
            path,
            method,
            deprecated,
            exists,
        ) in dependency_rows
        {
            let api_type = ApiType::from_str(&api_type).unwrap_or_default();
            let version = SemVer::new(major as u32, minor as u32, patch as u32);
            let stability: Stability = stability
                .parse()
                .map_err(|e: String| RepositoryError::Internal(e))?;
            if exists {
                dependency_graph.push(DependencyInfo {
                    api_type,
                    client,
                    service,
                    version,
                    stability,
                    path,
                    method,
                    deprecated,
                });
            } else {
                missing_endpoints.push(MissingEndpointInfo {
                    api_type,
                    client,
                    service,
                    version,
                    path,
                    method,
                });
            }
        }

        // Endpoints nobody requires, per version-line entry.
        type UnusedRow = (String, String, i64, i64, i64, String, String, String, bool);
        let unused_rows: Vec<UnusedRow> = sqlx::query_as(
            r#"
            SELECT s.name, e.api_type, v.major, v.minor, v.patch, v.stability, e.path, e.method, e.deprecated
            FROM endpoints e
            JOIN spec_versions v ON e.spec_version_id = v.id
            JOIN services s ON v.service_id = s.id
            WHERE NOT EXISTS (
                SELECT 1 FROM dependencies d
                WHERE d.spec_version_id = v.id AND d.api_type = e.api_type
                  AND d.normalized_path = e.normalized_path AND d.method = e.method
            )
            "#,
        )
        .fetch_all(&mut *conn)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;

        let unused_endpoints = unused_rows
            .into_iter()
            .map(
                |(service, api_type, major, minor, patch, stability, path, method, deprecated)| {
                    Ok(EndpointInfo {
                        api_type: ApiType::from_str(&api_type).unwrap_or_default(),
                        service,
                        version: SemVer::new(major as u32, minor as u32, patch as u32),
                        stability: stability
                            .parse()
                            .map_err(|e: String| RepositoryError::Internal(e))?,
                        path,
                        method,
                        deprecated,
                    })
                },
            )
            .collect::<Result<Vec<_>, RepositoryError>>()?;

        Ok(DependencyReport {
            trunk_graph: Vec::new(),
            trunk_stale_before: None,
            scope_label: None,
            unused_endpoints,
            missing_endpoints,
            dependency_graph,
            service_tags: HashMap::new(),
        })
    }

    async fn delete_all_services(&self) -> Result<u64, RepositoryError> {
        sqlx::query("DELETE FROM dependencies")
            .execute(&self.pool)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        sqlx::query("DELETE FROM endpoints")
            .execute(&self.pool)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        sqlx::query("DELETE FROM spec_versions")
            .execute(&self.pool)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        let result = sqlx::query("DELETE FROM services")
            .execute(&self.pool)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(result.rows_affected())
    }

    async fn delete_all_clients(&self) -> Result<u64, RepositoryError> {
        sqlx::query("DELETE FROM dependencies")
            .execute(&self.pool)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        let result = sqlx::query("DELETE FROM clients")
            .execute(&self.pool)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(result.rows_affected())
    }

    /// Delete every user who does not hold the `admin` role.
    ///
    /// Keys on the role grant rather than the flag it replaced, so the set it
    /// spares is the same set the permission checks treat as administrators.
    async fn delete_all_non_admin_users(
        &self,
        spare_usernames: &[String],
    ) -> Result<u64, RepositoryError> {
        // A user survives the purge when they are an effective administrator:
        // direct `admin` role, `admin` via group membership, or a
        // configuration-held root username (`spare_usernames`).
        const DOOMED: &str = "SELECT id FROM users \
             WHERE NOT EXISTS (SELECT 1 FROM user_roles r WHERE r.user_id = users.id AND r.role = 'admin') \
             AND NOT EXISTS (SELECT 1 FROM user_group_members m \
                 JOIN user_group_roles gr ON gr.group_id = m.group_id AND gr.role = 'admin' \
                 WHERE m.user_id = users.id) \
             AND users.username NOT IN (SELECT value FROM json_each(?))";
        let spare_json = serde_json::to_string(spare_usernames)
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        for table in ["sessions", "api_tokens"] {
            sqlx::query(&format!("DELETE FROM {table} WHERE user_id IN ({DOOMED})"))
                .bind(&spare_json)
                .execute(&self.pool)
                .await
                .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        }
        let result = sqlx::query(&format!("DELETE FROM users WHERE id IN ({DOOMED})"))
            .bind(&spare_json)
            .execute(&self.pool)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(result.rows_affected())
    }

    async fn nuke_database(&self, keep_user_id: Option<i64>) -> Result<(), RepositoryError> {
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;

        sqlx::query("DELETE FROM dependencies")
            .execute(&mut *tx)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        sqlx::query("DELETE FROM endpoints")
            .execute(&mut *tx)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        sqlx::query("DELETE FROM spec_versions")
            .execute(&mut *tx)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        // Release cuts are instance data too: without this the branch list and
        // the graph selector survive a factory reset as empty shells whose
        // names stay claimed. Their pin rows go with the participants.
        sqlx::query("DELETE FROM sanshain_branches")
            .execute(&mut *tx)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        sqlx::query("DELETE FROM channel_message_contracts")
            .execute(&mut *tx)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        sqlx::query("DELETE FROM services")
            .execute(&mut *tx)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        sqlx::query("DELETE FROM clients")
            .execute(&mut *tx)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        sqlx::query("DELETE FROM service_tags")
            .execute(&mut *tx)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        // Delete all sessions except the current admin's
        if let Some(uid) = keep_user_id {
            sqlx::query("DELETE FROM sessions WHERE user_id != ?")
                .bind(uid)
                .execute(&mut *tx)
                .await
                .map_err(|e| RepositoryError::Internal(e.to_string()))?;
            sqlx::query("DELETE FROM api_tokens WHERE user_id != ?")
                .bind(uid)
                .execute(&mut *tx)
                .await
                .map_err(|e| RepositoryError::Internal(e.to_string()))?;
            sqlx::query("DELETE FROM users WHERE id != ?")
                .bind(uid)
                .execute(&mut *tx)
                .await
                .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        } else {
            sqlx::query("DELETE FROM sessions")
                .execute(&mut *tx)
                .await
                .map_err(|e| RepositoryError::Internal(e.to_string()))?;
            sqlx::query("DELETE FROM api_tokens")
                .execute(&mut *tx)
                .await
                .map_err(|e| RepositoryError::Internal(e.to_string()))?;
            sqlx::query("DELETE FROM users")
                .execute(&mut *tx)
                .await
                .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        }

        tx.commit()
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;

        Ok(())
    }

    async fn delete_producer(&self, name: &str) -> Result<bool, RepositoryError> {
        // spec_versions cascade endpoints and dependencies via FK; SQLite
        // enforces them only with the pragma on, so delete explicitly.
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;

        let row: Option<(i64,)> = sqlx::query_as("SELECT id FROM services WHERE name = ?")
            .bind(name)
            .fetch_optional(&mut *tx)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;

        let service_id = match row {
            Some((id,)) => id,
            None => return Ok(false),
        };

        sqlx::query(
            "DELETE FROM dependencies WHERE spec_version_id IN (SELECT id FROM spec_versions WHERE service_id = ?)",
        )
        .bind(service_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        sqlx::query(
            "DELETE FROM endpoints WHERE spec_version_id IN (SELECT id FROM spec_versions WHERE service_id = ?)",
        )
        .bind(service_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        sqlx::query("DELETE FROM spec_versions WHERE service_id = ?")
            .bind(service_id)
            .execute(&mut *tx)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        sqlx::query("DELETE FROM services WHERE id = ?")
            .bind(service_id)
            .execute(&mut *tx)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;

        tx.commit()
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;

        Ok(true)
    }

    async fn delete_consumer(&self, name: &str) -> Result<bool, RepositoryError> {
        let row: Option<(i64,)> = sqlx::query_as("SELECT id FROM clients WHERE name = ?")
            .bind(name)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;

        let client_id = match row {
            Some((id,)) => id,
            None => return Ok(false),
        };

        sqlx::query("DELETE FROM dependencies WHERE client_id = ?")
            .bind(client_id)
            .execute(&self.pool)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;

        sqlx::query("DELETE FROM clients WHERE id = ?")
            .bind(client_id)
            .execute(&self.pool)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;

        Ok(true)
    }

    async fn list_producers(&self) -> Result<Vec<String>, RepositoryError> {
        let rows: Vec<(String,)> = sqlx::query_as("SELECT name FROM services ORDER BY name")
            .fetch_all(&self.pool)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(rows.into_iter().map(|r| r.0).collect())
    }

    async fn list_producers_detailed(&self) -> Result<Vec<ProducerSummary>, RepositoryError> {
        let rows: Vec<(String, Option<String>, Option<String>)> =
            sqlx::query_as("SELECT name, icon, domain FROM services ORDER BY name")
                .fetch_all(&self.pool)
                .await
                .map_err(|e| RepositoryError::Internal(e.to_string()))?;

        Ok(rows
            .into_iter()
            .map(|(name, icon, domain)| ProducerSummary {
                name,
                versions: Vec::new(),
                is_favorite: false,
                icon,
                domain,
            })
            .collect())
    }

    async fn update_producer_metadata(
        &self,
        service_name: &str,
        icon: Option<&str>,
        domain: Option<&str>,
    ) -> Result<(), RepositoryError> {
        sqlx::query("UPDATE services SET icon = ?, domain = ? WHERE name = ?")
            .bind(icon)
            .bind(domain)
            .bind(service_name)
            .execute(&self.pool)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(())
    }

    async fn list_consumers(&self) -> Result<Vec<String>, RepositoryError> {
        let rows: Vec<(String,)> = sqlx::query_as(
            "SELECT DISTINCT c.name FROM clients c INNER JOIN dependencies d ON d.client_id = c.id ORDER BY c.name"
        )
            .fetch_all(&self.pool)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(rows.into_iter().map(|r| r.0).collect())
    }

    async fn list_consumer_endpoints(
        &self,
        client_name: &str,
    ) -> Result<Vec<ConsumerEndpointInfo>, RepositoryError> {
        type ClientEndpointRow = (
            String,
            String,
            i64,
            i64,
            i64,
            String,
            String,
            String,
            Option<String>,
            bool,
        );
        let rows: Vec<ClientEndpointRow> = sqlx::query_as(
            "SELECT d.api_type, s.name, v.major, v.minor, v.patch, v.stability, d.path, d.method, e.yaml_content, \
             COALESCE(e.deprecated, 0) \
             FROM dependencies d \
             JOIN clients c ON d.client_id = c.id \
             JOIN spec_versions v ON d.spec_version_id = v.id \
             JOIN services s ON v.service_id = s.id \
             LEFT JOIN endpoints e ON e.spec_version_id = v.id AND e.api_type = d.api_type \
                  AND e.normalized_path = d.normalized_path AND e.method = d.method \
             WHERE c.name = ? \
             ORDER BY s.name, d.path, d.method",
        )
        .bind(client_name)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        rows.into_iter()
            .map(
                |(
                    api_type,
                    service,
                    major,
                    minor,
                    patch,
                    stability,
                    path,
                    method,
                    yaml_content,
                    deprecated,
                )| {
                    Ok(ConsumerEndpointInfo {
                        api_type: ApiType::from_str(&api_type).unwrap_or_default(),
                        service,
                        version: SemVer::new(major as u32, minor as u32, patch as u32),
                        stability: stability
                            .parse()
                            .map_err(|e: String| RepositoryError::Internal(e))?,
                        path,
                        method,
                        yaml_content,
                        deprecated,
                    })
                },
            )
            .collect()
    }

    async fn user_count(&self) -> Result<i64, RepositoryError> {
        let row: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM users")
            .fetch_one(&self.pool)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(row.0)
    }

    async fn find_user(&self, username: &str) -> Result<Option<User>, RepositoryError> {
        let row: Option<(i64, String, String, bool)> = sqlx::query_as(
            "SELECT id, username, password_hash, approved FROM users WHERE username = ?",
        )
        .bind(username)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;

        Ok(row.map(|(id, username, password_hash, approved)| User {
            id,
            username,
            password_hash,
            approved,
        }))
    }

    async fn create_user(
        &self,
        username: &str,
        password_hash: &str,
        approved: bool,
    ) -> Result<User, RepositoryError> {
        sqlx::query("INSERT INTO users (username, password_hash, approved) VALUES (?, ?, ?)")
            .bind(username)
            .bind(password_hash)
            .bind(approved)
            .execute(&self.pool)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;

        let row: (i64, String, String, bool) = sqlx::query_as(
            "SELECT id, username, password_hash, approved FROM users WHERE username = ?",
        )
        .bind(username)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;

        Ok(User {
            id: row.0,
            username: row.1,
            password_hash: row.2,
            approved: row.3,
        })
    }

    async fn list_users(&self) -> Result<Vec<User>, RepositoryError> {
        let rows: Vec<(i64, String, String, bool)> = sqlx::query_as(
            "SELECT id, username, password_hash, approved FROM users ORDER BY username",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;

        Ok(rows
            .into_iter()
            .map(|(id, username, password_hash, approved)| User {
                id,
                username,
                password_hash,
                approved,
            })
            .collect())
    }

    async fn approve_user(&self, user_id: i64) -> Result<bool, RepositoryError> {
        let result =
            sqlx::query("UPDATE users SET approved = TRUE WHERE id = ? AND approved = FALSE")
                .bind(user_id)
                .execute(&self.pool)
                .await
                .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(result.rows_affected() > 0)
    }

    async fn delete_user(&self, user_id: i64) -> Result<bool, RepositoryError> {
        let result = sqlx::query("DELETE FROM users WHERE id = ?")
            .bind(user_id)
            .execute(&self.pool)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(result.rows_affected() > 0)
    }

    async fn update_password(&self, user_id: i64, new_hash: &str) -> Result<(), RepositoryError> {
        sqlx::query("UPDATE users SET password_hash = ? WHERE id = ?")
            .bind(new_hash)
            .bind(user_id)
            .execute(&self.pool)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(())
    }

    async fn create_session(
        &self,
        user_id: i64,
        expires_at: &str,
    ) -> Result<Session, RepositoryError> {
        use rand::RngExt;
        let mut token_bytes = [0u8; 32];
        rand::rng().fill(&mut token_bytes);
        let token = hex::encode(token_bytes);
        self.create_session_with_token(user_id, &token, expires_at)
            .await
    }

    async fn create_session_with_token(
        &self,
        user_id: i64,
        token: &str,
        expires_at: &str,
    ) -> Result<Session, RepositoryError> {
        let token_hash = hash_session_token(token);
        sqlx::query("INSERT INTO sessions (token, user_id, expires_at) VALUES (?, ?, ?)")
            .bind(&token_hash)
            .bind(user_id)
            .bind(expires_at)
            .execute(&self.pool)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;

        Ok(Session {
            token: token.to_string(),
            user_id,
            expires_at: expires_at.to_string(),
        })
    }

    async fn validate_session(
        &self,
        token: &str,
    ) -> Result<Option<(User, Session)>, RepositoryError> {
        let row: Option<(i64, i64, String, String, String, bool)> = sqlx::query_as(
            r#"
            SELECT s.user_id, u.id, u.username, u.password_hash, s.expires_at, u.approved
            FROM sessions s
            JOIN users u ON s.user_id = u.id
            WHERE s.token = ? AND s.expires_at > datetime('now')
            "#,
        )
        .bind(hash_session_token(token))
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;

        Ok(row.map(
            |(_, user_id, username, password_hash, expires_at, approved)| {
                let user = User {
                    id: user_id,
                    username,
                    password_hash,
                    approved,
                };
                let session = Session {
                    token: token.to_string(),
                    user_id,
                    expires_at,
                };
                (user, session)
            },
        ))
    }

    async fn delete_session(&self, token: &str) -> Result<(), RepositoryError> {
        sqlx::query("DELETE FROM sessions WHERE token = ?")
            .bind(hash_session_token(token))
            .execute(&self.pool)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(())
    }

    async fn get_setting(&self, key: &str) -> Result<Option<String>, RepositoryError> {
        let row: Option<(String,)> = sqlx::query_as("SELECT value FROM settings WHERE key = ?")
            .bind(key)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(row.map(|r| r.0))
    }

    async fn set_setting(&self, key: &str, value: &str) -> Result<(), RepositoryError> {
        sqlx::query("INSERT OR REPLACE INTO settings (key, value) VALUES (?, ?)")
            .bind(key)
            .bind(value)
            .execute(&self.pool)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(())
    }

    // --- API Tokens ---

    async fn create_api_token(
        &self,
        id: &str,
        user_id: i64,
        name: &str,
        token_hash: &str,
        created_at: &str,
        expires_at: &str,
    ) -> Result<(), RepositoryError> {
        sqlx::query("INSERT INTO api_tokens (id, user_id, name, token_hash, created_at, expires_at) VALUES (?, ?, ?, ?, ?, ?)")
            .bind(id)
            .bind(user_id)
            .bind(name)
            .bind(token_hash)
            .bind(created_at)
            .bind(expires_at)
            .execute(&self.pool)
            .await
            .map_err(|e| {
                let msg = e.to_string();
                if msg.contains("UNIQUE") {
                    RepositoryError::Conflict
                } else {
                    RepositoryError::Internal(msg)
                }
            })?;
        Ok(())
    }

    async fn list_api_tokens(&self, user_id: i64) -> Result<Vec<ApiToken>, RepositoryError> {
        let rows: Vec<ApiTokenRow> = sqlx::query_as(
            "SELECT id, user_id, name, token_hash, created_at, expires_at, last_used_at FROM api_tokens WHERE user_id = ? ORDER BY created_at DESC"
        )
        .bind(user_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;

        Ok(rows.into_iter().map(ApiToken::from).collect())
    }

    async fn delete_api_token(
        &self,
        token_id: &str,
        user_id: i64,
    ) -> Result<bool, RepositoryError> {
        let result = sqlx::query("DELETE FROM api_tokens WHERE id = ? AND user_id = ?")
            .bind(token_id)
            .bind(user_id)
            .execute(&self.pool)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(result.rows_affected() > 0)
    }

    async fn validate_api_token(&self, token_hash: &str) -> Result<Option<User>, RepositoryError> {
        let row: Option<(i64, String, String, bool)> = sqlx::query_as(
            r#"
            SELECT u.id, u.username, u.password_hash, u.approved
            FROM api_tokens t
            JOIN users u ON t.user_id = u.id
            WHERE t.token_hash = ? AND t.expires_at > datetime('now')
            "#,
        )
        .bind(token_hash)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;

        if row.is_some() {
            // Update last_used_at
            let _ = sqlx::query(
                "UPDATE api_tokens SET last_used_at = datetime('now') WHERE token_hash = ?",
            )
            .bind(token_hash)
            .execute(&self.pool)
            .await;
        }

        Ok(row.map(|(id, username, password_hash, approved)| User {
            id,
            username,
            password_hash,
            approved,
        }))
    }

    async fn delete_stale_dependencies(&self, cutoff_iso: &str) -> Result<u64, RepositoryError> {
        let result = sqlx::query("DELETE FROM dependencies WHERE last_seen_at < ?")
            .bind(cutoff_iso)
            .execute(&self.pool)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(result.rows_affected())
    }

    async fn grant_user_role(&self, user_id: i64, role: &str) -> Result<(), RepositoryError> {
        sqlx::query("INSERT OR IGNORE INTO user_roles (user_id, role) VALUES (?, ?)")
            .bind(user_id)
            .bind(role)
            .execute(&self.pool)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(())
    }

    async fn revoke_user_role(&self, user_id: i64, role: &str) -> Result<bool, RepositoryError> {
        let result = sqlx::query("DELETE FROM user_roles WHERE user_id = ? AND role = ?")
            .bind(user_id)
            .bind(role)
            .execute(&self.pool)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(result.rows_affected() > 0)
    }

    async fn list_user_roles(&self, user_id: i64) -> Result<Vec<String>, RepositoryError> {
        let rows: Vec<(String,)> =
            sqlx::query_as("SELECT role FROM user_roles WHERE user_id = ? ORDER BY role")
                .bind(user_id)
                .fetch_all(&self.pool)
                .await
                .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(rows.into_iter().map(|r| r.0).collect())
    }

    async fn effective_stored_roles(&self, user_id: i64) -> Result<Vec<String>, RepositoryError> {
        let rows: Vec<(String,)> = sqlx::query_as(
            "SELECT role FROM user_roles WHERE user_id = ?
             UNION
             SELECT gr.role FROM user_group_roles gr
               JOIN user_group_members gm ON gm.group_id = gr.group_id
             WHERE gm.user_id = ?
             ORDER BY role",
        )
        .bind(user_id)
        .bind(user_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(rows.into_iter().map(|r| r.0).collect())
    }

    async fn create_group(
        &self,
        name: &str,
        source: GroupSource,
    ) -> Result<Group, RepositoryError> {
        sqlx::query("INSERT OR IGNORE INTO user_groups (name, source) VALUES (?, ?)")
            .bind(name)
            .bind(source.as_str())
            .execute(&self.pool)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        let row: (i64,) =
            sqlx::query_as("SELECT id FROM user_groups WHERE name = ? AND source = ?")
                .bind(name)
                .bind(source.as_str())
                .fetch_one(&self.pool)
                .await
                .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(Group {
            id: row.0,
            name: name.to_string(),
            source,
        })
    }

    async fn rename_group(&self, group_id: i64, name: &str) -> Result<bool, RepositoryError> {
        let result = sqlx::query("UPDATE user_groups SET name = ? WHERE id = ?")
            .bind(name)
            .bind(group_id)
            .execute(&self.pool)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(result.rows_affected() > 0)
    }

    async fn delete_group(&self, group_id: i64) -> Result<bool, RepositoryError> {
        // Membership and role grants are removed explicitly rather than relying
        // on cascade, which SQLite only applies when foreign keys are enabled.
        sqlx::query("DELETE FROM user_group_members WHERE group_id = ?")
            .bind(group_id)
            .execute(&self.pool)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        sqlx::query("DELETE FROM user_group_roles WHERE group_id = ?")
            .bind(group_id)
            .execute(&self.pool)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        let result = sqlx::query("DELETE FROM user_groups WHERE id = ?")
            .bind(group_id)
            .execute(&self.pool)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(result.rows_affected() > 0)
    }

    async fn list_groups(&self) -> Result<Vec<Group>, RepositoryError> {
        let rows: Vec<(i64, String, String)> =
            sqlx::query_as("SELECT id, name, source FROM user_groups ORDER BY source, name")
                .fetch_all(&self.pool)
                .await
                .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(rows
            .into_iter()
            .filter_map(|(id, name, source)| {
                GroupSource::parse(&source).map(|source| Group { id, name, source })
            })
            .collect())
    }

    async fn set_group_roles(
        &self,
        group_id: i64,
        roles: &[String],
    ) -> Result<(), RepositoryError> {
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        sqlx::query("DELETE FROM user_group_roles WHERE group_id = ?")
            .bind(group_id)
            .execute(&mut *tx)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        for role in roles {
            sqlx::query("INSERT OR IGNORE INTO user_group_roles (group_id, role) VALUES (?, ?)")
                .bind(group_id)
                .bind(role)
                .execute(&mut *tx)
                .await
                .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        }
        tx.commit()
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(())
    }

    async fn list_group_roles(&self, group_id: i64) -> Result<Vec<String>, RepositoryError> {
        let rows: Vec<(String,)> =
            sqlx::query_as("SELECT role FROM user_group_roles WHERE group_id = ? ORDER BY role")
                .bind(group_id)
                .fetch_all(&self.pool)
                .await
                .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(rows.into_iter().map(|r| r.0).collect())
    }

    async fn add_group_member(&self, group_id: i64, user_id: i64) -> Result<(), RepositoryError> {
        sqlx::query("INSERT OR IGNORE INTO user_group_members (group_id, user_id) VALUES (?, ?)")
            .bind(group_id)
            .bind(user_id)
            .execute(&self.pool)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(())
    }

    async fn remove_group_member(
        &self,
        group_id: i64,
        user_id: i64,
    ) -> Result<bool, RepositoryError> {
        let result =
            sqlx::query("DELETE FROM user_group_members WHERE group_id = ? AND user_id = ?")
                .bind(group_id)
                .bind(user_id)
                .execute(&self.pool)
                .await
                .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(result.rows_affected() > 0)
    }

    async fn list_group_member_ids(&self, group_id: i64) -> Result<Vec<i64>, RepositoryError> {
        let rows: Vec<(i64,)> = sqlx::query_as(
            "SELECT user_id FROM user_group_members WHERE group_id = ? ORDER BY user_id",
        )
        .bind(group_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(rows.into_iter().map(|r| r.0).collect())
    }

    // --- Pending Specs ---

    async fn add_user_maintainer(
        &self,
        service_id: i64,
        user_id: i64,
    ) -> Result<(), RepositoryError> {
        sqlx::query(
            "INSERT OR IGNORE INTO producer_user_maintainers (service_id, user_id) VALUES (?, ?)",
        )
        .bind(service_id)
        .bind(user_id)
        .execute(&self.pool)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(())
    }

    async fn remove_user_maintainer(
        &self,
        service_id: i64,
        user_id: i64,
    ) -> Result<bool, RepositoryError> {
        let result = sqlx::query(
            "DELETE FROM producer_user_maintainers WHERE service_id = ? AND user_id = ?",
        )
        .bind(service_id)
        .bind(user_id)
        .execute(&self.pool)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(result.rows_affected() > 0)
    }

    async fn add_group_maintainer(
        &self,
        service_id: i64,
        group_id: i64,
    ) -> Result<(), RepositoryError> {
        sqlx::query(
            "INSERT OR IGNORE INTO producer_group_maintainers (service_id, group_id) VALUES (?, ?)",
        )
        .bind(service_id)
        .bind(group_id)
        .execute(&self.pool)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(())
    }

    async fn remove_group_maintainer(
        &self,
        service_id: i64,
        group_id: i64,
    ) -> Result<bool, RepositoryError> {
        let result = sqlx::query(
            "DELETE FROM producer_group_maintainers WHERE service_id = ? AND group_id = ?",
        )
        .bind(service_id)
        .bind(group_id)
        .execute(&self.pool)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(result.rows_affected() > 0)
    }

    async fn list_user_maintainer_ids(&self, service_id: i64) -> Result<Vec<i64>, RepositoryError> {
        let rows: Vec<(i64,)> = sqlx::query_as(
            "SELECT user_id FROM producer_user_maintainers WHERE service_id = ? ORDER BY user_id",
        )
        .bind(service_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(rows.into_iter().map(|r| r.0).collect())
    }

    async fn list_group_maintainer_ids(
        &self,
        service_id: i64,
    ) -> Result<Vec<i64>, RepositoryError> {
        let rows: Vec<(i64,)> = sqlx::query_as(
            "SELECT group_id FROM producer_group_maintainers WHERE service_id = ? ORDER BY group_id",
        )
        .bind(service_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(rows.into_iter().map(|r| r.0).collect())
    }

    async fn list_all_user_maintainers(&self) -> Result<Vec<(String, i64)>, RepositoryError> {
        sqlx::query_as(
            "SELECT s.name, m.user_id FROM producer_user_maintainers m \
             JOIN services s ON s.id = m.service_id ORDER BY s.name, m.user_id",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))
    }

    async fn list_all_group_maintainers(&self) -> Result<Vec<(String, i64)>, RepositoryError> {
        sqlx::query_as(
            "SELECT s.name, m.group_id FROM producer_group_maintainers m \
             JOIN services s ON s.id = m.service_id ORDER BY s.name, m.group_id",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))
    }

    async fn list_group_maintained_producers(
        &self,
        group_ids: &[i64],
    ) -> Result<Vec<String>, RepositoryError> {
        let ids_json = serde_json::to_string(group_ids)
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        let rows: Vec<(String,)> = sqlx::query_as(
            "SELECT DISTINCT s.name FROM producer_group_maintainers pgm \
             JOIN services s ON s.id = pgm.service_id \
             WHERE pgm.group_id IN (SELECT value FROM json_each(?)) ORDER BY s.name",
        )
        .bind(ids_json)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(rows.into_iter().map(|r| r.0).collect())
    }

    async fn maintains_producer(
        &self,
        user_id: i64,
        service_id: i64,
    ) -> Result<bool, RepositoryError> {
        let row: (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM (
                 SELECT 1 FROM producer_user_maintainers
                  WHERE service_id = ? AND user_id = ?
                 UNION ALL
                 SELECT 1 FROM producer_group_maintainers pgm
                   JOIN user_group_members gm ON gm.group_id = pgm.group_id
                  WHERE pgm.service_id = ? AND gm.user_id = ?
             ) AS assignments",
        )
        .bind(service_id)
        .bind(user_id)
        .bind(service_id)
        .bind(user_id)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(row.0 > 0)
    }

    async fn list_maintained_producers(
        &self,
        user_id: i64,
    ) -> Result<Vec<String>, RepositoryError> {
        let rows: Vec<(String,)> = sqlx::query_as(
            "SELECT s.name FROM services s
               JOIN producer_user_maintainers m ON m.service_id = s.id
              WHERE m.user_id = ?
             UNION
             SELECT s.name FROM services s
               JOIN producer_group_maintainers pgm ON pgm.service_id = s.id
               JOIN user_group_members gm ON gm.group_id = pgm.group_id
              WHERE gm.user_id = ?
             ORDER BY name",
        )
        .bind(user_id)
        .bind(user_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(rows.into_iter().map(|r| r.0).collect())
    }

    async fn add_service_tags(
        &self,
        service_id: i64,
        tags: &[String],
    ) -> Result<(), RepositoryError> {
        for tag in tags {
            sqlx::query("INSERT OR IGNORE INTO service_tags (service_id, tag) VALUES (?, ?)")
                .bind(service_id)
                .bind(tag)
                .execute(&self.pool)
                .await
                .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        }
        Ok(())
    }

    async fn get_all_service_tags(&self) -> Result<HashMap<String, Vec<String>>, RepositoryError> {
        let rows: Vec<(String, String)> = sqlx::query_as(
            "SELECT s.name, st.tag FROM service_tags st JOIN services s ON s.id = st.service_id ORDER BY s.name, st.tag"
        )
            .fetch_all(&self.pool)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;

        let mut result: HashMap<String, Vec<String>> = HashMap::new();
        for (name, tag) in rows {
            result.entry(name).or_default().push(tag);
        }
        Ok(result)
    }

    async fn get_channel_message_contract(
        &self,
        channel: &str,
        message_name: &str,
    ) -> Result<Option<ChannelMessageContract>, RepositoryError> {
        let row: Option<(i64, String)> = sqlx::query_as(
            "SELECT owner_service_id, payload_yaml FROM channel_message_contracts WHERE channel = ? AND message_name = ?",
        )
        .bind(channel)
        .bind(message_name)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;

        Ok(
            row.map(|(owner_service_id, payload_yaml)| ChannelMessageContract {
                channel: channel.to_string(),
                message_name: message_name.to_string(),
                owner_service_id,
                payload_yaml,
            }),
        )
    }

    async fn upsert_channel_message_contract(
        &self,
        contract: &ChannelMessageContract,
    ) -> Result<(), RepositoryError> {
        sqlx::query(
            "INSERT INTO channel_message_contracts (channel, message_name, owner_service_id, payload_yaml) \
             VALUES (?, ?, ?, ?) \
             ON CONFLICT (channel, message_name) \
             DO UPDATE SET owner_service_id = excluded.owner_service_id, payload_yaml = excluded.payload_yaml",
        )
        .bind(&contract.channel)
        .bind(&contract.message_name)
        .bind(contract.owner_service_id)
        .bind(&contract.payload_yaml)
        .execute(&self.pool)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(())
    }

    async fn delete_channel_message_contract(
        &self,
        channel: &str,
        message_name: &str,
    ) -> Result<(), RepositoryError> {
        sqlx::query("DELETE FROM channel_message_contracts WHERE channel = ? AND message_name = ?")
            .bind(channel)
            .bind(message_name)
            .execute(&self.pool)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(())
    }

    async fn list_channel_message_contracts(
        &self,
    ) -> Result<Vec<ChannelMessageContract>, RepositoryError> {
        let rows: Vec<(String, String, i64, String)> = sqlx::query_as(
            "SELECT channel, message_name, owner_service_id, payload_yaml FROM channel_message_contracts ORDER BY channel, message_name",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;

        Ok(rows
            .into_iter()
            .map(
                |(channel, message_name, owner_service_id, payload_yaml)| ChannelMessageContract {
                    channel,
                    message_name,
                    owner_service_id,
                    payload_yaml,
                },
            )
            .collect())
    }

    async fn insert_audit_log(
        &self,
        username: &str,
        log: NewAuditLog<'_>,
    ) -> Result<(), RepositoryError> {
        let timestamp = chrono::Utc::now().to_rfc3339();
        sqlx::query(
            "INSERT INTO audit_logs (timestamp, username, action, details, service, version, action_type, diff, stream) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(timestamp)
        .bind(username)
        .bind(log.action)
        .bind(log.details)
        .bind(log.service)
        .bind(log.version)
        .bind(log.action_type)
        .bind(log.diff)
        .bind(log.stream)
        .execute(&self.pool)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;

        Ok(())
    }

    async fn get_audit_logs(
        &self,
        filter: AuditLogFilter,
    ) -> Result<Vec<AuditLogEntry>, RepositoryError> {
        let mut sql = String::from(
            "SELECT id, timestamp, username, action, details, service, version, action_type, diff, stream FROM audit_logs WHERE 1=1",
        );

        if filter.from_date.is_some() {
            sql.push_str(" AND timestamp >= ?");
        }
        if filter.to_date.is_some() {
            sql.push_str(" AND timestamp <= ?");
        }
        if filter.action_type.is_some() {
            sql.push_str(" AND action_type = ?");
        }
        if filter.service_wildcard.is_some() {
            sql.push_str(" AND service LIKE ?");
        }
        if filter.version_wildcard.is_some() {
            sql.push_str(" AND version LIKE ?");
        }
        if filter.stream.is_some() {
            sql.push_str(" AND stream = ?");
        }

        sql.push_str(" ORDER BY id DESC LIMIT ?");

        let mut query = sqlx::query_as::<
            _,
            (
                i64,
                String,
                String,
                String,
                String,
                Option<String>,
                Option<String>,
                Option<String>,
                Option<String>,
                Option<String>,
            ),
        >(&sql);

        if let Some(ref val) = filter.from_date {
            query = query.bind(val);
        }
        if let Some(ref val) = filter.to_date {
            query = query.bind(val);
        }
        if let Some(ref val) = filter.action_type {
            query = query.bind(val);
        }
        if let Some(ref val) = filter.service_wildcard {
            query = query.bind(val);
        }
        if let Some(ref val) = filter.version_wildcard {
            query = query.bind(val);
        }
        if let Some(ref val) = filter.stream {
            query = query.bind(val);
        }
        query = query.bind(filter.limit);

        let rows = query
            .fetch_all(&self.pool)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;

        Ok(rows
            .into_iter()
            .map(
                |(
                    id,
                    timestamp,
                    username,
                    action,
                    details,
                    service,
                    version,
                    action_type,
                    diff,
                    stream,
                )| {
                    AuditLogEntry {
                        id,
                        timestamp,
                        username,
                        action,
                        details,
                        service,
                        version,
                        action_type,
                        diff,
                        stream,
                    }
                },
            )
            .collect())
    }

    async fn get_recent_audit_logs(
        &self,
        limit: u32,
    ) -> Result<Vec<AuditLogEntry>, RepositoryError> {
        let rows: Vec<AuditLogRow> = sqlx::query_as(
            "SELECT id, timestamp, username, action, details, service, version, action_type, diff, stream FROM audit_logs ORDER BY id DESC LIMIT ?"
        )
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;

        Ok(rows
            .into_iter()
            .map(
                |(
                    id,
                    timestamp,
                    username,
                    action,
                    details,
                    service,
                    version,
                    action_type,
                    diff,
                    stream,
                )| {
                    AuditLogEntry {
                        id,
                        timestamp,
                        username,
                        action,
                        details,
                        service,
                        version,
                        action_type,
                        diff,
                        stream,
                    }
                },
            )
            .collect())
    }

    async fn get_user_favorites(
        &self,
        user_id: i64,
        item_type: &str,
    ) -> Result<Vec<String>, RepositoryError> {
        let rows: Vec<(String,)> = sqlx::query_as(
            "SELECT item_name FROM user_favorites WHERE user_id = ? AND item_type = ? ORDER BY item_name"
        )
        .bind(user_id)
        .bind(item_type)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;

        Ok(rows.into_iter().map(|r| r.0).collect())
    }

    async fn add_user_favorite(
        &self,
        user_id: i64,
        item_type: &str,
        item_name: &str,
    ) -> Result<(), RepositoryError> {
        sqlx::query(
            "INSERT OR IGNORE INTO user_favorites (user_id, item_type, item_name) VALUES (?, ?, ?)",
        )
        .bind(user_id)
        .bind(item_type)
        .bind(item_name)
        .execute(&self.pool)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;

        Ok(())
    }

    async fn remove_user_favorite(
        &self,
        user_id: i64,
        item_type: &str,
        item_name: &str,
    ) -> Result<(), RepositoryError> {
        sqlx::query(
            "DELETE FROM user_favorites WHERE user_id = ? AND item_type = ? AND item_name = ?",
        )
        .bind(user_id)
        .bind(item_type)
        .bind(item_name)
        .execute(&self.pool)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::ports::UpsertSpecVersion;

    async fn setup() -> SqliteSpecRepository {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        let repo = SqliteSpecRepository::new(pool);
        repo.run_migrations().await.unwrap();
        repo
    }

    fn endpoint(path: &str, method: &str, yaml: &str) -> EndpointRecord {
        EndpointRecord {
            id: None,
            api_type: ApiType::OpenApi,
            path: path.to_string(),
            normalized_path: crate::openapi::normalize_path(path),
            method: method.to_string(),
            yaml_content: yaml.to_string(),
            deprecated: false,
        }
    }

    async fn provide(
        repo: &SqliteSpecRepository,
        service_id: i64,
        version: &str,
        stability: Stability,
        endpoints: Vec<EndpointRecord>,
        now: &str,
    ) -> i64 {
        repo.upsert_spec_version(UpsertSpecVersion {
            service_id,
            api_type: ApiType::OpenApi,
            version: version.parse().unwrap(),
            stability,
            content: "content",
            content_hash: "sha256:x",
            provided_by: "ci",
            expected_prior_hash: None,
            now_iso: now,
            endpoints,
        })
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn upsert_overwrites_endpoints_wholesale_and_keeps_identity() {
        let repo = setup().await;
        let sid = repo.ensure_service("svc").await.unwrap();
        let id1 = provide(
            &repo,
            sid,
            "1.0.0",
            Stability::Snapshot,
            vec![endpoint("/a", "GET", "a"), endpoint("/b", "GET", "b")],
            "2026-01-01T00:00:00Z",
        )
        .await;
        let id2 = provide(
            &repo,
            sid,
            "1.0.0",
            Stability::Ga,
            vec![endpoint("/a", "GET", "a2")],
            "2026-01-02T00:00:00Z",
        )
        .await;
        assert_eq!(id1, id2, "overwrite/promotion keeps the row identity");

        let meta = repo
            .find_spec_version(sid, ApiType::OpenApi, "1.0.0".parse().unwrap())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(meta.stability, Stability::Ga);
        assert_eq!(meta.created_at, "2026-01-01T00:00:00Z");
        assert_eq!(meta.updated_at, "2026-01-02T00:00:00Z");

        let endpoints = repo.get_endpoints_for_version(id1).await.unwrap();
        assert_eq!(endpoints.len(), 1);
        assert_eq!(endpoints[0].yaml_content, "a2");
    }

    #[tokio::test]
    async fn find_endpoint_matches_lenient_paths() {
        let repo = setup().await;
        let sid = repo.ensure_service("svc").await.unwrap();
        let vid = provide(
            &repo,
            sid,
            "1.0.0",
            Stability::Ga,
            vec![endpoint("/users/{id}", "GET", "u")],
            "2026-01-01T00:00:00Z",
        )
        .await;
        let hit = repo
            .find_endpoint(vid, ApiType::OpenApi, "/users/{userId}", "GET")
            .await
            .unwrap();
        assert_eq!(hit.unwrap().1, "u");
    }

    #[tokio::test]
    async fn expired_snapshot_cleanup_is_use_based_and_spares_ga() {
        let repo = setup().await;
        let sid = repo.ensure_service("svc").await.unwrap();
        let old_snapshot = provide(
            &repo,
            sid,
            "1.0.0",
            Stability::Snapshot,
            vec![],
            "2026-01-01T00:00:00Z",
        )
        .await;
        let required_snapshot = provide(
            &repo,
            sid,
            "1.1.0",
            Stability::Snapshot,
            vec![],
            "2026-01-01T00:00:00Z",
        )
        .await;
        repo.touch_spec_version_required(required_snapshot, "2026-03-01T00:00:00Z")
            .await
            .unwrap();
        let old_ga = provide(
            &repo,
            sid,
            "0.9.0",
            Stability::Ga,
            vec![],
            "2026-01-01T00:00:00Z",
        )
        .await;

        let deleted = repo
            .delete_expired_snapshots("2026-02-01T00:00:00Z")
            .await
            .unwrap();
        assert_eq!(deleted, 1);
        assert!(repo.get_spec_content(old_snapshot).await.unwrap().is_none());
        assert!(
            repo.get_spec_content(required_snapshot)
                .await
                .unwrap()
                .is_some(),
            "a required snapshot is not unused"
        );
        assert!(repo.get_spec_content(old_ga).await.unwrap().is_some());
    }

    #[tokio::test]
    async fn dependents_and_report_reflect_recorded_pins() {
        let repo = setup().await;
        let sid = repo.ensure_service("svc").await.unwrap();
        let vid = provide(
            &repo,
            sid,
            "1.0.0",
            Stability::Ga,
            vec![endpoint("/a", "GET", "a"), endpoint("/b", "GET", "b")],
            "2026-01-01T00:00:00Z",
        )
        .await;
        let cid = repo.ensure_client("consumer-x").await.unwrap();
        repo.record_dependency(crate::domain::ports::RecordDependencyParams {
            client_id: cid,
            spec_version_id: vid,
            api_type: ApiType::OpenApi,
            path: "/a",
            normalized_path: "/a",
            method: "GET",
        })
        .await
        .unwrap();

        let dependents = repo.list_version_dependents(vid).await.unwrap();
        assert_eq!(dependents, vec!["consumer-x"]);

        let report = repo.get_report().await.unwrap();
        assert_eq!(report.dependency_graph.len(), 1);
        assert_eq!(report.dependency_graph[0].client, "consumer-x");
        assert_eq!(report.dependency_graph[0].version.to_string(), "1.0.0");
        assert_eq!(report.unused_endpoints.len(), 1);
        assert_eq!(report.unused_endpoints[0].path, "/b");
        assert!(report.missing_endpoints.is_empty());
    }

    #[tokio::test]
    async fn snapshot_overwrite_dropping_endpoint_yields_missing_in_report() {
        let repo = setup().await;
        let sid = repo.ensure_service("svc").await.unwrap();
        let vid = provide(
            &repo,
            sid,
            "1.0.0",
            Stability::Snapshot,
            vec![endpoint("/a", "GET", "a")],
            "2026-01-01T00:00:00Z",
        )
        .await;
        let cid = repo.ensure_client("consumer-x").await.unwrap();
        repo.record_dependency(crate::domain::ports::RecordDependencyParams {
            client_id: cid,
            spec_version_id: vid,
            api_type: ApiType::OpenApi,
            path: "/a",
            normalized_path: "/a",
            method: "GET",
        })
        .await
        .unwrap();

        // Overwrite drops /a — the dependency remains and reports as missing.
        provide(
            &repo,
            sid,
            "1.0.0",
            Stability::Snapshot,
            vec![endpoint("/c", "GET", "c")],
            "2026-01-02T00:00:00Z",
        )
        .await;

        let report = repo.get_report().await.unwrap();
        assert!(report.dependency_graph.is_empty());
        assert_eq!(report.missing_endpoints.len(), 1);
        assert_eq!(report.missing_endpoints[0].path, "/a");
    }

    #[tokio::test]
    async fn delete_spec_version_cascades_dependencies() {
        let repo = setup().await;
        let sid = repo.ensure_service("svc").await.unwrap();
        let vid = provide(
            &repo,
            sid,
            "1.0.0",
            Stability::Ga,
            vec![endpoint("/a", "GET", "a")],
            "2026-01-01T00:00:00Z",
        )
        .await;
        let cid = repo.ensure_client("consumer-x").await.unwrap();
        repo.record_dependency(crate::domain::ports::RecordDependencyParams {
            client_id: cid,
            spec_version_id: vid,
            api_type: ApiType::OpenApi,
            path: "/a",
            normalized_path: "/a",
            method: "GET",
        })
        .await
        .unwrap();

        assert!(repo.delete_spec_version(vid).await.unwrap());
        assert!(!repo.delete_spec_version(vid).await.unwrap());
        let report = repo.get_report().await.unwrap();
        assert!(report.dependency_graph.is_empty());
        assert!(report.missing_endpoints.is_empty());
    }

    #[tokio::test]
    async fn channel_message_contracts_roundtrip_globally() {
        let repo = setup().await;
        let sid = repo.ensure_service("svc").await.unwrap();
        let contract = ChannelMessageContract {
            channel: "orders.created".to_string(),
            message_name: "OrderCreated".to_string(),
            owner_service_id: sid,
            payload_yaml: "type: object".to_string(),
        };
        repo.upsert_channel_message_contract(&contract)
            .await
            .unwrap();
        let got = repo
            .get_channel_message_contract("orders.created", "OrderCreated")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(got, contract);
        assert_eq!(
            repo.list_channel_message_contracts().await.unwrap().len(),
            1
        );
        repo.delete_channel_message_contract("orders.created", "OrderCreated")
            .await
            .unwrap();
        assert!(
            repo.get_channel_message_contract("orders.created", "OrderCreated")
                .await
                .unwrap()
                .is_none()
        );
    }

    /// The last line of defense against two racing writers: once a GA row is
    /// stored, the upsert must refuse to touch it.
    #[tokio::test]
    async fn upsert_refuses_to_overwrite_a_stored_ga_row() {
        let repo = setup().await;
        let sid = repo.ensure_service("svc").await.unwrap();
        provide(
            &repo,
            sid,
            "1.0.0",
            Stability::Ga,
            vec![endpoint("/a", "GET", "a")],
            "2026-01-01T00:00:00Z",
        )
        .await;

        let err = repo
            .upsert_spec_version(UpsertSpecVersion {
                service_id: sid,
                api_type: ApiType::OpenApi,
                version: "1.0.0".parse().unwrap(),
                stability: Stability::Snapshot,
                content: "other",
                content_hash: "sha256:y",
                provided_by: "racer",
                expected_prior_hash: None,
                now_iso: "2026-01-02T00:00:00Z",
                endpoints: vec![endpoint("/b", "GET", "b")],
            })
            .await
            .unwrap_err();
        assert!(matches!(err, RepositoryError::Conflict));

        let meta = repo
            .find_spec_version(sid, ApiType::OpenApi, "1.0.0".parse().unwrap())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(meta.stability, Stability::Ga);
        assert_eq!(meta.content_hash, "sha256:x");
        let endpoints = repo.get_endpoints_for_version(meta.id).await.unwrap();
        assert_eq!(endpoints.len(), 1);
        assert_eq!(endpoints[0].path, "/a", "the GA endpoint set is untouched");
    }

    /// The compare-and-set arm of the upsert guard: a stale expected hash
    /// means the row moved since the caller read it.
    #[tokio::test]
    async fn upsert_with_a_stale_expected_hash_conflicts() {
        let repo = setup().await;
        let sid = repo.ensure_service("svc").await.unwrap();
        provide(
            &repo,
            sid,
            "1.0.0",
            Stability::Snapshot,
            vec![endpoint("/a", "GET", "a")],
            "2026-01-01T00:00:00Z",
        )
        .await;

        let err = repo
            .upsert_spec_version(UpsertSpecVersion {
                service_id: sid,
                api_type: ApiType::OpenApi,
                version: "1.0.0".parse().unwrap(),
                stability: Stability::Ga,
                content: "content",
                content_hash: "sha256:x",
                provided_by: "releaser",
                expected_prior_hash: Some("sha256:stale"),
                now_iso: "2026-01-02T00:00:00Z",
                endpoints: vec![endpoint("/a", "GET", "a")],
            })
            .await
            .unwrap_err();
        assert!(matches!(err, RepositoryError::Conflict));

        let meta = repo
            .find_spec_version(sid, ApiType::OpenApi, "1.0.0".parse().unwrap())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(meta.stability, Stability::Snapshot, "nothing was written");

        // The matching hash goes through.
        repo.upsert_spec_version(UpsertSpecVersion {
            service_id: sid,
            api_type: ApiType::OpenApi,
            version: "1.0.0".parse().unwrap(),
            stability: Stability::Ga,
            content: "content",
            content_hash: "sha256:x",
            provided_by: "releaser",
            expected_prior_hash: Some(&meta.content_hash),
            now_iso: "2026-01-02T00:00:00Z",
            endpoints: vec![endpoint("/a", "GET", "a")],
        })
        .await
        .expect("matching hash lands");
    }

    /// A CAS caller asserted a prior row exists: if it vanished, the upsert
    /// must refuse rather than resurrect the version as a fresh GA.
    #[tokio::test]
    async fn upsert_with_expected_hash_refuses_when_the_row_vanished() {
        let repo = setup().await;
        let sid = repo.ensure_service("svc").await.unwrap();

        let err = repo
            .upsert_spec_version(UpsertSpecVersion {
                service_id: sid,
                api_type: ApiType::OpenApi,
                version: "1.0.0".parse().unwrap(),
                stability: Stability::Ga,
                content: "content",
                content_hash: "sha256:x",
                provided_by: "releaser",
                expected_prior_hash: Some("sha256:x"),
                now_iso: "2026-01-02T00:00:00Z",
                endpoints: vec![endpoint("/a", "GET", "a")],
            })
            .await
            .unwrap_err();
        assert!(matches!(err, RepositoryError::Conflict));
        assert!(
            repo.find_spec_version(sid, ApiType::OpenApi, "1.0.0".parse().unwrap())
                .await
                .unwrap()
                .is_none(),
            "nothing was resurrected"
        );
    }

    /// AsyncAPI channels are stored verbatim, so lookups must not blank their
    /// `{param}` segments the way OpenAPI path matching does.
    #[tokio::test]
    async fn asyncapi_channel_lookups_match_verbatim() {
        let repo = setup().await;
        let sid = repo.ensure_service("svc").await.unwrap();
        let channel = EndpointRecord {
            id: None,
            api_type: ApiType::AsyncApi,
            path: "user/{id}/events".to_string(),
            normalized_path: "user/{id}/events".to_string(),
            method: "PUB".to_string(),
            yaml_content: "channel: user/{id}/events".to_string(),
            deprecated: false,
        };
        let id = repo
            .upsert_spec_version(UpsertSpecVersion {
                service_id: sid,
                api_type: ApiType::AsyncApi,
                version: "1.0.0".parse().unwrap(),
                stability: Stability::Snapshot,
                content: "content",
                content_hash: "sha256:x",
                provided_by: "ci",
                expected_prior_hash: None,
                now_iso: "2026-01-01T00:00:00Z",
                endpoints: vec![channel],
            })
            .await
            .unwrap();

        let found = repo
            .find_endpoint(id, ApiType::AsyncApi, "user/{id}/events", "PUB")
            .await
            .unwrap();
        assert!(
            found.is_some(),
            "a parameterized channel must resolve exactly as declared"
        );

        let bulk = repo
            .find_endpoints_bulk(
                id,
                ApiType::AsyncApi,
                &[("user/{id}/events".to_string(), "PUB".to_string())],
            )
            .await
            .unwrap();
        assert_eq!(bulk.len(), 1);
    }

    /// Two asked-for paths that normalize to the same stored endpoint must
    /// both resolve — the first lookup must not consume the answer.
    #[tokio::test]
    async fn bulk_lookup_resolves_two_paths_normalizing_alike() {
        let repo = setup().await;
        let sid = repo.ensure_service("svc").await.unwrap();
        let id = provide(
            &repo,
            sid,
            "1.0.0",
            Stability::Snapshot,
            vec![endpoint("/users/{id}", "GET", "u")],
            "2026-01-01T00:00:00Z",
        )
        .await;

        let asked = vec![
            ("/users/{id}".to_string(), "GET".to_string()),
            ("/users/{uid}".to_string(), "GET".to_string()),
        ];
        let bulk = repo
            .find_endpoints_bulk(id, ApiType::OpenApi, &asked)
            .await
            .unwrap();
        assert_eq!(
            bulk.len(),
            2,
            "both spellings match the one stored endpoint"
        );
        assert!(bulk.contains_key(&asked[0]));
        assert!(bulk.contains_key(&asked[1]));
    }

    /// The purge spares *effective* administrators: direct role, admin via
    /// group, and the configuration-held root usernames.
    #[tokio::test]
    async fn nuke_users_spares_group_admins_and_root_usernames() {
        let repo = setup().await;
        let direct = repo.create_user("direct", "h", true).await.unwrap();
        repo.grant_user_role(direct.id, "admin").await.unwrap();
        let via_group = repo.create_user("via-group", "h", true).await.unwrap();
        let group = repo
            .create_group("ops", crate::domain::models::GroupSource::Native)
            .await
            .unwrap();
        repo.set_group_roles(group.id, &["admin".to_string()])
            .await
            .unwrap();
        repo.add_group_member(group.id, via_group.id).await.unwrap();
        repo.create_user("ops-root", "h", true).await.unwrap();
        repo.create_user("doomed", "h", true).await.unwrap();

        let deleted = repo
            .delete_all_non_admin_users(&["ops-root".to_string()])
            .await
            .unwrap();
        assert_eq!(deleted, 1);
        assert!(repo.find_user("doomed").await.unwrap().is_none());
        assert!(repo.find_user("direct").await.unwrap().is_some());
        assert!(
            repo.find_user("via-group").await.unwrap().is_some(),
            "admin held through a group counts"
        );
        assert!(
            repo.find_user("ops-root").await.unwrap().is_some(),
            "root operators hold power through configuration, not stored roles"
        );
    }
}
