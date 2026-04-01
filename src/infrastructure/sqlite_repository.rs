use sqlx::SqlitePool;

use crate::domain::models::*;
use crate::domain::ports::{RepositoryError, SpecRepository};

fn now_iso() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let secs_per_day = 86400u64;
    let days = secs / secs_per_day;
    let tod = secs % secs_per_day;
    let h = tod / 3600;
    let m = (tod % 3600) / 60;
    let s = tod % 60;
    // Howard Hinnant's algorithm
    let z = days + 719468;
    let era = z / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let mo = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if mo <= 2 { y + 1 } else { y };
    format!("{:04}-{:02}-{:02}T{:02}:{:02}:{:02}", y, mo, d, h, m, s)
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
        sqlx::migrate!("src/infrastructure/migrations/sqlite")
            .run(&self.pool)
            .await
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
        let rows: Vec<(i64, String, String, String)> = sqlx::query_as(
            "SELECT id, path, method, yaml_content FROM endpoints WHERE branch_id = ? AND deleted = FALSE"
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
            WHERE b.service_id = ? AND b.name = ? AND e.path = ? AND e.method = ? AND e.deleted = FALSE
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
        let now = now_iso();
        sqlx::query(
            r#"
            INSERT INTO dependencies 
            (client_id, endpoint_id, requested_service_id, requested_branch_name, requested_path, requested_method, last_seen_at)
            VALUES (?, ?, ?, ?, ?, ?, ?)
            ON CONFLICT(client_id, endpoint_id, requested_service_id, requested_branch_name, requested_path, requested_method)
            DO UPDATE SET last_seen_at = excluded.last_seen_at
            "#,
        )
        .bind(client_id)
        .bind(endpoint_id)
        .bind(service_id)
        .bind(branch_name)
        .bind(path)
        .bind(method)
        .bind(&now)
        .execute(&self.pool)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;

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

    async fn update_endpoint(&self, branch_id: i64, path: &str, method: &str, yaml_content: &str) -> Result<(), RepositoryError> {
        sqlx::query("UPDATE endpoints SET yaml_content = ? WHERE branch_id = ? AND path = ? AND method = ?")
            .bind(yaml_content)
            .bind(branch_id)
            .bind(path)
            .bind(method)
            .execute(&self.pool)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(())
    }

    async fn soft_delete_endpoint(&self, branch_id: i64, path: &str, method: &str) -> Result<(), RepositoryError> {
        sqlx::query("UPDATE endpoints SET deleted = TRUE WHERE branch_id = ? AND path = ? AND method = ?")
            .bind(branch_id)
            .bind(path)
            .bind(method)
            .execute(&self.pool)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(())
    }

    async fn hard_delete_endpoint(&self, branch_id: i64, path: &str, method: &str) -> Result<(), RepositoryError> {
        sqlx::query("DELETE FROM endpoints WHERE branch_id = ? AND path = ? AND method = ?")
            .bind(branch_id)
            .bind(path)
            .bind(method)
            .execute(&self.pool)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(())
    }

    async fn is_endpoint_deleted(&self, branch_id: i64, path: &str, method: &str) -> Result<bool, RepositoryError> {
        let row: (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM endpoints WHERE branch_id = ? AND path = ? AND method = ? AND deleted = TRUE"
        )
        .bind(branch_id)
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
        let rows: Vec<(String,)> = sqlx::query_as("SELECT name FROM services ORDER BY name")
            .fetch_all(&self.pool)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;
        Ok(rows.into_iter().map(|r| r.0).collect())
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
        let rows: Vec<(String,)> = sqlx::query_as("SELECT name FROM clients ORDER BY name")
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
        let rows: Vec<(String, String, String, String, Option<String>)> = sqlx::query_as(
            "SELECT s.name, d.requested_branch_name, d.requested_path, d.requested_method, e.yaml_content \
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
        Ok(rows.into_iter().map(|(service, branch, path, method, yaml_content)| ClientEndpointInfo {
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
        use rand::Rng;
        let mut token_bytes = [0u8; 32];
        rand::rng().fill(&mut token_bytes);
        let token = hex::encode(token_bytes);

        sqlx::query("INSERT INTO sessions (token, user_id, expires_at) VALUES (?, ?, ?)")
            .bind(&token)
            .bind(user_id)
            .bind(expires_at)
            .execute(&self.pool)
            .await
            .map_err(|e| RepositoryError::Internal(e.to_string()))?;

        Ok(Session {
            token,
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
        let rows: Vec<(String, i64, String, String, String, String, Option<String>)> = sqlx::query_as(
            "SELECT id, user_id, name, token_hash, created_at, expires_at, last_used_at FROM api_tokens WHERE user_id = ? ORDER BY created_at DESC"
        )
        .bind(user_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| RepositoryError::Internal(e.to_string()))?;

        Ok(rows.into_iter().map(|(id, user_id, name, token_hash, created_at, expires_at, last_used_at)| ApiToken {
            id, user_id, name, token_hash, created_at, expires_at, last_used_at,
        }).collect())
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
}
