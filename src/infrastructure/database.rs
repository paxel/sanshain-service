use crate::domain::models::*;
use crate::domain::ports::{RepositoryError, SpecRepository};
use crate::infrastructure::sqlite_repository::SqliteSpecRepository;
use crate::infrastructure::postgres_repository::PostgresSpecRepository;

/// Enum-based dispatch to avoid trait object limitations with RPITIT.
#[derive(Clone)]
pub enum DatabaseRepo {
    Sqlite(SqliteSpecRepository),
    Postgres(PostgresSpecRepository),
}

impl DatabaseRepo {
    pub fn backend_name(&self) -> &'static str {
        match self {
            DatabaseRepo::Sqlite(_) => "sqlite",
            DatabaseRepo::Postgres(_) => "postgres",
        }
    }
}

/// Macro to reduce boilerplate: delegates each method to the inner variant.
macro_rules! delegate {
    ($self:ident, $method:ident ( $($arg:expr),* )) => {
        match $self {
            DatabaseRepo::Sqlite(r) => r.$method($($arg),*).await,
            DatabaseRepo::Postgres(r) => r.$method($($arg),*).await,
        }
    };
}

impl SpecRepository for DatabaseRepo {
    async fn ensure_service(&self, name: &str) -> Result<i64, RepositoryError> {
        delegate!(self, ensure_service(name))
    }
    async fn find_service(&self, name: &str) -> Result<Option<i64>, RepositoryError> {
        delegate!(self, find_service(name))
    }
    async fn ensure_branch(&self, service_id: i64, branch_name: &str) -> Result<i64, RepositoryError> {
        delegate!(self, ensure_branch(service_id, branch_name))
    }
    async fn find_branch(&self, service_id: i64, branch_name: &str) -> Result<Option<i64>, RepositoryError> {
        delegate!(self, find_branch(service_id, branch_name))
    }

    async fn get_endpoints_for_branch(&self, branch_id: i64) -> Result<Vec<EndpointRecord>, RepositoryError> {
        delegate!(self, get_endpoints_for_branch(branch_id))
    }

    async fn insert_endpoint(&self, branch_id: i64, endpoint: &EndpointRecord) -> Result<(), RepositoryError> {
        delegate!(self, insert_endpoint(branch_id, endpoint))
    }

    async fn ensure_client(&self, name: &str) -> Result<i64, RepositoryError> {
        delegate!(self, ensure_client(name))
    }

    async fn find_endpoint(&self, service_id: i64, branch_name: &str, path: &str, method: &str) -> Result<Option<(i64, String)>, RepositoryError> {
        delegate!(self, find_endpoint(service_id, branch_name, path, method))
    }

    async fn record_dependency(&self, client_id: i64, endpoint_id: Option<i64>, service_id: i64, branch_name: &str, path: &str, method: &str) -> Result<(), RepositoryError> {
        delegate!(self, record_dependency(client_id, endpoint_id, service_id, branch_name, path, method))
    }

    async fn get_report(&self, branch: &str) -> Result<DependencyReport, RepositoryError> {
        delegate!(self, get_report(branch))
    }

    async fn is_branch_protected(&self, branch_name: &str) -> Result<bool, RepositoryError> {
        delegate!(self, is_branch_protected(branch_name))
    }

    async fn add_protected_branch(&self, pattern: &str) -> Result<(), RepositoryError> {
        delegate!(self, add_protected_branch(pattern))
    }

    async fn remove_protected_branch(&self, pattern: &str) -> Result<bool, RepositoryError> {
        delegate!(self, remove_protected_branch(pattern))
    }

    async fn list_protected_branches(&self) -> Result<Vec<String>, RepositoryError> {
        delegate!(self, list_protected_branches())
    }

    async fn update_endpoint(&self, branch_id: i64, path: &str, method: &str, yaml_content: &str) -> Result<(), RepositoryError> {
        delegate!(self, update_endpoint(branch_id, path, method, yaml_content))
    }

    async fn soft_delete_endpoint(&self, branch_id: i64, path: &str, method: &str) -> Result<(), RepositoryError> {
        delegate!(self, soft_delete_endpoint(branch_id, path, method))
    }

    async fn hard_delete_endpoint(&self, branch_id: i64, path: &str, method: &str) -> Result<(), RepositoryError> {
        delegate!(self, hard_delete_endpoint(branch_id, path, method))
    }

    async fn is_endpoint_deleted(&self, branch_id: i64, path: &str, method: &str) -> Result<bool, RepositoryError> {
        delegate!(self, is_endpoint_deleted(branch_id, path, method))
    }

    async fn delete_service(&self, name: &str) -> Result<bool, RepositoryError> {
        delegate!(self, delete_service(name))
    }

    async fn delete_branch(&self, service_name: &str, branch_name: &str) -> Result<bool, RepositoryError> {
        delegate!(self, delete_branch(service_name, branch_name))
    }

