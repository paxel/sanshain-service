use sqlx::{PgPool, Row};
use std::collections::HashMap;
use std::str::FromStr;
use tracing::instrument;

/// Row shape for `spec_versions` metadata queries (id, service_id, api_type,
/// major, minor, patch, stability, content_hash, provided_by,
/// created_at, updated_at, last_required_at) — everything but the document.
/// `service_id` and the version components are `INTEGER` columns on
/// PostgreSQL: ids are BIGINT (`i64`), version parts INTEGER (`i32`).
type SpecVersionRow = (
    i64,
    i64,
    String,
    i32,
    i32,
    i32,
    String,
    String,
    String,
    String,
    String,
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
    })
}

/// Row shape for audit-log queries (id, timestamp, username, action, details,
/// service, version, action_type, diff).
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
    UpsertSpecVersion,
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

impl<'r> sqlx::FromRow<'r, sqlx::postgres::PgRow> for ApiTokenRow {
    fn from_row(row: &'r sqlx::postgres::PgRow) -> Result<Self, sqlx::Error> {
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
pub struct PostgresSpecRepository {
    pub pool: PgPool,
}

impl PostgresSpecRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub async fn run_migrations(&self) -> Result<(), sqlx::migrate::MigrateError> {
        tracing::info!("Running PostgreSQL migrations...");
        let migrator = sqlx::migrate::Migrator::new(std::path::Path::new(
            "src/infrastructure/migrations/postgres",
        ))
        .await?;
        migrator.run(&self.pool).await?;
        Ok(())
    }
}

impl SpecRepository for PostgresSpecRepository {
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
        sqlx::query(
            r#"
            INSERT INTO spec_versions
              (service_id, api_type, major, minor, patch, stability, content, content_hash, provided_by, created_at, updated_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
            ON CONFLICT(service_id, api_type, major, minor, patch) DO UPDATE SET
                stability = EXCLUDED.stability,
                content = EXCLUDED.content,
                content_hash = EXCLUDED.content_hash,
                provided_by = EXCLUDED.provided_by,
                updated_at = EXCLUDED.updated_at
            "#,
        )
        .bind(params.service_id)
        .bind(params.api_type.as_str())
        .bind(params.version.major as i32)
        .bind(params.version.minor as i32)
        .bind(params.version.patch as i32)
        .bind(params.stability.as_str())
        .bind(params.content)
        .bind(params.content_hash)
        .bind(params.provided_by)
        .bind(params.now_iso)
        .bind(params.now_iso)
        .execute(&mut *tx)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;

        let (id,): (i64,) = sqlx::query_as(
            "SELECT id FROM spec_versions WHERE service_id = $1 AND api_type = $2 AND major = $3 AND minor = $4 AND patch = $5",
        )
        .bind(params.service_id)
        .bind(params.api_type.as_str())
        .bind(params.version.major as i32)
        .bind(params.version.minor as i32)
        .bind(params.version.patch as i32)
        .fetch_one(&mut *tx)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;

        // A version's endpoint set is complete by definition — replace it
        // wholesale rather than diffing in SQL.
        sqlx::query("DELETE FROM endpoints WHERE spec_version_id = $1")
            .bind(id)
            .execute(&mut *tx)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        for endpoint in &params.endpoints {
            sqlx::query(
                "INSERT INTO endpoints (spec_version_id, api_type, path, normalized_path, method, yaml_content, deprecated, external) VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
            )
            .bind(id)
            .bind(endpoint.api_type.as_str())
            .bind(&endpoint.path)
            .bind(&endpoint.normalized_path)
            .bind(&endpoint.method)
            .bind(&endpoint.yaml_content)
            .bind(endpoint.deprecated)
            .bind(endpoint.external)
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
            "SELECT id, service_id, api_type, major, minor, patch, stability, content_hash, provided_by, created_at, updated_at, last_required_at \
             FROM spec_versions WHERE service_id = $1 AND api_type = $2 AND major = $3 AND minor = $4 AND patch = $5",
        )
        .bind(service_id)
        .bind(api_type.as_str())
        .bind(version.major as i32)
        .bind(version.minor as i32)
        .bind(version.patch as i32)
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
            "SELECT id, service_id, api_type, major, minor, patch, stability, content_hash, provided_by, created_at, updated_at, last_required_at \
             FROM spec_versions WHERE service_id = $1 ORDER BY api_type, major, minor, patch",
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
            i32,
            i32,
            i32,
            String,
            String,
            String,
            String,
            String,
            Option<String>,
            i64,
        );
        let rows: Vec<Row> = sqlx::query_as(
            r#"
            SELECT s.name, v.id, v.service_id, v.api_type, v.major, v.minor, v.patch, v.stability,
                   v.content_hash, v.provided_by, v.created_at, v.updated_at, v.last_required_at,
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
            sqlx::query_as("SELECT content FROM spec_versions WHERE id = $1")
                .bind(spec_version_id)
                .fetch_optional(&self.pool)
                .await
                .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(row.map(|r| r.0))
    }

