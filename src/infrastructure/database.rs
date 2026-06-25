use crate::domain::models::*;
use crate::domain::ports::{
    RecordDependencyParams, RepositoryError, SpecRepository, UpdateEndpointParams,
};
use crate::infrastructure::postgres_repository::PostgresSpecRepository;
use crate::infrastructure::sqlite_repository::SqliteSpecRepository;
use std::collections::HashMap;

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
    async fn get_spec_version(
        &self,
        service_id: i64,
        branch_id: i64,
    ) -> Result<Option<(SemVer, String)>, RepositoryError> {
        delegate!(self, get_spec_version(service_id, branch_id))
    }

    async fn increment_spec_version(
        &self,
        service_id: i64,
        branch_id: i64,
        content_hash: &str,
        impact: Impact,
    ) -> Result<SemVer, RepositoryError> {
        delegate!(
            self,
            increment_spec_version(service_id, branch_id, content_hash, impact)
        )
    }

    async fn ensure_service(&self, name: &str) -> Result<i64, RepositoryError> {
        delegate!(self, ensure_service(name))
    }
    async fn find_service(&self, name: &str) -> Result<Option<i64>, RepositoryError> {
        delegate!(self, find_service(name))
    }
    async fn get_service_name_by_id(
        &self,
        service_id: i64,
    ) -> Result<Option<String>, RepositoryError> {
        delegate!(self, get_service_name_by_id(service_id))
    }
    async fn ensure_branch(
        &self,
        service_id: i64,
        branch_name: &str,
    ) -> Result<i64, RepositoryError> {
        delegate!(self, ensure_branch(service_id, branch_name))
    }
    async fn find_branch(
        &self,
        service_id: i64,
        branch_name: &str,
    ) -> Result<Option<i64>, RepositoryError> {
        delegate!(self, find_branch(service_id, branch_name))
    }

    async fn get_endpoints_for_branch(
        &self,
        branch_id: i64,
    ) -> Result<Vec<EndpointRecord>, RepositoryError> {
        delegate!(self, get_endpoints_for_branch(branch_id))
    }

    async fn insert_endpoint(
        &self,
        branch_id: i64,
        endpoint: &EndpointRecord,
    ) -> Result<(), RepositoryError> {
        delegate!(self, insert_endpoint(branch_id, endpoint))
    }

    async fn reset_branch_history(
        &self,
        service_name: &str,
        branch_name: &str,
    ) -> Result<bool, RepositoryError> {
        delegate!(self, reset_branch_history(service_name, branch_name))
    }

    async fn ensure_client(&self, name: &str) -> Result<i64, RepositoryError> {
        delegate!(self, ensure_client(name))
    }

    async fn find_endpoint(
        &self,
        service_id: i64,
        branch_name: &str,
        api_type: ApiType,
        path: &str,
        method: &str,
    ) -> Result<Option<(i64, String, bool, bool)>, RepositoryError> {
        delegate!(
            self,
            find_endpoint(service_id, branch_name, api_type, path, method)
        )
    }

    async fn find_endpoints_bulk(
        &self,
        service_id: i64,
        branch_name: &str,
        api_type: ApiType,
        endpoints: &[(String, String)],
    ) -> Result<HashMap<(String, String), (i64, String, bool, bool)>, RepositoryError> {
        delegate!(
            self,
            find_endpoints_bulk(service_id, branch_name, api_type, endpoints)
        )
    }

    async fn record_dependency(
        &self,
        params: RecordDependencyParams<'_>,
    ) -> Result<(), RepositoryError> {
        delegate!(self, record_dependency(params))
    }

    async fn record_dependencies_bulk(
        &self,
        params: Vec<RecordDependencyParams<'_>>,
    ) -> Result<(), RepositoryError> {
        delegate!(self, record_dependencies_bulk(params))
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

    async fn update_endpoint(
        &self,
        params: UpdateEndpointParams<'_>,
    ) -> Result<(), RepositoryError> {
        delegate!(self, update_endpoint(params))
    }

    async fn soft_delete_endpoint(
        &self,
        branch_id: i64,
        api_type: ApiType,
        path: &str,
        method: &str,
    ) -> Result<(), RepositoryError> {
        delegate!(
            self,
            soft_delete_endpoint(branch_id, api_type, path, method)
        )
    }

    async fn hard_delete_endpoint(
        &self,
        branch_id: i64,
        api_type: ApiType,
        path: &str,
        method: &str,
    ) -> Result<(), RepositoryError> {
        delegate!(
            self,
            hard_delete_endpoint(branch_id, api_type, path, method)
        )
    }

    async fn is_endpoint_deleted(
        &self,
        branch_id: i64,
        api_type: ApiType,
        path: &str,
        method: &str,
    ) -> Result<bool, RepositoryError> {
        delegate!(self, is_endpoint_deleted(branch_id, api_type, path, method))
    }

    async fn delete_all_services(&self) -> Result<u64, RepositoryError> {
        delegate!(self, delete_all_services())
    }

    async fn delete_all_clients(&self) -> Result<u64, RepositoryError> {
        delegate!(self, delete_all_clients())
    }

    async fn delete_all_non_admin_users(&self) -> Result<u64, RepositoryError> {
        delegate!(self, delete_all_non_admin_users())
    }

    async fn nuke_database(&self, keep_user_id: Option<i64>) -> Result<(), RepositoryError> {
        delegate!(self, nuke_database(keep_user_id))
    }

    async fn delete_service(&self, name: &str) -> Result<bool, RepositoryError> {
        delegate!(self, delete_service(name))
    }

    async fn delete_branch(
        &self,
        service_name: &str,
        branch_name: &str,
    ) -> Result<bool, RepositoryError> {
        delegate!(self, delete_branch(service_name, branch_name))
    }

    async fn delete_client(&self, name: &str) -> Result<bool, RepositoryError> {
        delegate!(self, delete_client(name))
    }

    async fn list_services(&self) -> Result<Vec<String>, RepositoryError> {
        delegate!(self, list_services())
    }

    async fn list_services_detailed(&self) -> Result<Vec<ServiceSummary>, RepositoryError> {
        delegate!(self, list_services_detailed())
    }

    async fn set_fallback_branch(
        &self,
        service_name: &str,
        branch: Option<&str>,
    ) -> Result<(), RepositoryError> {
        delegate!(self, set_fallback_branch(service_name, branch))
    }

    async fn update_service_metadata(
        &self,
        service_name: &str,
        icon: Option<&str>,
        domain: Option<&str>,
    ) -> Result<(), RepositoryError> {
        delegate!(self, update_service_metadata(service_name, icon, domain))
    }

    async fn get_fallback_branch(
        &self,
        service_name: &str,
    ) -> Result<Option<String>, RepositoryError> {
        delegate!(self, get_fallback_branch(service_name))
    }

    async fn list_branches(&self, service_name: &str) -> Result<Vec<String>, RepositoryError> {
        delegate!(self, list_branches(service_name))
    }

    async fn list_all_branches(&self) -> Result<Vec<String>, RepositoryError> {
        delegate!(self, list_all_branches())
    }

    async fn list_clients(&self) -> Result<Vec<String>, RepositoryError> {
        delegate!(self, list_clients())
    }

    async fn list_client_branches(
        &self,
        client_name: &str,
    ) -> Result<Vec<String>, RepositoryError> {
        delegate!(self, list_client_branches(client_name))
    }

    async fn list_client_endpoints(
        &self,
        client_name: &str,
        branch: &str,
    ) -> Result<Vec<ClientEndpointInfo>, RepositoryError> {
        delegate!(self, list_client_endpoints(client_name, branch))
    }

    async fn user_count(&self) -> Result<i64, RepositoryError> {
        delegate!(self, user_count())
    }

    async fn find_user(&self, username: &str) -> Result<Option<User>, RepositoryError> {
        delegate!(self, find_user(username))
    }

    async fn create_user(
        &self,
        username: &str,
        password_hash: &str,
        is_admin: bool,
        approved: bool,
    ) -> Result<User, RepositoryError> {
        delegate!(
            self,
            create_user(username, password_hash, is_admin, approved)
        )
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

    async fn create_session(
        &self,
        user_id: i64,
        expires_at: &str,
    ) -> Result<Session, RepositoryError> {
        delegate!(self, create_session(user_id, expires_at))
    }

    async fn create_session_with_token(
        &self,
        user_id: i64,
        token: &str,
        expires_at: &str,
    ) -> Result<Session, RepositoryError> {
        delegate!(self, create_session_with_token(user_id, token, expires_at))
    }

    async fn validate_session(
        &self,
        token: &str,
    ) -> Result<Option<(User, Session)>, RepositoryError> {
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

    async fn create_api_token(
        &self,
        id: &str,
        user_id: i64,
        name: &str,
        token_hash: &str,
        created_at: &str,
        expires_at: &str,
    ) -> Result<(), RepositoryError> {
        delegate!(
            self,
            create_api_token(id, user_id, name, token_hash, created_at, expires_at)
        )
    }

    async fn list_api_tokens(&self, user_id: i64) -> Result<Vec<ApiToken>, RepositoryError> {
        delegate!(self, list_api_tokens(user_id))
    }

    async fn delete_api_token(
        &self,
        token_id: &str,
        user_id: i64,
    ) -> Result<bool, RepositoryError> {
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

    async fn get_endpoint_id(
        &self,
        branch_id: i64,
        api_type: ApiType,
        path: &str,
        method: &str,
    ) -> Result<Option<i64>, RepositoryError> {
        delegate!(self, get_endpoint_id(branch_id, api_type, path, method))
    }

    async fn insert_endpoint_version(
        &self,
        endpoint_id: i64,
        version: i32,
        yaml_content: &str,
        diff: Option<&str>,
        created_at: &str,
    ) -> Result<(), RepositoryError> {
        delegate!(
            self,
            insert_endpoint_version(endpoint_id, version, yaml_content, diff, created_at)
        )
    }

    async fn get_latest_endpoint_version(&self, endpoint_id: i64) -> Result<i32, RepositoryError> {
        delegate!(self, get_latest_endpoint_version(endpoint_id))
    }

    async fn get_endpoint_versions(
        &self,
        endpoint_id: i64,
    ) -> Result<Vec<EndpointVersion>, RepositoryError> {
        delegate!(self, get_endpoint_versions(endpoint_id))
    }

    async fn get_global_endpoint_versions(
        &self,
        limit: u32,
    ) -> Result<Vec<EndpointVersion>, RepositoryError> {
        delegate!(self, get_global_endpoint_versions(limit))
    }

    async fn apply_spec_changes(
        &self,
        branch_id: i64,
        changes: Vec<SpecChange>,
        is_protected: bool,
        username: Option<&str>,
        source_branch: Option<&str>,
    ) -> Result<(), RepositoryError> {
        delegate!(
            self,
            apply_spec_changes(branch_id, changes, is_protected, username, source_branch)
        )
    }

    async fn add_service_tags(
        &self,
        service_id: i64,
        tags: &[String],
    ) -> Result<(), RepositoryError> {
        delegate!(self, add_service_tags(service_id, tags))
    }

    async fn get_all_service_tags(&self) -> Result<HashMap<String, Vec<String>>, RepositoryError> {
        delegate!(self, get_all_service_tags())
    }

    async fn get_shared_contract(
        &self,
        branch_name: &str,
        service_id: i64,
        api_type: ApiType,
        path: &str,
        method: &str,
    ) -> Result<Option<SharedContract>, RepositoryError> {
        delegate!(
            self,
            get_shared_contract(branch_name, service_id, api_type, path, method)
        )
    }

    async fn upsert_shared_contract(
        &self,
        contract: SharedContract,
    ) -> Result<(), RepositoryError> {
        delegate!(self, upsert_shared_contract(contract))
    }

    async fn insert_audit_log(
        &self,
        username: &str,
        action: &str,
        details: &str,
    ) -> Result<(), RepositoryError> {
        delegate!(self, insert_audit_log(username, action, details))
    }

    async fn get_recent_audit_logs(
        &self,
        limit: u32,
    ) -> Result<Vec<AuditLogEntry>, RepositoryError> {
        delegate!(self, get_recent_audit_logs(limit))
    }

    async fn get_user_favorites(
        &self,
        user_id: i64,
        item_type: &str,
    ) -> Result<Vec<String>, RepositoryError> {
        delegate!(self, get_user_favorites(user_id, item_type))
    }

    async fn add_user_favorite(
        &self,
        user_id: i64,
        item_type: &str,
        item_name: &str,
    ) -> Result<(), RepositoryError> {
        delegate!(self, add_user_favorite(user_id, item_type, item_name))
    }

    async fn remove_user_favorite(
        &self,
        user_id: i64,
        item_type: &str,
        item_name: &str,
    ) -> Result<(), RepositoryError> {
        delegate!(self, remove_user_favorite(user_id, item_type, item_name))
    }

    async fn list_branches_with_metadata(&self) -> Result<Vec<BranchMetadata>, RepositoryError> {
        delegate!(self, list_branches_with_metadata())
    }
}