    async fn delete_client(&self, name: &str) -> Result<bool, RepositoryError> {
        delegate!(self, delete_client(name))
    }

    async fn list_services(&self) -> Result<Vec<String>, RepositoryError> {
        delegate!(self, list_services())
    }

    async fn list_branches(&self, service_name: &str) -> Result<Vec<String>, RepositoryError> {
        delegate!(self, list_branches(service_name))
    }

    async fn list_clients(&self) -> Result<Vec<String>, RepositoryError> {
        delegate!(self, list_clients())
    }

    async fn list_client_branches(&self, client_name: &str) -> Result<Vec<String>, RepositoryError> {
        delegate!(self, list_client_branches(client_name))
    }

    async fn list_client_endpoints(&self, client_name: &str, branch: &str) -> Result<Vec<ClientEndpointInfo>, RepositoryError> {
        delegate!(self, list_client_endpoints(client_name, branch))
    }

    async fn user_count(&self) -> Result<i64, RepositoryError> {
        delegate!(self, user_count())
    }

    async fn find_user(&self, username: &str) -> Result<Option<User>, RepositoryError> {
        delegate!(self, find_user(username))
    }

    async fn create_user(&self, username: &str, password_hash: &str, is_admin: bool, approved: bool) -> Result<User, RepositoryError> {
        delegate!(self, create_user(username, password_hash, is_admin, approved))
    }

    async fn update_password(&self, user_id: i64, new_hash: &str) -> Result<(), RepositoryError> {
        delegate!(self, update_password(user_id, new_hash))
    }

    async fn list_users(&self) -> Result<Vec<User>, RepositoryError> {
        delegate!(self, list_users())
    }

    async fn approve_user(&self, user_id: i64) -> Result<bool, RepositoryError> {
        delegate!(self, approve_user(user_id))
    }

    async fn delete_user(&self, user_id: i64) -> Result<bool, RepositoryError> {
        delegate!(self, delete_user(user_id))
    }

    async fn create_session(&self, user_id: i64, expires_at: &str) -> Result<Session, RepositoryError> {
        delegate!(self, create_session(user_id, expires_at))
    }

    async fn validate_session(&self, token: &str) -> Result<Option<(User, Session)>, RepositoryError> {
        delegate!(self, validate_session(token))
    }

    async fn delete_session(&self, token: &str) -> Result<(), RepositoryError> {
        delegate!(self, delete_session(token))
    }

    async fn get_setting(&self, key: &str) -> Result<Option<String>, RepositoryError> {
        delegate!(self, get_setting(key))
    }

    async fn set_setting(&self, key: &str, value: &str) -> Result<(), RepositoryError> {
        delegate!(self, set_setting(key, value))
    }

    async fn create_api_token(&self, id: &str, user_id: i64, name: &str, token_hash: &str, created_at: &str, expires_at: &str) -> Result<(), RepositoryError> {
        delegate!(self, create_api_token(id, user_id, name, token_hash, created_at, expires_at))
    }

    async fn list_api_tokens(&self, user_id: i64) -> Result<Vec<ApiToken>, RepositoryError> {
        delegate!(self, list_api_tokens(user_id))
    }

    async fn delete_api_token(&self, token_id: &str, user_id: i64) -> Result<bool, RepositoryError> {
        delegate!(self, delete_api_token(token_id, user_id))
    }

    async fn validate_api_token(&self, token_hash: &str) -> Result<Option<User>, RepositoryError> {
        delegate!(self, validate_api_token(token_hash))
    }

    async fn delete_stale_branches(&self, cutoff_iso: &str) -> Result<u64, RepositoryError> {
        delegate!(self, delete_stale_branches(cutoff_iso))
    }

    async fn delete_stale_dependencies(&self, cutoff_iso: &str) -> Result<u64, RepositoryError> {
        delegate!(self, delete_stale_dependencies(cutoff_iso))
    }

    async fn get_endpoint_id(&self, branch_id: i64, path: &str, method: &str) -> Result<Option<i64>, RepositoryError> {
        delegate!(self, get_endpoint_id(branch_id, path, method))
    }

    async fn insert_endpoint_version(&self, endpoint_id: i64, version: i32, yaml_content: &str, diff: Option<&str>, created_at: &str) -> Result<(), RepositoryError> {
        delegate!(self, insert_endpoint_version(endpoint_id, version, yaml_content, diff, created_at))
    }

    async fn get_latest_endpoint_version(&self, endpoint_id: i64) -> Result<i32, RepositoryError> {
        delegate!(self, get_latest_endpoint_version(endpoint_id))
    }

    async fn get_endpoint_versions(&self, endpoint_id: i64) -> Result<Vec<EndpointVersion>, RepositoryError> {
        delegate!(self, get_endpoint_versions(endpoint_id))
    }
}
