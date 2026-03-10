use sqlx::SqlitePool;

use crate::domain::models::*;
use crate::domain::ports::{RepositoryError, SpecRepository};

#[derive(Clone)]
pub struct SqliteSpecRepository {
    pub pool: SqlitePool,
}

impl SqliteSpecRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

impl SpecRepository for SqliteSpecRepository {
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
        sqlx::query("INSERT OR IGNORE INTO branches (service_id, name) VALUES (?, ?)")
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
        let rows: Vec<(i64, String, String, String)> = sqlx::query_as(
            "SELECT id, path, method, yaml_content FROM endpoints WHERE branch_id = ?"
        )
        .bind(branch_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;

        Ok(rows
            .into_iter()
            .map(|(id, path, method, yaml_content)| EndpointRecord {
                id: Some(id),
                path,
                method,
                yaml_content,
            })
            .collect())
    }

    async fn insert_endpoint(&self, branch_id: i64, endpoint: &EndpointRecord) -> Result<(), RepositoryError> {
        sqlx::query("INSERT INTO endpoints (branch_id, path, method, yaml_content) VALUES (?, ?, ?, ?)")
            .bind(branch_id)
            .bind(&endpoint.path)
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
        path: &str,
        method: &str,
    ) -> Result<Option<(i64, String)>, RepositoryError> {
        let row: Option<(i64, String)> = sqlx::query_as(
            r#"
            SELECT e.id, e.yaml_content
            FROM endpoints e
            JOIN branches b ON e.branch_id = b.id
            WHERE b.service_id = ? AND b.name = ? AND e.path = ? AND e.method = ?
            "#,
        )
        .bind(service_id)
        .bind(branch_name)
        .bind(path)
        .bind(method)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;

        Ok(row)
    }

    async fn record_dependency(
        &self,
        client_id: i64,
        endpoint_id: Option<i64>,
        service_id: i64,
        branch_name: &str,
        path: &str,
        method: &str,
    ) -> Result<(), RepositoryError> {
        sqlx::query(
            r#"
            INSERT INTO dependencies 
            (client_id, endpoint_id, requested_service_id, requested_branch_name, requested_path, requested_method)
            VALUES (?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(client_id)
        .bind(endpoint_id)
        .bind(service_id)
        .bind(branch_name)
        .bind(path)
        .bind(method)
        .execute(&self.pool)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;

        Ok(())
    }

    async fn get_report(&self, branch: &str) -> Result<DependencyReport, RepositoryError> {
        let mut conn = self.pool.acquire().await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;

        // Dependency graph
        let dependency_rows: Vec<(String, String, String, String)> = sqlx::query_as(
            r#"
            SELECT c.name, s.name, d.requested_path, d.requested_method
            FROM dependencies d
            JOIN clients c ON d.client_id = c.id
            JOIN services s ON d.requested_service_id = s.id
            WHERE d.requested_branch_name = ?
            "#,
        )
        .bind(branch)
        .fetch_all(&mut *conn)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;

        let dependency_graph = dependency_rows
            .into_iter()
            .map(|(client, service, path, method)| DependencyInfo {
                client,
                service,
                path,
                method,
            })
            .collect();

        // Unused endpoints
        let unused_rows: Vec<(String, String, String)> = sqlx::query_as(
            r#"
            SELECT s.name, e.path, e.method
            FROM endpoints e
            JOIN branches b ON e.branch_id = b.id
            JOIN services s ON b.service_id = s.id
            WHERE b.name = ? AND e.id NOT IN (
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
            .map(|(service, path, method)| EndpointInfo {
                service,
                path,
                method,
            })
            .collect();

        // Missing endpoints
        let missing_rows: Vec<(String, String, String, String)> = sqlx::query_as(
            r#"
            SELECT c.name, s.name, d.requested_path, d.requested_method
            FROM dependencies d
            JOIN clients c ON d.client_id = c.id
            JOIN services s ON d.requested_service_id = s.id
            WHERE d.requested_branch_name = ? AND d.endpoint_id IS NULL
            "#,
        )
        .bind(branch)
        .fetch_all(&mut *conn)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;

        let missing_endpoints = missing_rows
            .into_iter()
            .map(|(client, service, path, method)| MissingEndpointInfo {
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
}
