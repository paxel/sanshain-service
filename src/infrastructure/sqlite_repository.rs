use std::str::FromStr;
use std::collections::HashMap;
use sqlx::{SqlitePool, Row};

use crate::domain::models::*;
use crate::domain::ports::{RecordDependencyParams, RepositoryError, SpecRepository};

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
        let migrator = sqlx::migrate::Migrator::new(std::path::Path::new("src/infrastructure/migrations/sqlite")).await?;
        migrator.run(&self.pool).await?;

        // Backfill normalized_path
        self.backfill_normalized_paths().await.map_err(|e| {
            tracing::error!("Failed to backfill normalized paths: {}", e);
            sqlx::migrate::MigrateError::Execute(sqlx::Error::Configuration(format!("Backfill failed: {}", e).into()))
        })?;

        Ok(())
    }

    async fn backfill_normalized_paths(&self) -> Result<(), RepositoryError> {
        // Endpoints backfill
        let rows: Vec<(i64, String)> = sqlx::query_as("SELECT id, path FROM endpoints WHERE normalized_path = '' OR normalized_path IS NULL")
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

impl SpecRepository for SqliteSpecRepository {
    async fn find_service(&self, name: &str) -> Result<Option<i64>, RepositoryError> {
        let row: Option<(i64,)> = sqlx::query_as("SELECT id FROM services WHERE name = ?")
            .bind(name)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(row.map(|r| r.0))
    }
    async fn find_branch(&self, service_id: i64, branch_name: &str) -> Result<Option<i64>, RepositoryError> {
        let row: Option<(i64,)> = sqlx::query_as("SELECT id FROM branches WHERE service_id = ? AND name = ?")
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

    async fn ensure_branch(&self, service_id: i64, branch_name: &str) -> Result<i64, RepositoryError> {
        let now = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();
        sqlx::query("INSERT OR IGNORE INTO branches (service_id, name, updated_at) VALUES (?, ?, ?)")
            .bind(service_id)
            .bind(branch_name)
            .bind(&now)
            .execute(&self.pool)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;

        sqlx::query("UPDATE branches SET updated_at = ? WHERE service_id = ? AND name = ?")
            .bind(&now)
            .bind(service_id)
            .bind(branch_name)
            .execute(&self.pool)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;

        let row: (i64,) = sqlx::query_as("SELECT id FROM branches WHERE service_id = ? AND name = ?")
            .bind(service_id)
            .bind(branch_name)
            .fetch_one(&self.pool)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;

        Ok(row.0)
    }

    async fn get_endpoints_for_branch(&self, branch_id: i64) -> Result<Vec<EndpointRecord>, RepositoryError> {
        let rows: Vec<(i64, String, String, String, String, String)> = sqlx::query_as(
            "SELECT id, api_type, path, normalized_path, method, yaml_content FROM endpoints WHERE branch_id = ? AND deleted = FALSE"
        )
        .bind(branch_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;

        Ok(rows
            .into_iter()
            .map(|(id, api_type, path, normalized_path, method, yaml_content)| EndpointRecord {
                id: Some(id),
                api_type: ApiType::from_str(&api_type).unwrap_or_default(),
                path,
                normalized_path,
                method,
                yaml_content,
            })
            .collect())
    }

    async fn insert_endpoint(&self, branch_id: i64, endpoint: &EndpointRecord) -> Result<(), RepositoryError> {
        sqlx::query("INSERT INTO endpoints (branch_id, api_type, path, normalized_path, method, yaml_content) VALUES (?, ?, ?, ?, ?, ?)")
            .bind(branch_id)
            .bind(endpoint.api_type.as_str())
            .bind(&endpoint.path)
            .bind(&endpoint.normalized_path)
            .bind(&endpoint.method)
            .bind(&endpoint.yaml_content)
            .execute(&self.pool)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;

        Ok(())
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
    ) -> Result<Option<(i64, String)>, RepositoryError> {
        let normalized_path = crate::openapi::normalize_path(path);
        let row: Option<(i64, String)> = sqlx::query_as(
            r#"
            SELECT e.id, e.yaml_content
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

    async fn find_endpoints_bulk(
        &self,
        service_id: i64,
        branch_name: &str,
        api_type: ApiType,
        endpoints: &[(String, String)],
    ) -> Result<HashMap<(String, String), (i64, String)>, RepositoryError> {
        let mut result = HashMap::new();
        if endpoints.is_empty() {
            return Ok(result);
        }

        let mut query_builder = sqlx::QueryBuilder::new(
            r#"
            SELECT e.id, e.path, e.method, e.yaml_content
            FROM endpoints e
            JOIN branches b ON e.branch_id = b.id
            WHERE b.service_id = "#
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

        let rows: Vec<(i64, String, String, String)> = query_builder
            .build_query_as()
            .fetch_all(&self.pool)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;

        for (id, path, method, yaml) in rows {
            result.insert((path, method), (id, yaml));
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

    async fn record_dependencies_bulk(
        &self,
        params: Vec<RecordDependencyParams<'_>>,
    ) -> Result<(), RepositoryError> {
        if params.is_empty() {
            return Ok(());
        }

        let now = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();

        // Split into resolved (has endpoint_id) and missing (NULL endpoint_id)
        let (resolved, missing): (Vec<_>, Vec<_>) = params.into_iter().partition(|p| p.endpoint_id.is_some());

        if !resolved.is_empty() {
            let mut query_builder = sqlx::QueryBuilder::new(
                "INSERT INTO dependencies (client_id, endpoint_id, api_type, requested_service_id, requested_branch_name, requested_path, requested_normalized_path, requested_method, last_seen_at) "
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

            query_builder.build().execute(&self.pool).await.map_err(|e| RepositoryError::Internal(e.to_string()))?;
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
        let row: (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM protected_branches WHERE pattern = ?"
        )
        .bind(branch_name)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;

        Ok(row.0 > 0)
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
        let rows: Vec<(String,)> = sqlx::query_as(
            "SELECT pattern FROM protected_branches ORDER BY pattern"
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;

        Ok(rows.into_iter().map(|r| r.0).collect())
    }

    async fn update_endpoint(&self, branch_id: i64, api_type: ApiType, path: &str, method: &str, yaml_content: &str) -> Result<(), RepositoryError> {
        sqlx::query("UPDATE endpoints SET yaml_content = ? WHERE branch_id = ? AND api_type = ? AND path = ? AND method = ?")
            .bind(yaml_content)
            .bind(branch_id)
            .bind(api_type.as_str())
            .bind(path)
            .bind(method)
            .execute(&self.pool)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(())
    }

    async fn soft_delete_endpoint(&self, branch_id: i64, api_type: ApiType, path: &str, method: &str) -> Result<(), RepositoryError> {
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

    async fn hard_delete_endpoint(&self, branch_id: i64, api_type: ApiType, path: &str, method: &str) -> Result<(), RepositoryError> {
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

    async fn is_endpoint_deleted(&self, branch_id: i64, api_type: ApiType, path: &str, method: &str) -> Result<bool, RepositoryError> {
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

    async fn get_report(&self, branch: &str) -> Result<DependencyReport, RepositoryError> {
        let mut conn = self.pool.acquire().await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;

        // Dependency graph
        let dependency_rows: Vec<(String, String, String, String, String)> = sqlx::query_as(
            r#"
            SELECT c.name, d.api_type, s.name, d.requested_path, d.requested_method
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
            .map(|(client, api_type, service, path, method)| DependencyInfo {
                api_type: ApiType::from_str(&api_type).unwrap_or_default(),
                client,
                service,
                path,
                method,
            })
            .collect();

        // Unused endpoints
        let unused_rows: Vec<(String, String, String, String)> = sqlx::query_as(
            r#"
            SELECT s.name, e.api_type, e.path, e.method
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
            .map(|(service, api_type, path, method)| EndpointInfo {
                api_type: ApiType::from_str(&api_type).unwrap_or_default(),
                service,
                path,
                method,
            })
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
            .map(|(client, api_type, service, path, method)| MissingEndpointInfo {
                api_type: ApiType::from_str(&api_type).unwrap_or_default(),
                client,
                service,
                path,
                method,
            })
            .collect();

        Ok(DependencyReport {
            branch: branch.to_string(),
            unused_endpoints,
            missing_endpoints,
            dependency_graph,
        })
    }

    async fn delete_all_services(&self) -> Result<u64, RepositoryError> {
        let r1 = sqlx::query("DELETE FROM endpoint_versions WHERE endpoint_id IN (SELECT id FROM endpoints)")
            .execute(&self.pool).await.map_err(|e| RepositoryError::Internal(e.to_string()))?;
        sqlx::query("DELETE FROM dependencies")
            .execute(&self.pool).await.map_err(|e| RepositoryError::Internal(e.to_string()))?;
        sqlx::query("DELETE FROM endpoints")
            .execute(&self.pool).await.map_err(|e| RepositoryError::Internal(e.to_string()))?;
        sqlx::query("DELETE FROM branches")
            .execute(&self.pool).await.map_err(|e| RepositoryError::Internal(e.to_string()))?;
        let result = sqlx::query("DELETE FROM services")
            .execute(&self.pool).await.map_err(|e| RepositoryError::Internal(e.to_string()))?;
        let _ = r1;
        Ok(result.rows_affected())
    }

    async fn delete_all_clients(&self) -> Result<u64, RepositoryError> {
        sqlx::query("DELETE FROM dependencies")
            .execute(&self.pool).await.map_err(|e| RepositoryError::Internal(e.to_string()))?;
        let result = sqlx::query("DELETE FROM clients")
            .execute(&self.pool).await.map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(result.rows_affected())
    }

    async fn delete_all_non_admin_users(&self) -> Result<u64, RepositoryError> {
        // Delete sessions for non-admin users
        sqlx::query("DELETE FROM sessions WHERE user_id IN (SELECT id FROM users WHERE is_admin = 0)")
            .execute(&self.pool).await.map_err(|e| RepositoryError::Internal(e.to_string()))?;
        // Delete API tokens for non-admin users
        sqlx::query("DELETE FROM api_tokens WHERE user_id IN (SELECT id FROM users WHERE is_admin = 0)")
            .execute(&self.pool).await.map_err(|e| RepositoryError::Internal(e.to_string()))?;
        let result = sqlx::query("DELETE FROM users WHERE is_admin = 0")
            .execute(&self.pool).await.map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(result.rows_affected())
    }

    async fn nuke_database(&self, keep_user_id: Option<i64>) -> Result<(), RepositoryError> {
        sqlx::query("DELETE FROM endpoint_versions")
            .execute(&self.pool).await.map_err(|e| RepositoryError::Internal(e.to_string()))?;
        sqlx::query("DELETE FROM dependencies")
            .execute(&self.pool).await.map_err(|e| RepositoryError::Internal(e.to_string()))?;
        sqlx::query("DELETE FROM endpoints")
            .execute(&self.pool).await.map_err(|e| RepositoryError::Internal(e.to_string()))?;
        sqlx::query("DELETE FROM branches")
            .execute(&self.pool).await.map_err(|e| RepositoryError::Internal(e.to_string()))?;
        sqlx::query("DELETE FROM services")
            .execute(&self.pool).await.map_err(|e| RepositoryError::Internal(e.to_string()))?;
        sqlx::query("DELETE FROM clients")
            .execute(&self.pool).await.map_err(|e| RepositoryError::Internal(e.to_string()))?;
        sqlx::query("DELETE FROM protected_branches")
            .execute(&self.pool).await.map_err(|e| RepositoryError::Internal(e.to_string()))?;
        // Delete all sessions except the current admin's
        if let Some(uid) = keep_user_id {
            sqlx::query("DELETE FROM sessions WHERE user_id != ?")
                .bind(uid).execute(&self.pool).await.map_err(|e| RepositoryError::Internal(e.to_string()))?;
            sqlx::query("DELETE FROM api_tokens WHERE user_id != ?")
                .bind(uid).execute(&self.pool).await.map_err(|e| RepositoryError::Internal(e.to_string()))?;
            sqlx::query("DELETE FROM users WHERE id != ?")
                .bind(uid).execute(&self.pool).await.map_err(|e| RepositoryError::Internal(e.to_string()))?;
        } else {
            sqlx::query("DELETE FROM sessions")
                .execute(&self.pool).await.map_err(|e| RepositoryError::Internal(e.to_string()))?;
            sqlx::query("DELETE FROM api_tokens")
                .execute(&self.pool).await.map_err(|e| RepositoryError::Internal(e.to_string()))?;
            sqlx::query("DELETE FROM users")
                .execute(&self.pool).await.map_err(|e| RepositoryError::Internal(e.to_string()))?;
        }
        Ok(())
    }

    async fn delete_service(&self, name: &str) -> Result<bool, RepositoryError> {
        let row: Option<(i64,)> = sqlx::query_as("SELECT id FROM services WHERE name = ?")
            .bind(name)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;

        let service_id = match row {
            Some((id,)) => id,
            None => return Ok(false),
        };

        // Get all branch IDs for this service
        let branch_rows: Vec<(i64,)> = sqlx::query_as("SELECT id FROM branches WHERE service_id = ?")
            .bind(service_id)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;

        for (branch_id,) in &branch_rows {
            // Delete dependencies referencing endpoints in this branch
            sqlx::query(
                "DELETE FROM dependencies WHERE endpoint_id IN (SELECT id FROM endpoints WHERE branch_id = ?)"
            )
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
        }

        // Delete dependencies referencing this service (including those with NULL endpoint_id)
        sqlx::query("DELETE FROM dependencies WHERE requested_service_id = ?")
            .bind(service_id)
            .execute(&self.pool)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;

        // Delete branches
        sqlx::query("DELETE FROM branches WHERE service_id = ?")
            .bind(service_id)
            .execute(&self.pool)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;

        // Delete service
        sqlx::query("DELETE FROM services WHERE id = ?")
            .bind(service_id)
            .execute(&self.pool)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;

        Ok(true)
    }

    async fn delete_branch(&self, service_name: &str, branch_name: &str) -> Result<bool, RepositoryError> {
        let row: Option<(i64,)> = sqlx::query_as(
            "SELECT b.id FROM branches b JOIN services s ON b.service_id = s.id WHERE s.name = ? AND b.name = ?"
        )
        .bind(service_name)
        .bind(branch_name)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;

        let branch_id = match row {
            Some((id,)) => id,
            None => return Ok(false),
        };

        // Delete dependencies referencing endpoints in this branch
        sqlx::query(
            "DELETE FROM dependencies WHERE endpoint_id IN (SELECT id FROM endpoints WHERE branch_id = ?)"
        )
        .bind(branch_id)
        .execute(&self.pool)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;

        // Delete dependencies referencing this branch by name (including NULL endpoint_id)
        let service_row: Option<(i64,)> = sqlx::query_as(
            "SELECT s.id FROM services s JOIN branches b ON b.service_id = s.id WHERE b.id = ?"
        )
        .bind(branch_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;

        if let Some((service_id,)) = service_row {
            sqlx::query(
                "DELETE FROM dependencies WHERE requested_service_id = ? AND requested_branch_name = ?"
            )
            .bind(service_id)
            .bind(branch_name)
            .execute(&self.pool)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        }

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

        Ok(true)
    }

    async fn delete_client(&self, name: &str) -> Result<bool, RepositoryError> {
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

    async fn list_services(&self) -> Result<Vec<String>, RepositoryError> {
        let rows: Vec<(String,)> = sqlx::query_as(
            "SELECT DISTINCT s.name FROM services s INNER JOIN branches b ON b.service_id = s.id ORDER BY s.name"
        )
            .fetch_all(&self.pool)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(rows.into_iter().map(|r| r.0).collect())
    }

    async fn list_services_detailed(&self) -> Result<Vec<ServiceSummary>, RepositoryError> {
        let rows: Vec<(String, Option<String>, Option<String>)> = sqlx::query_as(
            r#"
            SELECT s.name, s.fallback_branch, GROUP_CONCAT(b.name) as branches
            FROM services s
            JOIN branches b ON b.service_id = s.id
            GROUP BY s.id
            ORDER BY s.name
            "#
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;

        Ok(rows.into_iter().map(|(name, fallback_branch, branches_str)| {
            let branches = branches_str
                .map(|s| s.split(',').map(|b| b.to_string()).collect())
                .unwrap_or_default();
            ServiceSummary { name, fallback_branch, branches }
        }).collect())
    }

    async fn set_fallback_branch(&self, service_name: &str, branch: Option<&str>) -> Result<(), RepositoryError> {
        sqlx::query("UPDATE services SET fallback_branch = ? WHERE name = ?")
            .bind(branch)
            .bind(service_name)
            .execute(&self.pool)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(())
    }

    async fn get_fallback_branch(&self, service_name: &str) -> Result<Option<String>, RepositoryError> {
        let row: Option<(Option<String>,)> = sqlx::query_as("SELECT fallback_branch FROM services WHERE name = ?")
            .bind(service_name)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(row.and_then(|r| r.0))
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

    async fn list_clients(&self) -> Result<Vec<String>, RepositoryError> {
        let rows: Vec<(String,)> = sqlx::query_as(
            "SELECT DISTINCT c.name FROM clients c INNER JOIN dependencies d ON d.client_id = c.id ORDER BY c.name"
        )
            .fetch_all(&self.pool)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(rows.into_iter().map(|r| r.0).collect())
    }

    async fn list_client_branches(&self, client_name: &str) -> Result<Vec<String>, RepositoryError> {
        let rows: Vec<(String,)> = sqlx::query_as(
            "SELECT DISTINCT d.requested_branch_name \
             FROM dependencies d \
             JOIN clients c ON d.client_id = c.id \
             WHERE c.name = ? \
             ORDER BY d.requested_branch_name"
        )
        .bind(client_name)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(rows.into_iter().map(|r| r.0).collect())
    }

    async fn list_client_endpoints(&self, client_name: &str, branch: &str) -> Result<Vec<ClientEndpointInfo>, RepositoryError> {
        let rows: Vec<(String, String, String, String, String, Option<String>)> = sqlx::query_as(
            "SELECT d.api_type, s.name, d.requested_branch_name, d.requested_path, d.requested_method, e.yaml_content \
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
        Ok(rows.into_iter().map(|(api_type, service, branch, path, method, yaml_content)| ClientEndpointInfo {
            api_type: ApiType::from_str(&api_type).unwrap_or_default(),
            service, branch, path, method, yaml_content,
        }).collect())
    }

    async fn user_count(&self) -> Result<i64, RepositoryError> {
        let row: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM users")
            .fetch_one(&self.pool)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(row.0)
    }

    async fn find_user(&self, username: &str) -> Result<Option<User>, RepositoryError> {
        let row: Option<(i64, String, String, bool, bool)> = sqlx::query_as(
            "SELECT id, username, password_hash, is_admin, approved FROM users WHERE username = ?"
        )
        .bind(username)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;

        Ok(row.map(|(id, username, password_hash, is_admin, approved)| User {
            id,
            username,
            password_hash,
            is_admin,
            approved,
        }))
    }

    async fn create_user(&self, username: &str, password_hash: &str, is_admin: bool, approved: bool) -> Result<User, RepositoryError> {
        sqlx::query("INSERT INTO users (username, password_hash, is_admin, approved) VALUES (?, ?, ?, ?)")
            .bind(username)
            .bind(password_hash)
            .bind(is_admin)
            .bind(approved)
            .execute(&self.pool)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;

        let row: (i64, String, String, bool, bool) = sqlx::query_as(
            "SELECT id, username, password_hash, is_admin, approved FROM users WHERE username = ?"
        )
        .bind(username)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;

        Ok(User {
            id: row.0,
            username: row.1,
            password_hash: row.2,
            is_admin: row.3,
            approved: row.4,
        })
    }

    async fn list_users(&self) -> Result<Vec<User>, RepositoryError> {
        let rows: Vec<(i64, String, String, bool, bool)> = sqlx::query_as(
            "SELECT id, username, password_hash, is_admin, approved FROM users ORDER BY username"
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;

        Ok(rows.into_iter().map(|(id, username, password_hash, is_admin, approved)| User {
            id, username, password_hash, is_admin, approved,
        }).collect())
    }

    async fn approve_user(&self, user_id: i64) -> Result<bool, RepositoryError> {
        let result = sqlx::query("UPDATE users SET approved = TRUE WHERE id = ? AND approved = FALSE")
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

    async fn create_session(&self, user_id: i64, expires_at: &str) -> Result<Session, RepositoryError> {
        use rand::RngExt;
        let mut token_bytes = [0u8; 32];
        rand::rng().fill(&mut token_bytes);
        let token = hex::encode(token_bytes);
        self.create_session_with_token(user_id, &token, expires_at).await
    }

    async fn create_session_with_token(&self, user_id: i64, token: &str, expires_at: &str) -> Result<Session, RepositoryError> {
        sqlx::query("INSERT INTO sessions (token, user_id, expires_at) VALUES (?, ?, ?)")
            .bind(token)
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

    async fn validate_session(&self, token: &str) -> Result<Option<(User, Session)>, RepositoryError> {
        let row: Option<(i64, i64, String, String, String, bool, bool)> = sqlx::query_as(
            r#"
            SELECT s.user_id, u.id, u.username, u.password_hash, s.expires_at, u.is_admin, u.approved
            FROM sessions s
            JOIN users u ON s.user_id = u.id
            WHERE s.token = ? AND s.expires_at > datetime('now')
            "#
        )
        .bind(token)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;

        Ok(row.map(|(_, user_id, username, password_hash, expires_at, is_admin, approved)| {
            let user = User { id: user_id, username, password_hash, is_admin, approved };
            let session = Session { token: token.to_string(), user_id, expires_at };
            (user, session)
        }))
    }

    async fn delete_session(&self, token: &str) -> Result<(), RepositoryError> {
        sqlx::query("DELETE FROM sessions WHERE token = ?")
            .bind(token)
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

    async fn create_api_token(&self, id: &str, user_id: i64, name: &str, token_hash: &str, created_at: &str, expires_at: &str) -> Result<(), RepositoryError> {
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

    async fn delete_api_token(&self, token_id: &str, user_id: i64) -> Result<bool, RepositoryError> {
        let result = sqlx::query("DELETE FROM api_tokens WHERE id = ? AND user_id = ?")
            .bind(token_id)
            .bind(user_id)
            .execute(&self.pool)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(result.rows_affected() > 0)
    }

    async fn delete_stale_branches(&self, cutoff_iso: &str) -> Result<u64, RepositoryError> {
        // Delete endpoints and dependencies for stale non-protected branches, then the branches themselves
        let stale_branch_ids: Vec<(i64,)> = sqlx::query_as(
            r#"
            SELECT b.id FROM branches b
            JOIN services s ON b.service_id = s.id
            WHERE b.updated_at < ?
            AND NOT EXISTS (
                SELECT 1 FROM protected_branches pb WHERE b.name GLOB pb.pattern OR b.name = pb.pattern
            )
            "#,
        )
        .bind(cutoff_iso)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;

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
                )"#
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
        let row: Option<(i64, String, String, bool, bool)> = sqlx::query_as(
            r#"
            SELECT u.id, u.username, u.password_hash, u.is_admin, u.approved
            FROM api_tokens t
            JOIN users u ON t.user_id = u.id
            WHERE t.token_hash = ? AND t.expires_at > datetime('now')
            "#
        )
        .bind(token_hash)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;

        if row.is_some() {
            // Update last_used_at
            let _ = sqlx::query("UPDATE api_tokens SET last_used_at = datetime('now') WHERE token_hash = ?")
                .bind(token_hash)
                .execute(&self.pool)
                .await;
        }

        Ok(row.map(|(id, username, password_hash, is_admin, approved)| User {
            id, username, password_hash, is_admin, approved,
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

    async fn get_endpoint_id(&self, branch_id: i64, api_type: ApiType, path: &str, method: &str) -> Result<Option<i64>, RepositoryError> {
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

    async fn insert_endpoint_version(&self, endpoint_id: i64, version: i32, yaml_content: &str, diff: Option<&str>, created_at: &str) -> Result<(), RepositoryError> {
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
        let row: Option<(i32,)> = sqlx::query_as(
            "SELECT MAX(version) FROM endpoint_versions WHERE endpoint_id = ?"
        )
        .bind(endpoint_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(row.map(|(v,)| v).unwrap_or(0))
    }

    async fn get_endpoint_versions(&self, endpoint_id: i64) -> Result<Vec<EndpointVersion>, RepositoryError> {
        let rows: Vec<(i64, i64, i32, String, Option<String>, String)> = sqlx::query_as(
            "SELECT id, endpoint_id, version, yaml_content, diff_from_previous, created_at FROM endpoint_versions WHERE endpoint_id = ? ORDER BY version ASC"
        )
        .bind(endpoint_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;

        Ok(rows.into_iter().map(|(id, endpoint_id, version, yaml_content, diff_from_previous, created_at)| EndpointVersion {
            id, endpoint_id, version, yaml_content, diff_from_previous, created_at,
        }).collect())
    }

    async fn apply_spec_changes(
        &self,
        branch_id: i64,
        changes: Vec<SpecChange>,
        is_protected: bool,
    ) -> Result<(), RepositoryError> {
        tracing::debug!("Applying {} spec changes to branch {}", changes.len(), branch_id);
        let mut tx = self.pool.begin().await.map_err(|e| RepositoryError::Internal(e.to_string()))?;
        let now = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();

        for change in changes {
            match change {
                SpecChange::Insert { api_type, path, normalized_path, method, yaml_content } => {
                    tracing::debug!("Inserting {:?} endpoint: {} {}", api_type, method, path);
                    sqlx::query("INSERT INTO endpoints (branch_id, api_type, path, normalized_path, method, yaml_content, deleted) VALUES (?, ?, ?, ?, ?, ?, 0)")
                        .bind(branch_id)
                        .bind(api_type.as_str())
                        .bind(&path)
                        .bind(&normalized_path)
                        .bind(&method)
                        .bind(&yaml_content)
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
                        
                        sqlx::query("INSERT INTO endpoint_versions (endpoint_id, version, yaml_content, diff_from_previous, created_at) VALUES (?, 1, ?, NULL, ?)")
                            .bind(row.0)
                            .bind(&yaml_content)
                            .bind(&now)
                            .execute(&mut *tx)
                            .await
                            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
                    }
                }
                SpecChange::Update { api_type, path, normalized_path, method, yaml_content } => {
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
                        
                        sqlx::query("INSERT INTO endpoint_versions (endpoint_id, version, yaml_content, diff_from_previous, created_at) VALUES (?, ?, ?, ?, ?)")
                            .bind(endpoint_id)
                            .bind(version_row.0 + 1)
                            .bind(&yaml_content)
                            .bind(diff)
                            .bind(&now)
                            .execute(&mut *tx)
                            .await
                            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
                    }

                    sqlx::query("UPDATE endpoints SET yaml_content = ?, normalized_path = ?, deleted = 0 WHERE branch_id = ? AND api_type = ? AND path = ? AND method = ?")
                        .bind(&yaml_content)
                        .bind(&normalized_path)
                        .bind(branch_id)
                        .bind(api_type.as_str())
                        .bind(&path)
                        .bind(&method)
                        .execute(&mut *tx)
                        .await
                        .map_err(|e| RepositoryError::Internal(e.to_string()))?;
                }
                SpecChange::Delete { api_type, path, method, soft_delete } => {
                    tracing::debug!("Deleting {:?} endpoint: {} {} (soft: {})", api_type, method, path, soft_delete);
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

        tx.commit().await.map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(())
    }
}
