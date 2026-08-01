use sqlx::{Row, SqlitePool};
use std::collections::HashMap;
use std::str::FromStr;
use tracing::instrument;

type EndpointRow = (i64, String, String, String, String, String, bool, bool);

/// Row shape for `list_producers_detailed` (name, fallback_branch, branches,
/// icon, domain); branches are `GROUP_CONCAT`ed into a single string on SQLite.
type ServiceSummaryRow = (
    String,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
);

/// Row shape for endpoint-version queries joining metadata, service, branch,
/// and endpoint columns.
type EndpointVersionRow = (
    i64,
    i64,
    i32,
    String,
    Option<String>,
    String,
    Option<String>,
    Option<String>,
    String,
    String,
    String,
    String,
    String,
);

/// Row shape for audit-log queries (id, timestamp, username, action, details,
/// service, branch, action_type, diff).
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
);

/// Hashes a session token with SHA-256 so that only the hash is stored at rest.
/// The raw token is returned to the client; lookups hash the incoming token.
fn hash_session_token(token: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(token.as_bytes());
    hex::encode(hasher.finalize())
}

use crate::domain::models::*;
use crate::domain::ports::{
    EndpointMap, NewAuditLog, RecordDependencyParams, RepositoryError, SpecRepository,
    UpdateEndpointParams,
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

        // Backfill normalized_path
        self.backfill_normalized_paths().await.map_err(|e| {
            tracing::error!("Failed to backfill normalized paths: {}", e);
            sqlx::migrate::MigrateError::Execute(sqlx::Error::Configuration(
                format!("Backfill failed: {}", e).into(),
            ))
        })?;

        Ok(())
    }

    async fn backfill_normalized_paths(&self) -> Result<(), RepositoryError> {
        // Endpoints backfill
        let rows: Vec<(i64, String)> = sqlx::query_as(
            "SELECT id, path FROM endpoints WHERE normalized_path = '' OR normalized_path IS NULL",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;

        if !rows.is_empty() {
            tracing::info!("Backfilling {} endpoint normalized paths...", rows.len());
            for (id, path) in rows {
                let normalized = crate::openapi::normalize_path(&path);
                sqlx::query("UPDATE endpoints SET normalized_path = ? WHERE id = ?")
                    .bind(normalized)
                    .bind(id)
                    .execute(&self.pool)
                    .await
                    .map_err(|e| RepositoryError::Internal(e.to_string()))?;
            }
        }

        // Dependencies backfill
        let rows: Vec<(i64, String)> = sqlx::query_as("SELECT id, requested_path FROM dependencies WHERE requested_normalized_path = '' OR requested_normalized_path IS NULL")
            .fetch_all(&self.pool)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;

        if !rows.is_empty() {
            tracing::info!("Backfilling {} dependency normalized paths...", rows.len());
            for (id, path) in rows {
                let normalized = crate::openapi::normalize_path(&path);
                sqlx::query("UPDATE dependencies SET requested_normalized_path = ? WHERE id = ?")
                    .bind(normalized)
                    .bind(id)
                    .execute(&self.pool)
                    .await
                    .map_err(|e| RepositoryError::Internal(e.to_string()))?;
            }
        }

        Ok(())
    }
}

/// A joined `pending_specs` row: id, Producer name, branch, API type, content,
/// reason, submitter, created-at.
type PendingSpecRow = (i64, String, String, String, String, String, String, String);

/// Build a [`PendingSpec`] from a joined row.
///
/// Returns `None` for an unparseable API type rather than guessing: a held spec
/// whose type is unknown cannot be replayed, so surfacing it would offer the
/// reviewer a button that could not work.
fn pending_from_row(row: PendingSpecRow) -> Option<PendingSpec> {
    let (id, producer, branch, api_type, content, reason, submitted_by, created_at) = row;
    Some(PendingSpec {
        id,
        producer,
        branch,
        api_type: api_type.parse().ok()?,
        content,
        reason,
        submitted_by,
        created_at,
    })
}

impl SpecRepository for SqliteSpecRepository {
    async fn ping(&self) -> Result<(), RepositoryError> {
        sqlx::query("SELECT 1")
            .execute(&self.pool)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(())
    }