    async fn delete_spec_version(&self, spec_version_id: i64) -> Result<bool, RepositoryError> {
        let result = sqlx::query("DELETE FROM spec_versions WHERE id = $1")
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
        sqlx::query("UPDATE spec_versions SET last_required_at = $1 WHERE id = $2")
            .bind(now_iso)
            .bind(spec_version_id)
            .execute(&self.pool)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(())
    }

    async fn delete_expired_snapshots(&self, cutoff_iso: &str) -> Result<u64, RepositoryError> {
        // Use-based: a snapshot survives if it was provided (updated_at) OR
        // required (last_required_at) since the cutoff. GA never expires.
        let result = sqlx::query(
            r#"
            DELETE FROM spec_versions
            WHERE stability = 'snapshot'
              AND updated_at < $1
              AND (last_required_at IS NULL OR last_required_at < $2)
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
            WHERE d.spec_version_id = $1
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
        let row: Option<(i64,)> = sqlx::query_as("SELECT id FROM services WHERE name = $1")
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
        let row: Option<(String,)> = sqlx::query_as("SELECT name FROM services WHERE id = $1")
            .bind(service_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(row.map(|r| r.0))
    }
    async fn ensure_service(&self, name: &str) -> Result<i64, RepositoryError> {
        sqlx::query("INSERT INTO services (name) VALUES ($1) ON CONFLICT DO NOTHING")
            .bind(name)
            .execute(&self.pool)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;

        let row: (i64,) = sqlx::query_as("SELECT id FROM services WHERE name = $1")
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
        type EndpointRow = (i64, String, String, String, String, String, bool, bool);
        let rows: Vec<EndpointRow> = sqlx::query_as(
            "SELECT e.id, e.api_type, e.path, e.normalized_path, e.method, e.yaml_content, \
             e.deprecated, e.external \
             FROM endpoints e \
             WHERE e.spec_version_id = $1",
        )
        .bind(spec_version_id)
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

    async fn ensure_client(&self, name: &str) -> Result<i64, RepositoryError> {
        sqlx::query("INSERT INTO clients (name) VALUES ($1) ON CONFLICT DO NOTHING")
            .bind(name)
            .execute(&self.pool)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;

        let row: (i64,) = sqlx::query_as("SELECT id FROM clients WHERE name = $1")
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
    ) -> Result<Option<(i64, String, bool, bool)>, RepositoryError> {
        let normalized_path = crate::openapi::normalize_path(path);
        let row: Option<(i64, String, bool, bool)> = sqlx::query_as(
            r#"
            SELECT e.id, e.yaml_content, e.deprecated, e.external
            FROM endpoints e
            WHERE e.spec_version_id = $1 AND e.api_type = $2 AND e.normalized_path = $3 AND e.method = $4
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
            SELECT e.id, e.path, e.normalized_path, e.method, e.yaml_content, e.deprecated, e.external
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
            query_builder.push_bind(crate::openapi::normalize_path(path));
            query_builder.push(" AND e.method = ");
            query_builder.push_bind(method);
            query_builder.push(")");
        }
        query_builder.push(")");

        type BulkRow = (i64, String, String, String, String, bool, bool);
        let rows: Vec<BulkRow> = query_builder
            .build_query_as()
            .fetch_all(&self.pool)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;

        // Key the result by what the caller *asked for*, matched via the
        // normalized path, so lenient path matching works for bundles too.
        let mut by_normalized: HashMap<(String, String), (i64, String, bool, bool)> = rows
            .into_iter()
            .map(
                |(id, _path, normalized, method, yaml, deprecated, external)| {
                    ((normalized, method), (id, yaml, deprecated, external))
                },
            )
            .collect();
        for (path, method) in endpoints {
            let normalized = crate::openapi::normalize_path(path);
            if let Some(details) = by_normalized.remove(&(normalized, method.clone())) {
                result.insert((path.clone(), method.clone()), details);
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
            " ON CONFLICT(client_id, spec_version_id, api_type, path, method) DO UPDATE SET last_seen_at = EXCLUDED.last_seen_at",
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
            i32,
            i32,
            i32,
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
        type UnusedRow = (String, String, i32, i32, i32, String, String, String, bool);
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
            sqlx::query("DELETE FROM sessions WHERE user_id != $1")
                .bind(uid)
                .execute(&mut *tx)
                .await
                .map_err(|e| RepositoryError::Internal(e.to_string()))?;
            sqlx::query("DELETE FROM api_tokens WHERE user_id != $1")
                .bind(uid)
                .execute(&mut *tx)
                .await
                .map_err(|e| RepositoryError::Internal(e.to_string()))?;
            sqlx::query("DELETE FROM users WHERE id != $1")
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
        // spec_versions cascade endpoints and dependencies via FK; deleting
        // explicitly keeps both backends behaving identically.
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;

        let row: Option<(i64,)> = sqlx::query_as("SELECT id FROM services WHERE name = $1")
            .bind(name)
            .fetch_optional(&mut *tx)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;

        let service_id = match row {
            Some((id,)) => id,
            None => return Ok(false),
        };

        sqlx::query(
            "DELETE FROM dependencies WHERE spec_version_id IN (SELECT id FROM spec_versions WHERE service_id = $1)",
        )
        .bind(service_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        sqlx::query(
            "DELETE FROM endpoints WHERE spec_version_id IN (SELECT id FROM spec_versions WHERE service_id = $1)",
        )
        .bind(service_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        sqlx::query("DELETE FROM spec_versions WHERE service_id = $1")
            .bind(service_id)
            .execute(&mut *tx)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        sqlx::query("DELETE FROM services WHERE id = $1")
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
        let row: Option<(i64,)> = sqlx::query_as("SELECT id FROM clients WHERE name = $1")
            .bind(name)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;

        let client_id = match row {
            Some((id,)) => id,
            None => return Ok(false),
        };

        sqlx::query("DELETE FROM dependencies WHERE client_id = $1")
            .bind(client_id)
            .execute(&self.pool)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;

        sqlx::query("DELETE FROM clients WHERE id = $1")
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
        sqlx::query("UPDATE services SET icon = $1, domain = $2 WHERE name = $3")
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
            i32,
            i32,
            i32,
            String,
            String,
            String,
            Option<String>,
            bool,
            bool,
        );
        let rows: Vec<ClientEndpointRow> = sqlx::query_as(
            "SELECT d.api_type, s.name, v.major, v.minor, v.patch, v.stability, d.path, d.method, e.yaml_content, \
             COALESCE(e.deprecated, FALSE), COALESCE(e.external, FALSE) \
             FROM dependencies d \
             JOIN clients c ON d.client_id = c.id \
             JOIN spec_versions v ON d.spec_version_id = v.id \
             JOIN services s ON v.service_id = s.id \
             LEFT JOIN endpoints e ON e.spec_version_id = v.id AND e.api_type = d.api_type \
                  AND e.normalized_path = d.normalized_path AND e.method = d.method \
             WHERE c.name = $1 \
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
                    external,
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
                        external,
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
            "SELECT id, username, password_hash, approved FROM users WHERE username = $1",
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
        sqlx::query("INSERT INTO users (username, password_hash, approved) VALUES ($1, $2, $3)")
            .bind(username)
            .bind(password_hash)
            .bind(approved)
            .execute(&self.pool)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;

        let row: (i64, String, String, bool) = sqlx::query_as(
            "SELECT id, username, password_hash, approved FROM users WHERE username = $1",
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
            sqlx::query("UPDATE users SET approved = TRUE WHERE id = $1 AND approved = FALSE")
                .bind(user_id)
                .execute(&self.pool)
                .await
                .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(result.rows_affected() > 0)
    }

    async fn delete_user(&self, user_id: i64) -> Result<bool, RepositoryError> {
        let result = sqlx::query("DELETE FROM users WHERE id = $1")
            .bind(user_id)
            .execute(&self.pool)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(result.rows_affected() > 0)
    }

    async fn update_password(&self, user_id: i64, new_hash: &str) -> Result<(), RepositoryError> {
        sqlx::query("UPDATE users SET password_hash = $1 WHERE id = $2")
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
        sqlx::query(
            "INSERT INTO sessions (token, user_id, expires_at) VALUES ($1, $2, $3::timestamp)",
        )
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
            SELECT s.user_id, u.id, u.username, u.password_hash, s.expires_at::text, u.approved
            FROM sessions s
            JOIN users u ON s.user_id = u.id
            WHERE s.token = $1 AND s.expires_at > NOW()
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
        sqlx::query("DELETE FROM sessions WHERE token = $1")
            .bind(hash_session_token(token))
            .execute(&self.pool)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(())
    }

    async fn get_setting(&self, key: &str) -> Result<Option<String>, RepositoryError> {
        let row: Option<(String,)> = sqlx::query_as("SELECT value FROM settings WHERE key = $1")
            .bind(key)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(row.map(|r| r.0))
    }

    async fn set_setting(&self, key: &str, value: &str) -> Result<(), RepositoryError> {
        sqlx::query("INSERT INTO settings (key, value) VALUES ($1, $2) ON CONFLICT (key) DO UPDATE SET value = EXCLUDED.value")
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
        sqlx::query("INSERT INTO api_tokens (id, user_id, name, token_hash, created_at, expires_at) VALUES ($1, $2, $3, $4, $5, $6)")
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
                if msg.contains("duplicate key") || msg.contains("unique constraint") {
                    RepositoryError::Conflict
                } else {
                    RepositoryError::Internal(msg)
                }
            })?;
        Ok(())
    }

    async fn list_api_tokens(&self, user_id: i64) -> Result<Vec<ApiToken>, RepositoryError> {
        let rows: Vec<ApiTokenRow> = sqlx::query_as(
            "SELECT id, user_id, name, token_hash, created_at, expires_at, last_used_at FROM api_tokens WHERE user_id = $1 ORDER BY created_at DESC"
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
        let result = sqlx::query("DELETE FROM api_tokens WHERE id = $1 AND user_id = $2")
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
            WHERE t.token_hash = $1 AND t.expires_at > TO_CHAR(NOW(), 'YYYY-MM-DD HH24:MI:SS')
            "#,
        )
        .bind(token_hash)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;

        if row.is_some() {
            // Update last_used_at
            let _ = sqlx::query("UPDATE api_tokens SET last_used_at = TO_CHAR(NOW(), 'YYYY-MM-DD HH24:MI:SS') WHERE token_hash = $1")
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
        let result = sqlx::query("DELETE FROM dependencies WHERE last_seen_at < $1")
            .bind(cutoff_iso)
            .execute(&self.pool)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(result.rows_affected())
    }

    async fn grant_user_role(&self, user_id: i64, role: &str) -> Result<(), RepositoryError> {
        sqlx::query("INSERT INTO user_roles (user_id, role) VALUES ($1, $2) ON CONFLICT (user_id, role) DO NOTHING")
            .bind(user_id)
            .bind(role)
            .execute(&self.pool)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(())
    }

    async fn revoke_user_role(&self, user_id: i64, role: &str) -> Result<bool, RepositoryError> {
        let result = sqlx::query("DELETE FROM user_roles WHERE user_id = $1 AND role = $2")
            .bind(user_id)
            .bind(role)
            .execute(&self.pool)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(result.rows_affected() > 0)
    }

    async fn list_user_roles(&self, user_id: i64) -> Result<Vec<String>, RepositoryError> {
        let rows: Vec<(String,)> =
            sqlx::query_as("SELECT role FROM user_roles WHERE user_id = $1 ORDER BY role")
                .bind(user_id)
                .fetch_all(&self.pool)
                .await
                .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(rows.into_iter().map(|r| r.0).collect())
    }

    async fn effective_stored_roles(&self, user_id: i64) -> Result<Vec<String>, RepositoryError> {
        let rows: Vec<(String,)> = sqlx::query_as(
            "SELECT role FROM user_roles WHERE user_id = $1
             UNION
             SELECT gr.role FROM user_group_roles gr
               JOIN user_group_members gm ON gm.group_id = gr.group_id
             WHERE gm.user_id = $2
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
        sqlx::query("INSERT INTO user_groups (name, source) VALUES ($1, $2) ON CONFLICT (name, source) DO NOTHING")
            .bind(name)
            .bind(source.as_str())
            .execute(&self.pool)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        let row: (i64,) =
            sqlx::query_as("SELECT id FROM user_groups WHERE name = $1 AND source = $2")
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
        let result = sqlx::query("UPDATE user_groups SET name = $1 WHERE id = $2")
            .bind(name)
            .bind(group_id)
            .execute(&self.pool)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(result.rows_affected() > 0)
    }

    async fn delete_group(&self, group_id: i64) -> Result<bool, RepositoryError> {
        // Cascade would handle these, but removing them explicitly keeps the
        // two backends behaving identically.
        sqlx::query("DELETE FROM user_group_members WHERE group_id = $1")
            .bind(group_id)
            .execute(&self.pool)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        sqlx::query("DELETE FROM user_group_roles WHERE group_id = $1")
            .bind(group_id)
            .execute(&self.pool)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        let result = sqlx::query("DELETE FROM user_groups WHERE id = $1")
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
        sqlx::query("DELETE FROM user_group_roles WHERE group_id = $1")
            .bind(group_id)
            .execute(&mut *tx)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        for role in roles {
            sqlx::query("INSERT INTO user_group_roles (group_id, role) VALUES ($1, $2) ON CONFLICT (group_id, role) DO NOTHING")
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
            sqlx::query_as("SELECT role FROM user_group_roles WHERE group_id = $1 ORDER BY role")
                .bind(group_id)
                .fetch_all(&self.pool)
                .await
                .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(rows.into_iter().map(|r| r.0).collect())
    }

    async fn add_group_member(&self, group_id: i64, user_id: i64) -> Result<(), RepositoryError> {
        sqlx::query("INSERT INTO user_group_members (group_id, user_id) VALUES ($1, $2) ON CONFLICT (group_id, user_id) DO NOTHING")
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
            sqlx::query("DELETE FROM user_group_members WHERE group_id = $1 AND user_id = $2")
                .bind(group_id)
                .bind(user_id)
                .execute(&self.pool)
                .await
                .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(result.rows_affected() > 0)
    }

    async fn list_group_member_ids(&self, group_id: i64) -> Result<Vec<i64>, RepositoryError> {
        let rows: Vec<(i64,)> = sqlx::query_as(
            "SELECT user_id FROM user_group_members WHERE group_id = $1 ORDER BY user_id",
        )
        .bind(group_id)
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
            "INSERT INTO producer_user_maintainers (service_id, user_id) VALUES ($1, $2) ON CONFLICT (service_id, user_id) DO NOTHING",
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
            "DELETE FROM producer_user_maintainers WHERE service_id = $1 AND user_id = $2",
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
            "INSERT INTO producer_group_maintainers (service_id, group_id) VALUES ($1, $2) ON CONFLICT (service_id, group_id) DO NOTHING",
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
            "DELETE FROM producer_group_maintainers WHERE service_id = $1 AND group_id = $2",
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
            "SELECT user_id FROM producer_user_maintainers WHERE service_id = $1 ORDER BY user_id",
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
            "SELECT group_id FROM producer_group_maintainers WHERE service_id = $1 ORDER BY group_id",
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
                  WHERE service_id = $1 AND user_id = $2
                 UNION ALL
                 SELECT 1 FROM producer_group_maintainers pgm
                   JOIN user_group_members gm ON gm.group_id = pgm.group_id
                  WHERE pgm.service_id = $3 AND gm.user_id = $4
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
              WHERE m.user_id = $1
             UNION
             SELECT s.name FROM services s
               JOIN producer_group_maintainers pgm ON pgm.service_id = s.id
               JOIN user_group_members gm ON gm.group_id = pgm.group_id
              WHERE gm.user_id = $2
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
            sqlx::query("INSERT INTO service_tags (service_id, tag) VALUES ($1, $2) ON CONFLICT (service_id, tag) DO NOTHING")
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
            "SELECT owner_service_id, payload_yaml FROM channel_message_contracts WHERE channel = $1 AND message_name = $2",
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
             VALUES ($1, $2, $3, $4) \
             ON CONFLICT (channel, message_name) \
             DO UPDATE SET owner_service_id = EXCLUDED.owner_service_id, payload_yaml = EXCLUDED.payload_yaml",
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
        sqlx::query(
            "DELETE FROM channel_message_contracts WHERE channel = $1 AND message_name = $2",
        )
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
            "INSERT INTO audit_logs (timestamp, username, action, details, service, version, action_type, diff) VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
        )
        .bind(timestamp)
        .bind(username)
        .bind(log.action)
        .bind(log.details)
        .bind(log.service)
        .bind(log.version)
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
            "SELECT id, timestamp, username, action, details, service, version, action_type, diff FROM audit_logs WHERE 1=1",
        );
        let mut param_idx = 1;

        if filter.from_date.is_some() {
            sql.push_str(&format!(" AND timestamp >= ${}", param_idx));
            param_idx += 1;
        }
        if filter.to_date.is_some() {
            sql.push_str(&format!(" AND timestamp <= ${}", param_idx));
            param_idx += 1;
        }
        if filter.action_type.is_some() {
            sql.push_str(&format!(" AND action_type = ${}", param_idx));
            param_idx += 1;
        }
        if filter.service_wildcard.is_some() {
            sql.push_str(&format!(" AND service LIKE ${}", param_idx));
            param_idx += 1;
        }
        if filter.version_wildcard.is_some() {
            sql.push_str(&format!(" AND version LIKE ${}", param_idx));
            param_idx += 1;
        }

        sql.push_str(&format!(" ORDER BY id DESC LIMIT ${}", param_idx));

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
        if let Some(ref val) = filter.version_wildcard {
            query = query.bind(val);
        }
        query = query.bind(filter.limit as i64);

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
            "SELECT id, timestamp, username, action, details, service, version, action_type, diff FROM audit_logs ORDER BY id DESC LIMIT $1"
        )
        .bind(limit as i64)
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
            "SELECT item_name FROM user_favorites WHERE user_id = $1 AND item_type = $2 ORDER BY item_name"
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
            "INSERT INTO user_favorites (user_id, item_type, item_name) VALUES ($1, $2, $3) ON CONFLICT(user_id, item_type, item_name) DO NOTHING"
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
            "DELETE FROM user_favorites WHERE user_id = $1 AND item_type = $2 AND item_name = $3",
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