    async fn get_spec_version(
        &self,
        service_id: i64,
        branch_id: i64,
    ) -> Result<Option<(SemVer, String)>, RepositoryError> {
        let row: Option<(i32, i32, i32, String)> = sqlx::query_as("SELECT major, minor, patch, content_hash FROM service_spec_versions WHERE service_id = ? AND branch_id = ?")
            .bind(service_id)
            .bind(branch_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;

        Ok(row.map(|(ma, mi, pa, h)| (SemVer::new(ma as u32, mi as u32, pa as u32), h)))
    }

    #[instrument(skip_all)]
    async fn increment_spec_version(
        &self,
        service_id: i64,
        branch_id: i64,
        content_hash: &str,
        impact: Impact,
    ) -> Result<SemVer, RepositoryError> {
        let current = self.get_spec_version(service_id, branch_id).await?;
        let next = match current {
            Some((v, _)) => v.increment(impact),
            None => SemVer::initial(),
        };

        sqlx::query("
            INSERT INTO service_spec_versions (service_id, branch_id, major, minor, patch, version, content_hash, updated_at)
            VALUES (?, ?, ?, ?, ?, 1, ?, CURRENT_TIMESTAMP)
            ON CONFLICT(service_id, branch_id) DO UPDATE SET
                major = excluded.major,
                minor = excluded.minor,
                patch = excluded.patch,
                version = service_spec_versions.version + 1,
                content_hash = excluded.content_hash,
                updated_at = excluded.updated_at
        ")
        .bind(service_id)
        .bind(branch_id)
        .bind(next.major as i32)
        .bind(next.minor as i32)
        .bind(next.patch as i32)
        .bind(content_hash)
        .execute(&self.pool)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;

        Ok(next)
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
    async fn find_branch(
        &self,
        service_id: i64,
        branch_name: &str,
    ) -> Result<Option<i64>, RepositoryError> {
        let row: Option<(i64,)> =
            sqlx::query_as("SELECT id FROM branches WHERE service_id = ? AND name = ?")
                .bind(service_id)
                .bind(branch_name)
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

    async fn ensure_branch(
        &self,
        service_id: i64,
        branch_name: &str,
    ) -> Result<i64, RepositoryError> {
        let now = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();
        // Only create the branch here; do NOT bump `updated_at` on existing branches.
        // `ensure_branch` is called on read paths too (viewing history, listing
        // endpoints), so bumping here would make `updated_at` mean "last touched"
        // instead of "last published". The publish write-path (`apply_spec_changes`)
        // is responsible for advancing `updated_at`.
        sqlx::query(
            "INSERT OR IGNORE INTO branches (service_id, name, updated_at) VALUES (?, ?, ?)",
        )
        .bind(service_id)
        .bind(branch_name)
        .bind(&now)
        .execute(&self.pool)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;

        let row: (i64,) =
            sqlx::query_as("SELECT id FROM branches WHERE service_id = ? AND name = ?")
                .bind(service_id)
                .bind(branch_name)
                .fetch_one(&self.pool)
                .await
                .map_err(|e| RepositoryError::Internal(e.to_string()))?;

        Ok(row.0)
    }

    async fn get_endpoints_for_branch(
        &self,
        branch_id: i64,
    ) -> Result<Vec<EndpointRecord>, RepositoryError> {
        let rows: Vec<EndpointRow> = sqlx::query_as(
            "SELECT e.id, e.api_type, e.path, e.normalized_path, e.method, e.yaml_content, \
             e.deprecated, e.external \
             FROM endpoints e \
             WHERE e.branch_id = ? AND e.deleted = FALSE",
        )
        .bind(branch_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;

        Ok(rows
            .into_iter()
            .map(
                |(
                    id,
                    api_type,
                    path,
                    normalized_path,
                    method,
                    yaml_content,
                    deprecated,
                    external,
                )| {
                    EndpointRecord {
                        id: Some(id),
                        api_type: ApiType::from_str(&api_type).unwrap_or_default(),
                        path,
                        normalized_path,
                        method,
                        yaml_content,
                        deprecated,
                        external,
                    }
                },
            )
            .collect())
    }

    async fn insert_endpoint(
        &self,
        branch_id: i64,
        endpoint: &EndpointRecord,
    ) -> Result<(), RepositoryError> {
        sqlx::query("INSERT INTO endpoints (branch_id, api_type, path, normalized_path, method, yaml_content, deprecated, external) VALUES (?, ?, ?, ?, ?, ?, ?, ?)")
            .bind(branch_id)
            .bind(endpoint.api_type.as_str())
            .bind(&endpoint.path)
            .bind(&endpoint.normalized_path)
            .bind(&endpoint.method)
            .bind(&endpoint.yaml_content)
            .bind(endpoint.deprecated)
            .bind(endpoint.external)
            .execute(&self.pool)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;

        Ok(())
    }

    async fn reset_branch_history(
        &self,
        service_name: &str,
        branch_name: &str,
    ) -> Result<bool, RepositoryError> {
        let service_id = self
            .find_service(service_name)
            .await?
            .ok_or(RepositoryError::NotFound)?;
        let branch_id = self
            .find_branch(service_id, branch_name)
            .await?
            .ok_or(RepositoryError::NotFound)?;

        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;

        // 1. Delete all versions except the latest for each endpoint in the branch
        sqlx::query(
            r#"
            DELETE FROM endpoint_versions
            WHERE id IN (
                SELECT ev.id
                FROM endpoint_versions ev
                JOIN endpoints e ON ev.endpoint_id = e.id
                WHERE e.branch_id = ?
                AND ev.version < (
                    SELECT MAX(version)
                    FROM endpoint_versions
                    WHERE endpoint_id = ev.endpoint_id
                )
            )
        "#,
        )
        .bind(branch_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;

        // 2. Renumber the remaining version to 1 and clear diff
        sqlx::query(
            r#"
            UPDATE endpoint_versions
            SET version = 1, diff_from_previous = NULL
            WHERE endpoint_id IN (
                SELECT id FROM endpoints WHERE branch_id = ?
            )
        "#,
        )
        .bind(branch_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;

        // 3. Reset branch-level version counter
        sqlx::query(
            "UPDATE service_spec_versions SET version = 1, major = 1, minor = 0, patch = 0 WHERE service_id = ? AND branch_id = ?",
        )
        .bind(service_id)
        .bind(branch_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;

        tx.commit()
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;

        Ok(true)
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
        service_id: i64,
        branch_name: &str,
        api_type: ApiType,
        path: &str,
        method: &str,
    ) -> Result<Option<(i64, String, bool, bool)>, RepositoryError> {
        let normalized_path = crate::openapi::normalize_path(path);
        let row: Option<(i64, String, bool, bool)> = sqlx::query_as(
            r#"
            SELECT e.id, e.yaml_content, e.deprecated, e.external
            FROM endpoints e
            JOIN branches b ON e.branch_id = b.id
            WHERE b.service_id = ? AND b.name = ? AND e.api_type = ? AND e.normalized_path = ? AND e.method = ? AND e.deleted = FALSE
            "#,
        )
        .bind(service_id)
        .bind(branch_name)
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
        service_id: i64,
        branch_name: &str,
        api_type: ApiType,
        endpoints: &[(String, String)],
    ) -> Result<EndpointMap, RepositoryError> {
        let mut result = EndpointMap::new();
        if endpoints.is_empty() {
            return Ok(result);
        }

        let mut query_builder = sqlx::QueryBuilder::new(
            r#"
            SELECT e.id, e.path, e.method, e.yaml_content, e.deprecated, e.external
            FROM endpoints e
            JOIN branches b ON e.branch_id = b.id
            WHERE b.service_id = "#,
        );
        query_builder.push_bind(service_id);
        query_builder.push(" AND b.name = ");
        query_builder.push_bind(branch_name);
        query_builder.push(" AND e.api_type = ");
        query_builder.push_bind(api_type.as_str());
        query_builder.push(" AND e.deleted = FALSE AND (");

        for (i, (path, method)) in endpoints.iter().enumerate() {
            if i > 0 {
                query_builder.push(" OR ");
            }
            query_builder.push("(e.normalized_path = ");
            query_builder.push_bind(crate::openapi::normalize_path(path));
            query_builder.push(" AND e.method = ");
            query_builder.push_bind(method);
            query_builder.push(")");
        }
        query_builder.push(")");

        let rows: Vec<(i64, String, String, String, bool, bool)> = query_builder
            .build_query_as()
            .fetch_all(&self.pool)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;

        for (id, path, method, yaml, deprecated, external) in rows {
            result.insert((path, method), (id, yaml, deprecated, external));
        }

        Ok(result)
    }

    async fn record_dependency(
        &self,
        params: RecordDependencyParams<'_>,
    ) -> Result<(), RepositoryError> {
        let now = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();
        let normalized_path = crate::openapi::normalize_path(params.path);

        if params.endpoint_id.is_some() {
            // Resolved endpoint: use the table-level UNIQUE constraint
            sqlx::query(
                r#"
                INSERT INTO dependencies 
                (client_id, endpoint_id, api_type, requested_service_id, requested_branch_name, requested_path, requested_normalized_path, requested_method, last_seen_at)
                VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
                ON CONFLICT(client_id, endpoint_id, requested_service_id, requested_branch_name, api_type, requested_path, requested_method)
                DO UPDATE SET last_seen_at = excluded.last_seen_at
                "#,
            )
            .bind(params.client_id)
            .bind(params.endpoint_id)
            .bind(params.api_type.as_str())
            .bind(params.service_id)
            .bind(params.branch_name)
            .bind(params.path)
            .bind(&normalized_path)
            .bind(params.method)
            .bind(&now)
            .execute(&self.pool)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        } else {
            // Missing endpoint (NULL endpoint_id): use the partial unique index
            sqlx::query(
                r#"
                INSERT INTO dependencies 
                (client_id, endpoint_id, api_type, requested_service_id, requested_branch_name, requested_path, requested_normalized_path, requested_method, last_seen_at)
                VALUES (?, NULL, ?, ?, ?, ?, ?, ?, ?)
                ON CONFLICT(client_id, requested_service_id, requested_branch_name, api_type, requested_path, requested_method)
                WHERE endpoint_id IS NULL
                DO UPDATE SET last_seen_at = excluded.last_seen_at
                "#,
            )
            .bind(params.client_id)
            .bind(params.api_type.as_str())
            .bind(params.service_id)
            .bind(params.branch_name)
            .bind(params.path)
            .bind(&normalized_path)
            .bind(params.method)
            .bind(&now)
            .execute(&self.pool)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        }

        Ok(())
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

        // Split into resolved (has endpoint_id) and missing (NULL endpoint_id)
        let (resolved, missing): (Vec<_>, Vec<_>) =
            params.into_iter().partition(|p| p.endpoint_id.is_some());

        if !resolved.is_empty() {
            let mut query_builder = sqlx::QueryBuilder::new(
                "INSERT INTO dependencies (client_id, endpoint_id, api_type, requested_service_id, requested_branch_name, requested_path, requested_normalized_path, requested_method, last_seen_at) ",
            );

            query_builder.push_values(resolved, |mut b, p| {
                let normalized_path = crate::openapi::normalize_path(p.path);
                b.push_bind(p.client_id)
                    .push_bind(p.endpoint_id)
                    .push_bind(p.api_type.as_str())
                    .push_bind(p.service_id)
                    .push_bind(p.branch_name)
                    .push_bind(p.path)
                    .push_bind(normalized_path)
                    .push_bind(p.method)
                    .push_bind(&now);
            });

            query_builder.push(" ON CONFLICT(client_id, endpoint_id, requested_service_id, requested_branch_name, api_type, requested_path, requested_method) DO UPDATE SET last_seen_at = excluded.last_seen_at");

            query_builder
                .build()
                .execute(&self.pool)
                .await
                .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        }

        // For missing endpoint deps, use the partial unique index
        for p in missing {
            let normalized_path = crate::openapi::normalize_path(p.path);
            sqlx::query(
                r#"
                INSERT INTO dependencies 
                (client_id, endpoint_id, api_type, requested_service_id, requested_branch_name, requested_path, requested_normalized_path, requested_method, last_seen_at)
                VALUES (?, NULL, ?, ?, ?, ?, ?, ?, ?)
                ON CONFLICT(client_id, requested_service_id, requested_branch_name, api_type, requested_path, requested_method)
                WHERE endpoint_id IS NULL
                DO UPDATE SET last_seen_at = excluded.last_seen_at
                "#,
            )
            .bind(p.client_id)
            .bind(p.api_type.as_str())
            .bind(p.service_id)
            .bind(p.branch_name)
            .bind(p.path)
            .bind(normalized_path)
            .bind(p.method)
            .bind(&now)
            .execute(&self.pool)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        }

        Ok(())
    }

    async fn is_branch_protected(&self, branch_name: &str) -> Result<bool, RepositoryError> {
        // Matching runs in Rust (`branch_pattern`) rather than in SQL so both
        // backends agree on what a pattern means: `*`/`?` are wildcards and
        // everything else — notably `_` — is literal. A wildcard pattern
        // (e.g. `release/*`) must protect the branches it covers everywhere,
        // otherwise a matching branch would be exempt from stale cleanup yet
        // reported "not protected" for breaking-change enforcement, version
        // recording, and fallback resolution.
        let patterns = self.list_protected_branches().await?;
        Ok(crate::domain::branch_pattern::branch_matches_any(
            branch_name,
            &patterns,
        ))
    }

    async fn add_protected_branch(&self, pattern: &str) -> Result<(), RepositoryError> {
        sqlx::query("INSERT OR IGNORE INTO protected_branches (pattern) VALUES (?)")
            .bind(pattern)
            .execute(&self.pool)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(())
    }

    async fn remove_protected_branch(&self, pattern: &str) -> Result<bool, RepositoryError> {
        let result = sqlx::query("DELETE FROM protected_branches WHERE pattern = ?")
            .bind(pattern)
            .execute(&self.pool)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(result.rows_affected() > 0)
    }

    async fn list_protected_branches(&self) -> Result<Vec<String>, RepositoryError> {
        let rows: Vec<(String,)> =
            sqlx::query_as("SELECT pattern FROM protected_branches ORDER BY pattern")
                .fetch_all(&self.pool)
                .await
                .map_err(|e| RepositoryError::Internal(e.to_string()))?;

        Ok(rows.into_iter().map(|r| r.0).collect())
    }

    async fn update_endpoint(
        &self,
        params: UpdateEndpointParams<'_>,
    ) -> Result<(), RepositoryError> {
        sqlx::query("UPDATE endpoints SET yaml_content = ?, deprecated = ?, external = ?, deleted = FALSE WHERE branch_id = ? AND api_type = ? AND path = ? AND method = ?")
            .bind(params.yaml_content)
            .bind(params.deprecated)
            .bind(params.external)
            .bind(params.branch_id)
            .bind(params.api_type.as_str())
            .bind(params.path)
            .bind(params.method)
            .execute(&self.pool)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        // An admin manual edit is branch activity — advance the branch's
        // last-published time so it does not look stale (and is not culled).
        let now = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();
        sqlx::query("UPDATE branches SET updated_at = ? WHERE id = ?")
            .bind(&now)
            .bind(params.branch_id)
            .execute(&self.pool)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(())
    }

    async fn soft_delete_endpoint(
        &self,
        branch_id: i64,
        api_type: ApiType,
        path: &str,
        method: &str,
    ) -> Result<(), RepositoryError> {
        sqlx::query("UPDATE endpoints SET deleted = TRUE WHERE branch_id = ? AND api_type = ? AND path = ? AND method = ?")
            .bind(branch_id)
            .bind(api_type.as_str())
            .bind(path)
            .bind(method)
            .execute(&self.pool)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(())
    }

    async fn hard_delete_endpoint(
        &self,
        branch_id: i64,
        api_type: ApiType,
        path: &str,
        method: &str,
    ) -> Result<(), RepositoryError> {
        sqlx::query("DELETE FROM endpoints WHERE branch_id = ? AND api_type = ? AND path = ? AND method = ?")
            .bind(branch_id)
            .bind(api_type.as_str())
            .bind(path)
            .bind(method)
            .execute(&self.pool)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(())
    }

    async fn is_endpoint_deleted(
        &self,
        branch_id: i64,
        api_type: ApiType,
        path: &str,
        method: &str,
    ) -> Result<bool, RepositoryError> {
        let row: (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM endpoints WHERE branch_id = ? AND api_type = ? AND path = ? AND method = ? AND deleted = TRUE"
        )
        .bind(branch_id)
        .bind(api_type.as_str())
        .bind(path)
        .bind(method)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(row.0 > 0)
    }

    #[instrument(skip_all)]
    async fn get_report(&self, branch: &str) -> Result<DependencyReport, RepositoryError> {
        let mut conn = self
            .pool
            .acquire()
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;

        // Dependency graph
        let dependency_rows: Vec<(String, String, String, String, String, bool)> = sqlx::query_as(
            r#"
            SELECT c.name, d.api_type, s.name, d.requested_path, d.requested_method, COALESCE(e.deprecated, FALSE)
            FROM dependencies d
            JOIN clients c ON d.client_id = c.id
            JOIN services s ON d.requested_service_id = s.id
            LEFT JOIN branches b ON b.service_id = s.id AND b.name = d.requested_branch_name
            LEFT JOIN endpoints e ON (e.id = d.endpoint_id OR (e.branch_id = b.id AND e.api_type = d.api_type AND e.normalized_path = d.requested_normalized_path AND e.method = d.requested_method))
            WHERE d.requested_branch_name = ?
            AND (e.id IS NULL OR e.deleted = FALSE)
            "#,
        )
        .bind(branch)
        .fetch_all(&mut *conn)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;

        let dependency_graph = dependency_rows
            .into_iter()
            .map(
                |(client, api_type, service, path, method, deprecated)| DependencyInfo {
                    api_type: ApiType::from_str(&api_type).unwrap_or_default(),
                    client,
                    service,
                    path,
                    method,
                    deprecated,
                },
            )
            .collect();

        // Unused endpoints
        let unused_rows: Vec<(String, String, String, String, bool)> = sqlx::query_as(
            r#"
            SELECT s.name, e.api_type, e.path, e.method, e.deprecated
            FROM endpoints e
            JOIN branches b ON e.branch_id = b.id
            JOIN services s ON b.service_id = s.id
            WHERE b.name = ? AND e.deleted = FALSE AND e.id NOT IN (
                SELECT endpoint_id FROM dependencies 
                WHERE requested_branch_name = ? AND endpoint_id IS NOT NULL
            )
            "#,
        )
        .bind(branch)
        .bind(branch)
        .fetch_all(&mut *conn)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;

        let unused_endpoints = unused_rows
            .into_iter()
            .map(
                |(service, api_type, path, method, deprecated)| EndpointInfo {
                    api_type: ApiType::from_str(&api_type).unwrap_or_default(),
                    service,
                    path,
                    method,
                    deprecated,
                },
            )
            .collect();

        // Missing endpoints
        let missing_rows: Vec<(String, String, String, String, String)> = sqlx::query_as(
            r#"
            SELECT c.name, d.api_type, s.name, d.requested_path, d.requested_method
            FROM dependencies d
            JOIN clients c ON d.client_id = c.id
            JOIN services s ON d.requested_service_id = s.id
            LEFT JOIN branches b ON b.service_id = s.id AND b.name = d.requested_branch_name
            LEFT JOIN endpoints e ON (e.id = d.endpoint_id OR (e.branch_id = b.id AND e.api_type = d.api_type AND e.normalized_path = d.requested_normalized_path AND e.method = d.requested_method))
            WHERE d.requested_branch_name = ? 
            AND e.id IS NULL
            "#,
        )
        .bind(branch)
        .fetch_all(&mut *conn)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;

        let missing_endpoints = missing_rows
            .into_iter()
            .map(
                |(client, api_type, service, path, method)| MissingEndpointInfo {
                    api_type: ApiType::from_str(&api_type).unwrap_or_default(),
                    client,
                    service,
                    path,
                    method,
                },
            )
            .collect();

        Ok(DependencyReport {
            branch: branch.to_string(),
            unused_endpoints,
            missing_endpoints,
            dependency_graph,
            service_tags: HashMap::new(),
        })
    }

    async fn delete_all_services(&self) -> Result<u64, RepositoryError> {
        let r1 = sqlx::query(
            "DELETE FROM endpoint_versions WHERE endpoint_id IN (SELECT id FROM endpoints)",
        )
        .execute(&self.pool)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        sqlx::query("DELETE FROM service_spec_versions")
            .execute(&self.pool)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        sqlx::query("DELETE FROM dependencies")
            .execute(&self.pool)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        sqlx::query("DELETE FROM endpoints")
            .execute(&self.pool)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        sqlx::query("DELETE FROM branches")
            .execute(&self.pool)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        let result = sqlx::query("DELETE FROM services")
            .execute(&self.pool)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        let _ = r1;
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
    async fn delete_all_non_admin_users(&self) -> Result<u64, RepositoryError> {
        // Delete sessions for non-admin users
        sqlx::query(
            "DELETE FROM sessions WHERE user_id IN (SELECT id FROM users WHERE NOT EXISTS (SELECT 1 FROM user_roles r WHERE r.user_id = users.id AND r.role = 'admin'))",
        )
        .execute(&self.pool)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        // Delete API tokens for non-admin users
        sqlx::query(
            "DELETE FROM api_tokens WHERE user_id IN (SELECT id FROM users WHERE NOT EXISTS (SELECT 1 FROM user_roles r WHERE r.user_id = users.id AND r.role = 'admin'))",
        )
        .execute(&self.pool)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        let result = sqlx::query("DELETE FROM users WHERE NOT EXISTS (SELECT 1 FROM user_roles r WHERE r.user_id = users.id AND r.role = 'admin')")
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

        sqlx::query("DELETE FROM endpoint_versions")
            .execute(&mut *tx)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        sqlx::query("DELETE FROM service_spec_versions")
            .execute(&mut *tx)
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
        sqlx::query("DELETE FROM branches")
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

        // Get all branch IDs for this service
        let branch_rows: Vec<(i64,)> =
            sqlx::query_as("SELECT id FROM branches WHERE service_id = ?")
                .bind(service_id)
                .fetch_all(&mut *tx)
                .await
                .map_err(|e| RepositoryError::Internal(e.to_string()))?;

        for (branch_id,) in &branch_rows {
            sqlx::query("DELETE FROM service_spec_versions WHERE branch_id = ?")
                .bind(branch_id)
                .execute(&mut *tx)
                .await
                .map_err(|e| RepositoryError::Internal(e.to_string()))?;

            // Delete dependencies referencing endpoints in this branch
            sqlx::query(
                "DELETE FROM dependencies WHERE endpoint_id IN (SELECT id FROM endpoints WHERE branch_id = ?)"
            )
            .bind(branch_id)
            .execute(&mut *tx)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;

            // Delete endpoints
            sqlx::query("DELETE FROM endpoints WHERE branch_id = ?")
                .bind(branch_id)
                .execute(&mut *tx)
                .await
                .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        }

        // Delete dependencies referencing this service (including those with NULL endpoint_id)
        sqlx::query("DELETE FROM dependencies WHERE requested_service_id = ?")
            .bind(service_id)
            .execute(&mut *tx)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;

        sqlx::query("DELETE FROM service_spec_versions WHERE service_id = ?")
            .bind(service_id)
            .execute(&mut *tx)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;

        // Delete branches
        sqlx::query("DELETE FROM branches WHERE service_id = ?")
            .bind(service_id)
            .execute(&mut *tx)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;

        // Delete service
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

    async fn delete_branch(
        &self,
        service_name: &str,
        branch_name: &str,
    ) -> Result<bool, RepositoryError> {
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;

        let row: Option<(i64, i64)> = sqlx::query_as(
            "SELECT b.id, s.id FROM branches b JOIN services s ON b.service_id = s.id WHERE s.name = ? AND b.name = ?"
        )
        .bind(service_name)
        .bind(branch_name)
        .fetch_optional(&mut *tx)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;

        let (branch_id, service_id) = match row {
            Some(r) => r,
            None => return Ok(false),
        };

        // Delete dependencies referencing endpoints in this branch
        sqlx::query(
            "DELETE FROM dependencies WHERE endpoint_id IN (SELECT id FROM endpoints WHERE branch_id = ?)"
        )
        .bind(branch_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;

        // Delete dependencies referencing this branch by name (including NULL endpoint_id)
        sqlx::query(
            "DELETE FROM dependencies WHERE requested_service_id = ? AND requested_branch_name = ?",
        )
        .bind(service_id)
        .bind(branch_name)
        .execute(&mut *tx)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;

        // Delete spec versions
        sqlx::query("DELETE FROM service_spec_versions WHERE branch_id = ?")
            .bind(branch_id)
            .execute(&mut *tx)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;

        // Delete endpoints
        sqlx::query("DELETE FROM endpoints WHERE branch_id = ?")
            .bind(branch_id)
            .execute(&mut *tx)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;

        // Delete branch
        sqlx::query("DELETE FROM branches WHERE id = ?")
            .bind(branch_id)
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
        let rows: Vec<(String,)> = sqlx::query_as(
            "SELECT DISTINCT s.name FROM services s INNER JOIN branches b ON b.service_id = s.id ORDER BY s.name"
        )
            .fetch_all(&self.pool)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(rows.into_iter().map(|r| r.0).collect())
    }

    async fn list_producers_detailed(&self) -> Result<Vec<ProducerSummary>, RepositoryError> {
        let rows: Vec<ServiceSummaryRow> = sqlx::query_as(
            r#"
            SELECT s.name, s.fallback_branch, GROUP_CONCAT(b.name) as branches, s.icon, s.domain
            FROM services s
            JOIN branches b ON b.service_id = s.id
            GROUP BY s.id
            ORDER BY s.name
            "#,
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;

        Ok(rows
            .into_iter()
            .map(|(name, fallback_branch, branches_str, icon, domain)| {
                let branches = branches_str
                    .map(|s| s.split(',').map(|b| b.to_string()).collect())
                    .unwrap_or_default();
                ProducerSummary {
                    name,
                    fallback_branch,
                    branches,
                    branches_last_published: std::collections::HashMap::new(),
                    branches_expire_at: std::collections::HashMap::new(),
                    branches_endpoint_count: std::collections::HashMap::new(),
                    is_favorite: false,
                    icon,
                    domain,
                }
            })
            .collect())
    }

    async fn set_fallback_branch(
        &self,
        service_name: &str,
        branch: Option<&str>,
    ) -> Result<(), RepositoryError> {
        sqlx::query("UPDATE services SET fallback_branch = ? WHERE name = ?")
            .bind(branch)
            .bind(service_name)
            .execute(&self.pool)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(())
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

    async fn get_fallback_branch(
        &self,
        service_name: &str,
    ) -> Result<Option<String>, RepositoryError> {
        let row: Option<(Option<String>,)> =
            sqlx::query_as("SELECT fallback_branch FROM services WHERE name = ?")
                .bind(service_name)
                .fetch_optional(&self.pool)
                .await
                .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(row.and_then(|r| r.0))
    }

    async fn set_source_protected_branch_if_unset(
        &self,
        branch_id: i64,
        value: &str,
    ) -> Result<bool, RepositoryError> {
        let result = sqlx::query(
            "UPDATE branches SET source_protected_branch = ? WHERE id = ? AND source_protected_branch IS NULL",
        )
        .bind(value)
        .bind(branch_id)
        .execute(&self.pool)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(result.rows_affected() > 0)
    }

    async fn get_source_protected_branch(
        &self,
        branch_id: i64,
    ) -> Result<Option<String>, RepositoryError> {
        let row: Option<(Option<String>,)> =
            sqlx::query_as("SELECT source_protected_branch FROM branches WHERE id = ?")
                .bind(branch_id)
                .fetch_optional(&self.pool)
                .await
                .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(row.and_then(|r| r.0))
    }

    async fn admin_set_source_protected_branch(
        &self,
        branch_id: i64,
        value: Option<&str>,
    ) -> Result<(), RepositoryError> {
        sqlx::query("UPDATE branches SET source_protected_branch = ? WHERE id = ?")
            .bind(value)
            .bind(branch_id)
            .execute(&self.pool)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(())
    }

    async fn list_branches(&self, service_name: &str) -> Result<Vec<String>, RepositoryError> {
        let rows: Vec<(String,)> = sqlx::query_as(
            "SELECT b.name FROM branches b JOIN services s ON b.service_id = s.id WHERE s.name = ? ORDER BY b.name"
        )
        .bind(service_name)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(rows.into_iter().map(|r| r.0).collect())
    }

    async fn list_all_branches(&self) -> Result<Vec<String>, RepositoryError> {
        let rows: Vec<(String,)> =
            sqlx::query_as("SELECT DISTINCT b.name FROM branches b ORDER BY b.name")
                .fetch_all(&self.pool)
                .await
                .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(rows.into_iter().map(|r| r.0).collect())
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

    async fn list_consumer_branches(
        &self,
        client_name: &str,
    ) -> Result<Vec<String>, RepositoryError> {
        let rows: Vec<(String,)> = sqlx::query_as(
            "SELECT DISTINCT d.requested_branch_name \
             FROM dependencies d \
             JOIN clients c ON d.client_id = c.id \
             WHERE c.name = ? \
             ORDER BY d.requested_branch_name",
        )
        .bind(client_name)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(rows.into_iter().map(|r| r.0).collect())
    }

    async fn list_consumer_endpoints(
        &self,
        client_name: &str,
        branch: &str,
    ) -> Result<Vec<ConsumerEndpointInfo>, RepositoryError> {
        type ClientEndpointRow = (
            String,
            String,
            String,
            String,
            String,
            Option<String>,
            bool,
            bool,
        );
        let rows: Vec<ClientEndpointRow> = sqlx::query_as(
            "SELECT d.api_type, s.name, d.requested_branch_name, d.requested_path, d.requested_method, e.yaml_content, \
             COALESCE(e.deprecated, 0), COALESCE(e.external, 0) \
             FROM dependencies d \
             JOIN clients c ON d.client_id = c.id \
             JOIN services s ON d.requested_service_id = s.id \
             LEFT JOIN endpoints e ON d.endpoint_id = e.id \
             WHERE c.name = ? AND d.requested_branch_name = ? \
             ORDER BY s.name, d.requested_path, d.requested_method"
        )
        .bind(client_name)
        .bind(branch)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(rows
            .into_iter()
            .map(
                |(api_type, service, branch, path, method, yaml_content, deprecated, external)| {
                    ConsumerEndpointInfo {
                        api_type: ApiType::from_str(&api_type).unwrap_or_default(),
                        service,
                        branch,
                        path,
                        method,
                        yaml_content,
                        deprecated,
                        external,
                    }
                },
            )
            .collect())
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

    async fn delete_stale_branches(&self, cutoff_iso: &str) -> Result<u64, RepositoryError> {
        // Delete endpoints and dependencies for stale non-protected branches, then the branches themselves.
        // Protection is decided in Rust (`branch_pattern`), the same way
        // `is_branch_protected` does it, so a branch can never be culled here
        // while counting as protected elsewhere.
        let patterns = self.list_protected_branches().await?;
        let candidates: Vec<(i64, String)> = sqlx::query_as(
            r#"
            SELECT b.id, b.name FROM branches b
            JOIN services s ON b.service_id = s.id
            WHERE b.updated_at < ?
            "#,
        )
        .bind(cutoff_iso)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;

        let stale_branch_ids: Vec<(i64,)> = candidates
            .into_iter()
            .filter(|(_, name)| !crate::domain::branch_pattern::branch_matches_any(name, &patterns))
            .map(|(id, _)| (id,))
            .collect();

        if stale_branch_ids.is_empty() {
            return Ok(0);
        }

        let count = stale_branch_ids.len() as u64;
        for (branch_id,) in &stale_branch_ids {
            // Delete dependencies referencing endpoints on this branch
            sqlx::query("DELETE FROM dependencies WHERE endpoint_id IN (SELECT id FROM endpoints WHERE branch_id = ?)")
                .bind(branch_id)
                .execute(&self.pool)
                .await
                .map_err(|e| RepositoryError::Internal(e.to_string()))?;
            // Delete dependencies referencing this branch by name
            sqlx::query(
                r#"DELETE FROM dependencies WHERE requested_branch_name = (
                    SELECT name FROM branches WHERE id = ?
                ) AND requested_service_id = (
                    SELECT service_id FROM branches WHERE id = ?
                )"#,
            )
            .bind(branch_id)
            .bind(branch_id)
            .execute(&self.pool)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
            // Delete endpoints
            sqlx::query("DELETE FROM endpoints WHERE branch_id = ?")
                .bind(branch_id)
                .execute(&self.pool)
                .await
                .map_err(|e| RepositoryError::Internal(e.to_string()))?;
            // Delete branch
            sqlx::query("DELETE FROM branches WHERE id = ?")
                .bind(branch_id)
                .execute(&self.pool)
                .await
                .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        }

        Ok(count)
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

    async fn get_endpoint_id(
        &self,
        branch_id: i64,
        api_type: ApiType,
        path: &str,
        method: &str,
    ) -> Result<Option<i64>, RepositoryError> {
        let normalized_path = crate::openapi::normalize_path(path);
        let row: Option<(i64,)> = sqlx::query_as(
            "SELECT id FROM endpoints WHERE branch_id = ? AND api_type = ? AND normalized_path = ? AND method = ? AND deleted = 0"
        )
        .bind(branch_id)
        .bind(api_type.as_str())
        .bind(normalized_path)
        .bind(method)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(row.map(|(id,)| id))
    }

    async fn insert_endpoint_version(
        &self,
        endpoint_id: i64,
        version: i32,
        yaml_content: &str,
        diff: Option<&str>,
        created_at: &str,
    ) -> Result<(), RepositoryError> {
        sqlx::query(
            "INSERT INTO endpoint_versions (endpoint_id, version, yaml_content, diff_from_previous, created_at) VALUES (?, ?, ?, ?, ?)"
        )
        .bind(endpoint_id)
        .bind(version)
        .bind(yaml_content)
        .bind(diff)
        .bind(created_at)
        .execute(&self.pool)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(())
    }

    async fn get_latest_endpoint_version(&self, endpoint_id: i64) -> Result<i32, RepositoryError> {
        let row: Option<(i32,)> =
            sqlx::query_as("SELECT MAX(version) FROM endpoint_versions WHERE endpoint_id = ?")
                .bind(endpoint_id)
                .fetch_optional(&self.pool)
                .await
                .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(row.map(|(v,)| v).unwrap_or(0))
    }

    async fn get_endpoint_versions(
        &self,
        endpoint_id: i64,
    ) -> Result<Vec<EndpointVersion>, RepositoryError> {
        let rows: Vec<EndpointVersionRow> = sqlx::query_as(
            "SELECT ev.id, ev.endpoint_id, ev.version, ev.yaml_content, ev.diff_from_previous, ev.created_at, 
                   m.username, m.source_branch,
                   s.name as service_name, b.name as branch_name, e.api_type, e.path, e.method
             FROM endpoint_versions ev 
             LEFT JOIN endpoint_version_metadata m ON ev.id = m.endpoint_version_id 
             JOIN endpoints e ON ev.endpoint_id = e.id
             JOIN branches b ON e.branch_id = b.id
             JOIN services s ON b.service_id = s.id
             WHERE ev.endpoint_id = ? 
             ORDER BY ev.version ASC"
        )
        .bind(endpoint_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;

        Ok(rows
            .into_iter()
            .map(
                |(
                    id,
                    endpoint_id,
                    version,
                    yaml_content,
                    diff_from_previous,
                    created_at,
                    username,
                    source_branch,
                    service_name,
                    branch_name,
                    api_type_str,
                    path,
                    method,
                )| {
                    EndpointVersion {
                        id,
                        endpoint_id,
                        version,
                        yaml_content,
                        diff_from_previous,
                        created_at,
                        username,
                        source_branch,
                        service_name: Some(service_name),
                        branch_name: Some(branch_name),
                        api_type: ApiType::from_str(&api_type_str).ok(),
                        path: Some(path),
                        method: Some(method),
                    }
                },
            )
            .collect())
    }

    async fn get_global_endpoint_versions(
        &self,
        limit: u32,
    ) -> Result<Vec<EndpointVersion>, RepositoryError> {
        let rows = sqlx::query(
            "SELECT 
                ev.id, ev.endpoint_id, ev.version, ev.yaml_content, ev.diff_from_previous, ev.created_at, 
                m.username, m.source_branch,
                s.name as service_name, b.name as branch_name, e.api_type, e.path, e.method
             FROM endpoint_versions ev 
             LEFT JOIN endpoint_version_metadata m ON ev.id = m.endpoint_version_id 
             JOIN endpoints e ON ev.endpoint_id = e.id
             JOIN branches b ON e.branch_id = b.id
             JOIN services s ON b.service_id = s.id
             ORDER BY ev.created_at DESC, ev.id DESC
             LIMIT ?"
        )
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;

        use sqlx::Row;
        Ok(rows
            .into_iter()
            .map(|r| EndpointVersion {
                id: r.get(0),
                endpoint_id: r.get(1),
                version: r.get(2),
                yaml_content: r.get(3),
                diff_from_previous: r.get(4),
                created_at: r.get(5),
                username: r.get(6),
                source_branch: r.get(7),
                service_name: Some(r.get(8)),
                branch_name: Some(r.get(9)),
                api_type: ApiType::from_str(r.get::<String, _>(10).as_str()).ok(),
                path: Some(r.get(11)),
                method: Some(r.get(12)),
            })
            .collect())
    }

    #[instrument(skip_all)]
    async fn apply_spec_changes(
        &self,
        branch_id: i64,
        changes: Vec<SpecChange>,
        is_protected: bool,
        username: Option<&str>,
        source_branch: Option<&str>,
    ) -> Result<(), RepositoryError> {
        tracing::debug!(
            "Applying {} spec changes to branch {}",
            changes.len(),
            branch_id
        );
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        let now = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();

        // Advance the branch's `updated_at` on every publish (provide), so it
        // reflects the last-published time. Read paths no longer bump it.
        sqlx::query("UPDATE branches SET updated_at = ? WHERE id = ?")
            .bind(&now)
            .bind(branch_id)
            .execute(&mut *tx)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;

        for change in changes {
            match change {
                SpecChange::Insert {
                    api_type,
                    path,
                    normalized_path,
                    method,
                    yaml_content,
                    deprecated,
                    external,
                } => {
                    tracing::debug!("Inserting {:?} endpoint: {} {}", api_type, method, path);
                    // Revive on conflict: a soft-deleted row (deleted = 1) still occupies the
                    // UNIQUE(branch_id, api_type, path, method) slot, so a plain INSERT would
                    // violate the constraint when a previously removed endpoint is
                    // re-introduced (e.g. on a branch that is no longer protected).
                    sqlx::query("INSERT INTO endpoints (branch_id, api_type, path, normalized_path, method, yaml_content, deleted, deprecated, external) VALUES (?, ?, ?, ?, ?, ?, 0, ?, ?) ON CONFLICT(branch_id, api_type, path, method) DO UPDATE SET deleted = 0, normalized_path = excluded.normalized_path, yaml_content = excluded.yaml_content, deprecated = excluded.deprecated, external = excluded.external")
                        .bind(branch_id)
                        .bind(api_type.as_str())
                        .bind(&path)
                        .bind(&normalized_path)
                        .bind(&method)
                        .bind(&yaml_content)
                        .bind(deprecated)
                        .bind(external)
                        .execute(&mut *tx)
                        .await
                        .map_err(|e| RepositoryError::Internal(e.to_string()))?;

                    if is_protected {
                        let row: (i64,) = sqlx::query_as("SELECT id FROM endpoints WHERE branch_id = ? AND api_type = ? AND path = ? AND method = ? AND deleted = 0")
                            .bind(branch_id)
                            .bind(api_type.as_str())
                            .bind(&path)
                            .bind(&method)
                            .fetch_one(&mut *tx)
                            .await
                            .map_err(|e| RepositoryError::Internal(e.to_string()))?;

                        let insert_res = sqlx::query("INSERT INTO endpoint_versions (endpoint_id, version, yaml_content, diff_from_previous, created_at) VALUES (?, 1, ?, NULL, ?)")
                            .bind(row.0)
                            .bind(&yaml_content)
                            .bind(&now)
                            .execute(&mut *tx)
                            .await
                            .map_err(|e| RepositoryError::Internal(e.to_string()))?;

                        let version_id = insert_res.last_insert_rowid();

                        sqlx::query("INSERT INTO endpoint_version_metadata (endpoint_version_id, username, source_branch) VALUES (?, ?, ?)")
                            .bind(version_id)
                            .bind(username)
                            .bind(source_branch)
                            .execute(&mut *tx)
                            .await
                            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
                    }
                }
                SpecChange::Update {
                    api_type,
                    path,
                    normalized_path,
                    method,
                    yaml_content,
                    deprecated,
                    external,
                } => {
                    tracing::debug!("Updating {:?} endpoint: {} {}", api_type, method, path);
                    if is_protected {
                        let row: (i64, String) = sqlx::query_as("SELECT id, yaml_content FROM endpoints WHERE branch_id = ? AND api_type = ? AND path = ? AND method = ? AND deleted = 0")
                            .bind(branch_id)
                            .bind(api_type.as_str())
                            .bind(&path)
                            .bind(&method)
                            .fetch_one(&mut *tx)
                            .await
                            .map_err(|e| RepositoryError::Internal(e.to_string()))?;

                        let endpoint_id = row.0;
                        let old_yaml = row.1;

                        let version_row: (i32,) = sqlx::query_as("SELECT COALESCE(MAX(version), 0) FROM endpoint_versions WHERE endpoint_id = ?")
                            .bind(endpoint_id)
                            .fetch_one(&mut *tx)
                            .await
                            .map_err(|e| RepositoryError::Internal(e.to_string()))?;

                        let diff = crate::openapi::generate_diff(&old_yaml, &yaml_content);

                        let insert_res = sqlx::query("INSERT INTO endpoint_versions (endpoint_id, version, yaml_content, diff_from_previous, created_at) VALUES (?, ?, ?, ?, ?)")
                            .bind(endpoint_id)
                            .bind(version_row.0 + 1)
                            .bind(&yaml_content)
                            .bind(diff)
                            .bind(&now)
                            .execute(&mut *tx)
                            .await
                            .map_err(|e| RepositoryError::Internal(e.to_string()))?;

                        let version_id = insert_res.last_insert_rowid();

                        sqlx::query("INSERT INTO endpoint_version_metadata (endpoint_version_id, username, source_branch) VALUES (?, ?, ?)")
                            .bind(version_id)
                            .bind(username)
                            .bind(source_branch)
                            .execute(&mut *tx)
                            .await
                            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
                    }

                    sqlx::query("UPDATE endpoints SET yaml_content = ?, normalized_path = ?, deprecated = ?, external = ? WHERE branch_id = ? AND api_type = ? AND path = ? AND method = ?")
                        .bind(&yaml_content)
                        .bind(&normalized_path)
                        .bind(deprecated)
                        .bind(external)
                        .bind(branch_id)
                        .bind(api_type.as_str())
                        .bind(&path)
                        .bind(&method)
                        .execute(&mut *tx)
                        .await
                        .map_err(|e| RepositoryError::Internal(e.to_string()))?;
                }
                SpecChange::Delete {
                    api_type,
                    path,
                    method,
                    soft_delete,
                } => {
                    tracing::debug!(
                        "Deleting {:?} endpoint: {} {} (soft: {})",
                        api_type,
                        method,
                        path,
                        soft_delete
                    );
                    if soft_delete {
                        sqlx::query("UPDATE endpoints SET deleted = 1 WHERE branch_id = ? AND api_type = ? AND path = ? AND method = ?")
                            .bind(branch_id)
                            .bind(api_type.as_str())
                            .bind(&path)
                            .bind(&method)
                            .execute(&mut *tx)
                            .await
                            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
                    } else {
                        sqlx::query("DELETE FROM endpoints WHERE branch_id = ? AND api_type = ? AND path = ? AND method = ?")
                            .bind(branch_id)
                            .bind(api_type.as_str())
                            .bind(&path)
                            .bind(&method)
                            .execute(&mut *tx)
                            .await
                            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
                    }
                }
            }
        }

        tx.commit()
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(())
    }

    // --- Roles and Groups ---

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

    async fn upsert_pending_spec(
        &self,
        service_id: i64,
        branch: &str,
        api_type: ApiType,
        content: &str,
        reason: &str,
        submitted_by: &str,
    ) -> Result<i64, RepositoryError> {
        // Replace rather than accumulate: one entry per key, latest wins.
        sqlx::query(
            "INSERT INTO pending_specs (service_id, branch, api_type, content, reason, submitted_by)
             VALUES (?, ?, ?, ?, ?, ?)
             ON CONFLICT (service_id, branch, api_type) DO UPDATE SET
               content = excluded.content,
               reason = excluded.reason,
               submitted_by = excluded.submitted_by,
               created_at = CURRENT_TIMESTAMP",
        )
        .bind(service_id)
        .bind(branch)
        .bind(api_type.as_str())
        .bind(content)
        .bind(reason)
        .bind(submitted_by)
        .execute(&self.pool)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;

        let row: (i64,) = sqlx::query_as(
            "SELECT id FROM pending_specs WHERE service_id = ? AND branch = ? AND api_type = ?",
        )
        .bind(service_id)
        .bind(branch)
        .bind(api_type.as_str())
        .fetch_one(&self.pool)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(row.0)
    }

    async fn get_pending_spec(&self, id: i64) -> Result<Option<PendingSpec>, RepositoryError> {
        let row: Option<PendingSpecRow> = sqlx::query_as(
            "SELECT p.id, s.name, p.branch, p.api_type, p.content, p.reason, p.submitted_by,
                        CAST(p.created_at AS TEXT)
                   FROM pending_specs p JOIN services s ON s.id = p.service_id
                  WHERE p.id = ?",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(row.and_then(pending_from_row))
    }

    async fn list_pending_specs(&self) -> Result<Vec<PendingSpec>, RepositoryError> {
        let rows: Vec<PendingSpecRow> = sqlx::query_as(
            "SELECT p.id, s.name, p.branch, p.api_type, p.content, p.reason, p.submitted_by,
                        CAST(p.created_at AS TEXT)
                   FROM pending_specs p JOIN services s ON s.id = p.service_id
                  ORDER BY p.created_at DESC, p.id DESC",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(rows.into_iter().filter_map(pending_from_row).collect())
    }

    async fn delete_pending_spec(&self, id: i64) -> Result<bool, RepositoryError> {
        let result = sqlx::query("DELETE FROM pending_specs WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(result.rows_affected() > 0)
    }

    async fn clear_pending_spec(
        &self,
        service_id: i64,
        branch: &str,
        api_type: ApiType,
    ) -> Result<bool, RepositoryError> {
        let result = sqlx::query(
            "DELETE FROM pending_specs WHERE service_id = ? AND branch = ? AND api_type = ?",
        )
        .bind(service_id)
        .bind(branch)
        .bind(api_type.as_str())
        .execute(&self.pool)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(result.rows_affected() > 0)
    }

    // --- Producer Onboarding ---

    async fn set_producer_onboarding(
        &self,
        service_id: i64,
        onboarding: bool,
    ) -> Result<(), RepositoryError> {
        sqlx::query("UPDATE services SET onboarding = ? WHERE id = ?")
            .bind(onboarding)
            .bind(service_id)
            .execute(&self.pool)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(())
    }

    async fn is_producer_onboarding(&self, service_id: i64) -> Result<bool, RepositoryError> {
        let row: Option<(bool,)> = sqlx::query_as("SELECT onboarding FROM services WHERE id = ?")
            .bind(service_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(row.map(|r| r.0).unwrap_or(false))
    }

    async fn list_onboarding_producers(&self) -> Result<Vec<String>, RepositoryError> {
        let rows: Vec<(String,)> =
            sqlx::query_as("SELECT name FROM services WHERE onboarding = TRUE ORDER BY name")
                .fetch_all(&self.pool)
                .await
                .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(rows.into_iter().map(|r| r.0).collect())
    }

    // --- Maintainer Scope ---

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
        branch_name: &str,
        channel: &str,
        message_name: &str,
    ) -> Result<Option<ChannelMessageContract>, RepositoryError> {
        let row: Option<(i64, String)> = sqlx::query_as(
            "SELECT owner_service_id, payload_yaml FROM channel_message_contracts WHERE branch_name = ? AND channel = ? AND message_name = ?",
        )
        .bind(branch_name)
        .bind(channel)
        .bind(message_name)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;

        Ok(
            row.map(|(owner_service_id, payload_yaml)| ChannelMessageContract {
                branch_name: branch_name.to_string(),
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
            "INSERT INTO channel_message_contracts (branch_name, channel, message_name, owner_service_id, payload_yaml) \
             VALUES (?, ?, ?, ?, ?) \
             ON CONFLICT (branch_name, channel, message_name) \
             DO UPDATE SET owner_service_id = excluded.owner_service_id, payload_yaml = excluded.payload_yaml",
        )
        .bind(&contract.branch_name)
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
        branch_name: &str,
        channel: &str,
        message_name: &str,
    ) -> Result<(), RepositoryError> {
        sqlx::query(
            "DELETE FROM channel_message_contracts WHERE branch_name = ? AND channel = ? AND message_name = ?",
        )
        .bind(branch_name)
        .bind(channel)
        .bind(message_name)
        .execute(&self.pool)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(())
    }

    async fn list_channel_message_contracts(
        &self,
        branch_name: &str,
    ) -> Result<Vec<ChannelMessageContract>, RepositoryError> {
        let rows: Vec<(String, String, i64, String)> = sqlx::query_as(
            "SELECT channel, message_name, owner_service_id, payload_yaml FROM channel_message_contracts WHERE branch_name = ? ORDER BY channel, message_name",
        )
        .bind(branch_name)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;

        Ok(rows
            .into_iter()
            .map(
                |(channel, message_name, owner_service_id, payload_yaml)| ChannelMessageContract {
                    branch_name: branch_name.to_string(),
                    channel,
                    message_name,
                    owner_service_id,
                    payload_yaml,
                },
            )
            .collect())
    }

    async fn delete_orphaned_channel_message_contracts(
        &self,
        live_branches: &[String],
    ) -> Result<u64, RepositoryError> {
        let existing: Vec<(String,)> =
            sqlx::query_as("SELECT DISTINCT branch_name FROM channel_message_contracts")
                .fetch_all(&self.pool)
                .await
                .map_err(|e| RepositoryError::Internal(e.to_string()))?;

        let live: std::collections::HashSet<&str> =
            live_branches.iter().map(String::as_str).collect();

        let mut deleted = 0u64;
        for (branch_name,) in existing {
            if live.contains(branch_name.as_str()) {
                continue;
            }
            let result = sqlx::query("DELETE FROM channel_message_contracts WHERE branch_name = ?")
                .bind(&branch_name)
                .execute(&self.pool)
                .await
                .map_err(|e| RepositoryError::Internal(e.to_string()))?;
            deleted += result.rows_affected();
        }
        Ok(deleted)
    }

    async fn insert_audit_log(
        &self,
        username: &str,
        log: NewAuditLog<'_>,
    ) -> Result<(), RepositoryError> {
        let timestamp = chrono::Utc::now().to_rfc3339();
        sqlx::query(
            "INSERT INTO audit_logs (timestamp, username, action, details, service, branch, action_type, diff) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(timestamp)
        .bind(username)
        .bind(log.action)
        .bind(log.details)
        .bind(log.service)
        .bind(log.branch)
        .bind(log.action_type)
        .bind(log.diff)
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
            "SELECT id, timestamp, username, action, details, service, branch, action_type, diff FROM audit_logs WHERE 1=1",
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
        if filter.branch_wildcard.is_some() {
            sql.push_str(" AND branch LIKE ?");
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
        if let Some(ref val) = filter.branch_wildcard {
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
                |(id, timestamp, username, action, details, service, branch, action_type, diff)| {
                    AuditLogEntry {
                        id,
                        timestamp,
                        username,
                        action,
                        details,
                        service,
                        branch,
                        action_type,
                        diff,
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
            "SELECT id, timestamp, username, action, details, service, branch, action_type, diff FROM audit_logs ORDER BY id DESC LIMIT ?"
        )
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;

        Ok(rows
            .into_iter()
            .map(
                |(id, timestamp, username, action, details, service, branch, action_type, diff)| {
                    AuditLogEntry {
                        id,
                        timestamp,
                        username,
                        action,
                        details,
                        service,
                        branch,
                        action_type,
                        diff,
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

    async fn list_branches_with_metadata(&self) -> Result<Vec<BranchMetadata>, RepositoryError> {
        let rows: Vec<(String, String)> = sqlx::query_as(
            "SELECT name, MAX(updated_at) as last_modified FROM branches GROUP BY name",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;

        Ok(rows
            .into_iter()
            .map(|(name, last_modified)| BranchMetadata {
                name,
                last_modified,
            })
            .collect())
    }

    async fn list_branch_last_published(
        &self,
    ) -> Result<Vec<(String, String, String)>, RepositoryError> {
        sqlx::query_as(
            "SELECT s.name, b.name, b.updated_at
             FROM branches b JOIN services s ON s.id = b.service_id",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))
    }

    async fn list_branch_endpoint_counts(
        &self,
    ) -> Result<Vec<(String, String, i64)>, RepositoryError> {
        sqlx::query_as(
            "SELECT s.name, b.name, COUNT(e.id)
             FROM branches b
             JOIN services s ON s.id = b.service_id
             LEFT JOIN endpoints e ON e.branch_id = b.id AND e.deleted = FALSE
             GROUP BY s.id, b.id, s.name, b.name",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))
    }
}

#[cfg(test)]
mod channel_message_contract_tests {
    use super::*;

    async fn setup() -> SqliteSpecRepository {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        let repo = SqliteSpecRepository::new(pool);
        repo.run_migrations().await.unwrap();
        repo
    }

    fn contract(
        branch: &str,
        channel: &str,
        message: &str,
        owner: i64,
        payload: &str,
    ) -> ChannelMessageContract {
        ChannelMessageContract {
            branch_name: branch.to_string(),
            channel: channel.to_string(),
            message_name: message.to_string(),
            owner_service_id: owner,
            payload_yaml: payload.to_string(),
        }
    }

    #[tokio::test]
    async fn upsert_then_get_roundtrips() {
        let repo = setup().await;
        let owner = repo.ensure_service("producer").await.unwrap();
        let c = contract("main", "orders", "OrderPlaced", owner, "type: object");
        repo.upsert_channel_message_contract(&c).await.unwrap();

        let got = repo
            .get_channel_message_contract("main", "orders", "OrderPlaced")
            .await
            .unwrap();
        assert_eq!(got, Some(c));
    }

    #[tokio::test]
    async fn get_missing_returns_none() {
        let repo = setup().await;
        let got = repo
            .get_channel_message_contract("main", "orders", "Nope")
            .await
            .unwrap();
        assert_eq!(got, None);
    }

    #[tokio::test]
    async fn upsert_same_key_updates_owner_and_payload() {
        let repo = setup().await;
        let a = repo.ensure_service("a").await.unwrap();
        let b = repo.ensure_service("b").await.unwrap();
        repo.upsert_channel_message_contract(&contract("main", "orders", "OrderPlaced", a, "v1"))
            .await
            .unwrap();
        repo.upsert_channel_message_contract(&contract("main", "orders", "OrderPlaced", b, "v2"))
            .await
            .unwrap();

        let got = repo
            .get_channel_message_contract("main", "orders", "OrderPlaced")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(got.owner_service_id, b);
        assert_eq!(got.payload_yaml, "v2");
        // Still a single row for the key.
        let all = repo.list_channel_message_contracts("main").await.unwrap();
        assert_eq!(all.len(), 1);
    }

    #[tokio::test]
    async fn delete_removes_only_that_message() {
        let repo = setup().await;
        let owner = repo.ensure_service("producer").await.unwrap();
        repo.upsert_channel_message_contract(&contract("main", "orders", "A", owner, "y"))
            .await
            .unwrap();
        repo.upsert_channel_message_contract(&contract("main", "orders", "B", owner, "y"))
            .await
            .unwrap();

        repo.delete_channel_message_contract("main", "orders", "A")
            .await
            .unwrap();

        assert!(
            repo.get_channel_message_contract("main", "orders", "A")
                .await
                .unwrap()
                .is_none()
        );
        assert!(
            repo.get_channel_message_contract("main", "orders", "B")
                .await
                .unwrap()
                .is_some()
        );
    }

    #[tokio::test]
    async fn list_is_scoped_to_branch_and_sorted() {
        let repo = setup().await;
        let owner = repo.ensure_service("producer").await.unwrap();
        repo.upsert_channel_message_contract(&contract("main", "orders", "B", owner, "y"))
            .await
            .unwrap();
        repo.upsert_channel_message_contract(&contract("main", "orders", "A", owner, "y"))
            .await
            .unwrap();
        repo.upsert_channel_message_contract(&contract("main", "audit", "Z", owner, "y"))
            .await
            .unwrap();
        repo.upsert_channel_message_contract(&contract("feature", "orders", "A", owner, "y"))
            .await
            .unwrap();

        let main = repo.list_channel_message_contracts("main").await.unwrap();
        let keys: Vec<(String, String)> = main
            .iter()
            .map(|c| (c.channel.clone(), c.message_name.clone()))
            .collect();
        assert_eq!(
            keys,
            vec![
                ("audit".to_string(), "Z".to_string()),
                ("orders".to_string(), "A".to_string()),
                ("orders".to_string(), "B".to_string()),
            ]
        );

        let feature = repo
            .list_channel_message_contracts("feature")
            .await
            .unwrap();
        assert_eq!(feature.len(), 1);
    }

    #[tokio::test]
    async fn orphan_cleanup_keeps_live_and_removes_dead() {
        let repo = setup().await;
        let owner = repo.ensure_service("producer").await.unwrap();
        repo.upsert_channel_message_contract(&contract("main", "orders", "A", owner, "y"))
            .await
            .unwrap();
        repo.upsert_channel_message_contract(&contract("gone", "orders", "A", owner, "y"))
            .await
            .unwrap();

        let removed = repo
            .delete_orphaned_channel_message_contracts(&["main".to_string()])
            .await
            .unwrap();
        assert_eq!(removed, 1);
        assert_eq!(
            repo.list_channel_message_contracts("main")
                .await
                .unwrap()
                .len(),
            1
        );
        assert!(
            repo.list_channel_message_contracts("gone")
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[tokio::test]
    async fn orphan_cleanup_empty_live_removes_all() {
        let repo = setup().await;
        let owner = repo.ensure_service("producer").await.unwrap();
        repo.upsert_channel_message_contract(&contract("main", "orders", "A", owner, "y"))
            .await
            .unwrap();

        let removed = repo
            .delete_orphaned_channel_message_contracts(&[])
            .await
            .unwrap();
        assert_eq!(removed, 1);
        assert!(
            repo.list_channel_message_contracts("main")
                .await
                .unwrap()
                .is_empty()
        );
    }
}

#[cfg(test)]
mod endpoint_revive_tests {
    use super::*;

    async fn setup() -> SqliteSpecRepository {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        let repo = SqliteSpecRepository::new(pool);
        repo.run_migrations().await.unwrap();
        repo
    }

    fn insert(path: &str, method: &str, yaml: &str) -> SpecChange {
        SpecChange::Insert {
            api_type: ApiType::OpenApi,
            path: path.to_string(),
            normalized_path: path.to_string(),
            method: method.to_string(),
            yaml_content: yaml.to_string(),
            deprecated: false,
            external: false,
        }
    }

    // Regression: a soft-deleted endpoint (as produced by a protected-branch removal)
    // still occupies the UNIQUE(branch_id, api_type, path, method) slot. Re-introducing
    // the same path+method later — e.g. on a branch that is no longer protected, so the
    // application-layer re-introduction guard does not fire — used to fail with a
    // "duplicate key" constraint violation. It must now revive the row instead.
    #[tokio::test]
    async fn reinserting_soft_deleted_endpoint_revives_row() {
        let repo = setup().await;
        let service_id = repo.ensure_service("svc").await.unwrap();
        let branch_id = repo.ensure_branch(service_id, "main").await.unwrap();

        repo.apply_spec_changes(
            branch_id,
            vec![insert("/users", "GET", "v1")],
            false,
            None,
            None,
        )
        .await
        .unwrap();

        repo.apply_spec_changes(
            branch_id,
            vec![SpecChange::Delete {
                api_type: ApiType::OpenApi,
                path: "/users".to_string(),
                method: "GET".to_string(),
                soft_delete: true,
            }],
            false,
            None,
            None,
        )
        .await
        .unwrap();

        assert!(
            repo.get_endpoints_for_branch(branch_id)
                .await
                .unwrap()
                .is_empty(),
            "endpoint should be hidden after soft delete"
        );

        // Previously violated the UNIQUE constraint against the leftover soft-deleted row.
        repo.apply_spec_changes(
            branch_id,
            vec![insert("/users", "GET", "v2")],
            false,
            None,
            None,
        )
        .await
        .unwrap();

        let endpoints = repo.get_endpoints_for_branch(branch_id).await.unwrap();
        assert_eq!(
            endpoints.len(),
            1,
            "revived endpoint should be visible again"
        );
        assert_eq!(endpoints[0].path, "/users");
        assert_eq!(endpoints[0].method, "GET");
        assert_eq!(
            endpoints[0].yaml_content, "v2",
            "revive should refresh the stored content"
        );
    }
}
