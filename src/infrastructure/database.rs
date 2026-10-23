use crate::domain::models::*;
use crate::domain::ports::{
    EndpointMap, NewApiToken, NewAuditLog, NewWebhook, RecordDependencyParams,
    RecordTrunkPinParams, RepositoryError, RewriteSpecText, SpecRepository, UpsertSpecVersion,
};
use crate::infrastructure::postgres_repository::PostgresSpecRepository;
use crate::infrastructure::sqlite_repository::SqliteSpecRepository;
use std::collections::{BTreeMap, HashMap};

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
    async fn ping(&self) -> Result<(), RepositoryError> {
        delegate!(self, ping())
    }

    async fn upsert_spec_version(
        &self,
        params: UpsertSpecVersion<'_>,
    ) -> Result<i64, RepositoryError> {
        delegate!(self, upsert_spec_version(params))
    }

    async fn rewrite_spec_text(&self, params: RewriteSpecText<'_>) -> Result<(), RepositoryError> {
        delegate!(self, rewrite_spec_text(params))
    }

    async fn find_spec_version(
        &self,
        service_id: i64,
        api_type: ApiType,
        version: SemVer,
    ) -> Result<Option<SpecVersionMeta>, RepositoryError> {
        delegate!(self, find_spec_version(service_id, api_type, version))
    }

    async fn list_spec_versions(
        &self,
        service_id: i64,
    ) -> Result<Vec<SpecVersionMeta>, RepositoryError> {
        delegate!(self, list_spec_versions(service_id))
    }

    async fn list_all_spec_versions(
        &self,
    ) -> Result<Vec<(String, SpecVersionMeta, i64)>, RepositoryError> {
        delegate!(self, list_all_spec_versions())
    }

    async fn get_spec_content(
        &self,
        spec_version_id: i64,
    ) -> Result<Option<String>, RepositoryError> {
        delegate!(self, get_spec_content(spec_version_id))
    }

    async fn delete_spec_version(&self, spec_version_id: i64) -> Result<bool, RepositoryError> {
        delegate!(self, delete_spec_version(spec_version_id))
    }

    async fn touch_spec_version_required(
        &self,
        spec_version_id: i64,
        now_iso: &str,
    ) -> Result<(), RepositoryError> {
        delegate!(self, touch_spec_version_required(spec_version_id, now_iso))
    }

    async fn touch_spec_version_provided(
        &self,
        spec_version_id: i64,
        now_iso: &str,
    ) -> Result<(), RepositoryError> {
        delegate!(self, touch_spec_version_provided(spec_version_id, now_iso))
    }

    async fn touch_spec_version_trunk(
        &self,
        spec_version_id: i64,
        now_iso: &str,
    ) -> Result<(), RepositoryError> {
        delegate!(self, touch_spec_version_trunk(spec_version_id, now_iso))
    }

    async fn set_version_origin(
        &self,
        spec_version_id: i64,
        origin: &VersionOrigin,
        now_iso: &str,
    ) -> Result<(), RepositoryError> {
        delegate!(self, set_version_origin(spec_version_id, origin, now_iso))
    }

    async fn get_version_origin(
        &self,
        spec_version_id: i64,
    ) -> Result<Option<VersionOrigin>, RepositoryError> {
        delegate!(self, get_version_origin(spec_version_id))
    }

    async fn list_version_origins(&self) -> Result<BTreeMap<i64, VersionOrigin>, RepositoryError> {
        delegate!(self, list_version_origins())
    }

    async fn list_trunk_pins_at(&self, at: &str) -> Result<Vec<TrunkPinInfo>, RepositoryError> {
        delegate!(self, list_trunk_pins_at(at))
    }

    async fn list_graph_change_dates(
        &self,
        branch_id: Option<i64>,
    ) -> Result<Vec<String>, RepositoryError> {
        delegate!(self, list_graph_change_dates(branch_id))
    }

    async fn list_branch_memberships_for_service(
        &self,
        service_id: i64,
    ) -> Result<Vec<BranchMembership>, RepositoryError> {
        delegate!(self, list_branch_memberships_for_service(service_id))
    }

    async fn list_branches_referencing(
        &self,
        service_id: i64,
        api_type: ApiType,
        version: SemVer,
    ) -> Result<Vec<String>, RepositoryError> {
        delegate!(
            self,
            list_branches_referencing(service_id, api_type, version)
        )
    }

    async fn close_expired_trunk_data(
        &self,
        cutoff_iso: &str,
        now_iso: &str,
    ) -> Result<u64, RepositoryError> {
        delegate!(self, close_expired_trunk_data(cutoff_iso, now_iso))
    }

    async fn rename_branch(&self, branch_id: i64, new_name: &str) -> Result<(), RepositoryError> {
        delegate!(self, rename_branch(branch_id, new_name))
    }

    async fn delete_branch(&self, branch_id: i64) -> Result<(), RepositoryError> {
        delegate!(self, delete_branch(branch_id))
    }

    async fn record_branch_pins(
        &self,
        branch_id: i64,
        pins: Vec<RecordTrunkPinParams<'_>>,
    ) -> Result<(), RepositoryError> {
        delegate!(self, record_branch_pins(branch_id, pins))
    }

    async fn record_branch_member_version(
        &self,
        branch_id: i64,
        service_id: i64,
        api_type: ApiType,
        version: SemVer,
        now_iso: &str,
    ) -> Result<(), RepositoryError> {
        delegate!(
            self,
            record_branch_member_version(branch_id, service_id, api_type, version, now_iso)
        )
    }

    async fn record_trunk_pins(
        &self,
        pins: Vec<RecordTrunkPinParams<'_>>,
    ) -> Result<(), RepositoryError> {
        delegate!(self, record_trunk_pins(pins))
    }

    async fn list_current_trunk_pins(&self) -> Result<Vec<TrunkPinInfo>, RepositoryError> {
        delegate!(self, list_current_trunk_pins())
    }

    async fn insert_branch(
        &self,
        name: &str,
        created_at: &str,
        created_by: &str,
        source: &str,
        as_of: &str,
        outbox: Vec<NewWebhookEvent>,
    ) -> Result<i64, RepositoryError> {
        delegate!(
            self,
            insert_branch(name, created_at, created_by, source, as_of, outbox)
        )
    }

    async fn find_branch(&self, name: &str) -> Result<Option<BranchInfo>, RepositoryError> {
        delegate!(self, find_branch(name))
    }

    async fn list_branches(&self) -> Result<Vec<BranchInfo>, RepositoryError> {
        delegate!(self, list_branches())
    }

    async fn copy_trunk_graph_to_branch(
        &self,
        branch_id: i64,
        as_of: &str,
        now_iso: &str,
    ) -> Result<(), RepositoryError> {
        delegate!(self, copy_trunk_graph_to_branch(branch_id, as_of, now_iso))
    }

    async fn copy_branch_graph_to_branch(
        &self,
        target_branch_id: i64,
        source_branch_id: i64,
        as_of: &str,
        now_iso: &str,
    ) -> Result<(), RepositoryError> {
        delegate!(
            self,
            copy_branch_graph_to_branch(target_branch_id, source_branch_id, as_of, now_iso)
        )
    }

    async fn list_branch_pins(
        &self,
        branch_id: i64,
        at: Option<&str>,
    ) -> Result<Vec<TrunkPinInfo>, RepositoryError> {
        delegate!(self, list_branch_pins(branch_id, at))
    }

    async fn delete_expired_snapshots(&self, cutoff_iso: &str) -> Result<u64, RepositoryError> {
        delegate!(self, delete_expired_snapshots(cutoff_iso))
    }

    async fn list_version_dependents(
        &self,
        spec_version_id: i64,
    ) -> Result<Vec<String>, RepositoryError> {
        delegate!(self, list_version_dependents(spec_version_id))
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

    async fn get_endpoints_for_version(
        &self,
        spec_version_id: i64,
    ) -> Result<Vec<EndpointRecord>, RepositoryError> {
        delegate!(self, get_endpoints_for_version(spec_version_id))
    }

    async fn ensure_client(&self, name: &str) -> Result<i64, RepositoryError> {
        delegate!(self, ensure_client(name))
    }

    async fn find_endpoint(
        &self,
        spec_version_id: i64,
        api_type: ApiType,
        path: &str,
        method: &str,
    ) -> Result<Option<(i64, String, bool)>, RepositoryError> {
        delegate!(self, find_endpoint(spec_version_id, api_type, path, method))
    }

    async fn find_endpoints_bulk(
        &self,
        spec_version_id: i64,
        api_type: ApiType,
        endpoints: &[(String, String)],
    ) -> Result<EndpointMap, RepositoryError> {
        delegate!(
            self,
            find_endpoints_bulk(spec_version_id, api_type, endpoints)
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
        outbox: Vec<NewWebhookEvent>,
    ) -> Result<(), RepositoryError> {
        delegate!(self, record_dependencies_bulk(params, outbox))
    }

    async fn get_report(&self) -> Result<DependencyReport, RepositoryError> {
        delegate!(self, get_report())
    }

    async fn delete_all_services(&self) -> Result<u64, RepositoryError> {
        delegate!(self, delete_all_services())
    }

    async fn delete_all_clients(&self) -> Result<u64, RepositoryError> {
        delegate!(self, delete_all_clients())
    }

    async fn delete_all_non_admin_users(
        &self,
        spare_usernames: &[String],
    ) -> Result<u64, RepositoryError> {
        delegate!(self, delete_all_non_admin_users(spare_usernames))
    }

    async fn nuke_database(&self, keep_user_id: Option<i64>) -> Result<(), RepositoryError> {
        delegate!(self, nuke_database(keep_user_id))
    }

    async fn delete_producer(&self, name: &str) -> Result<bool, RepositoryError> {
        delegate!(self, delete_producer(name))
    }

    async fn delete_consumer(&self, name: &str) -> Result<bool, RepositoryError> {
        delegate!(self, delete_consumer(name))
    }

    async fn list_producers(&self) -> Result<Vec<String>, RepositoryError> {
        delegate!(self, list_producers())
    }

    async fn list_producers_detailed(&self) -> Result<Vec<ProducerSummary>, RepositoryError> {
        delegate!(self, list_producers_detailed())
    }

    async fn update_producer_metadata(
        &self,
        service_name: &str,
        icon: Option<&str>,
        domain: Option<&str>,
    ) -> Result<(), RepositoryError> {
        delegate!(self, update_producer_metadata(service_name, icon, domain))
    }

    async fn set_producer_external(
        &self,
        service_name: &str,
        external: bool,
    ) -> Result<bool, RepositoryError> {
        delegate!(self, set_producer_external(service_name, external))
    }

    async fn list_consumers(&self) -> Result<Vec<String>, RepositoryError> {
        delegate!(self, list_consumers())
    }

    async fn list_consumer_endpoints(
        &self,
        client_name: &str,
    ) -> Result<Vec<ConsumerEndpointInfo>, RepositoryError> {
        delegate!(self, list_consumer_endpoints(client_name))
    }

    async fn search_endpoints(
        &self,
        search: &EndpointSearch,
    ) -> Result<Vec<EndpointSearchHit>, RepositoryError> {
        delegate!(self, search_endpoints(search))
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
        approved: bool,
    ) -> Result<User, RepositoryError> {
        delegate!(self, create_user(username, password_hash, approved))
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

    async fn delete_expired_credentials(&self, now_iso: &str) -> Result<u64, RepositoryError> {
        delegate!(self, delete_expired_credentials(now_iso))
    }

    async fn delete_session(&self, token: &str) -> Result<(), RepositoryError> {
        delegate!(self, delete_session(token))
    }

    async fn record_user_sign_in(
        &self,
        user_id: i64,
        method: SignInMethod,
    ) -> Result<(), RepositoryError> {
        delegate!(self, record_user_sign_in(user_id, method))
    }

    async fn set_display_name(
        &self,
        user_id: i64,
        display_name: Option<&str>,
    ) -> Result<(), RepositoryError> {
        delegate!(self, set_display_name(user_id, display_name))
    }

    async fn set_avatar_hash(
        &self,
        user_id: i64,
        avatar_hash: Option<&str>,
    ) -> Result<(), RepositoryError> {
        delegate!(self, set_avatar_hash(user_id, avatar_hash))
    }

    async fn set_own_avatar_hash(
        &self,
        user_id: i64,
        own_avatar_hash: Option<&str>,
    ) -> Result<(), RepositoryError> {
        delegate!(self, set_own_avatar_hash(user_id, own_avatar_hash))
    }

    async fn set_directory_id(
        &self,
        user_id: i64,
        directory_id: &str,
    ) -> Result<(), RepositoryError> {
        delegate!(self, set_directory_id(user_id, directory_id))
    }

    async fn find_user_by_directory_id(
        &self,
        directory_id: &str,
    ) -> Result<Option<User>, RepositoryError> {
        delegate!(self, find_user_by_directory_id(directory_id))
    }

    async fn rename_user(&self, user_id: i64, username: &str) -> Result<(), RepositoryError> {
        delegate!(self, rename_user(user_id, username))
    }

    async fn merge_user_into(&self, from: i64, into: i64) -> Result<(), RepositoryError> {
        delegate!(self, merge_user_into(from, into))
    }

    async fn list_user_profiles(&self) -> Result<Vec<UserProfile>, RepositoryError> {
        delegate!(self, list_user_profiles())
    }

    async fn get_user_profile(&self, user_id: i64) -> Result<Option<UserProfile>, RepositoryError> {
        delegate!(self, get_user_profile(user_id))
    }

    async fn set_session_sign_in(
        &self,
        token: &str,
        method: SignInMethod,
    ) -> Result<(), RepositoryError> {
        delegate!(self, set_session_sign_in(token, method))
    }

    async fn session_sign_in(&self, token: &str) -> Result<Option<SignInMethod>, RepositoryError> {
        delegate!(self, session_sign_in(token))
    }

    async fn get_setting(&self, key: &str) -> Result<Option<String>, RepositoryError> {
        delegate!(self, get_setting(key))
    }

    async fn set_setting(&self, key: &str, value: &str) -> Result<(), RepositoryError> {
        delegate!(self, set_setting(key, value))
    }

    async fn create_api_token(&self, token: NewApiToken<'_>) -> Result<(), RepositoryError> {
        delegate!(self, create_api_token(token))
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

    async fn validate_api_token(
        &self,
        token_hash: &str,
    ) -> Result<Option<ApiTokenIdentity>, RepositoryError> {
        delegate!(self, validate_api_token(token_hash))
    }

    async fn touch_api_token(&self, token_id: &str, used_at: &str) -> Result<(), RepositoryError> {
        delegate!(self, touch_api_token(token_id, used_at))
    }

    async fn delete_stale_dependencies(&self, cutoff_iso: &str) -> Result<u64, RepositoryError> {
        delegate!(self, delete_stale_dependencies(cutoff_iso))
    }

    async fn grant_user_role(&self, user_id: i64, role: &str) -> Result<(), RepositoryError> {
        delegate!(self, grant_user_role(user_id, role))
    }

    async fn revoke_user_role(&self, user_id: i64, role: &str) -> Result<bool, RepositoryError> {
        delegate!(self, revoke_user_role(user_id, role))
    }

    async fn list_user_roles(&self, user_id: i64) -> Result<Vec<String>, RepositoryError> {
        delegate!(self, list_user_roles(user_id))
    }

    async fn effective_stored_roles(&self, user_id: i64) -> Result<Vec<String>, RepositoryError> {
        delegate!(self, effective_stored_roles(user_id))
    }

    async fn create_group(
        &self,
        name: &str,
        source: GroupSource,
    ) -> Result<Group, RepositoryError> {
        delegate!(self, create_group(name, source))
    }

    async fn rename_group(&self, group_id: i64, name: &str) -> Result<bool, RepositoryError> {
        delegate!(self, rename_group(group_id, name))
    }

    async fn delete_group(&self, group_id: i64) -> Result<bool, RepositoryError> {
        delegate!(self, delete_group(group_id))
    }

    async fn list_groups(&self) -> Result<Vec<Group>, RepositoryError> {
        delegate!(self, list_groups())
    }

    async fn set_group_roles(
        &self,
        group_id: i64,
        roles: &[String],
    ) -> Result<(), RepositoryError> {
        delegate!(self, set_group_roles(group_id, roles))
    }

    async fn list_group_roles(&self, group_id: i64) -> Result<Vec<String>, RepositoryError> {
        delegate!(self, list_group_roles(group_id))
    }

    async fn add_group_member(&self, group_id: i64, user_id: i64) -> Result<(), RepositoryError> {
        delegate!(self, add_group_member(group_id, user_id))
    }

    async fn remove_group_member(
        &self,
        group_id: i64,
        user_id: i64,
    ) -> Result<bool, RepositoryError> {
        delegate!(self, remove_group_member(group_id, user_id))
    }

    async fn list_group_member_ids(&self, group_id: i64) -> Result<Vec<i64>, RepositoryError> {
        delegate!(self, list_group_member_ids(group_id))
    }

    async fn add_user_maintainer(
        &self,
        service_id: i64,
        user_id: i64,
    ) -> Result<(), RepositoryError> {
        delegate!(self, add_user_maintainer(service_id, user_id))
    }

    async fn remove_user_maintainer(
        &self,
        service_id: i64,
        user_id: i64,
    ) -> Result<bool, RepositoryError> {
        delegate!(self, remove_user_maintainer(service_id, user_id))
    }

    async fn add_group_maintainer(
        &self,
        service_id: i64,
        group_id: i64,
    ) -> Result<(), RepositoryError> {
        delegate!(self, add_group_maintainer(service_id, group_id))
    }

    async fn remove_group_maintainer(
        &self,
        service_id: i64,
        group_id: i64,
    ) -> Result<bool, RepositoryError> {
        delegate!(self, remove_group_maintainer(service_id, group_id))
    }

    async fn list_user_maintainer_ids(&self, service_id: i64) -> Result<Vec<i64>, RepositoryError> {
        delegate!(self, list_user_maintainer_ids(service_id))
    }

    async fn list_group_maintainer_ids(
        &self,
        service_id: i64,
    ) -> Result<Vec<i64>, RepositoryError> {
        delegate!(self, list_group_maintainer_ids(service_id))
    }

    async fn list_all_user_maintainers(&self) -> Result<Vec<(String, i64)>, RepositoryError> {
        delegate!(self, list_all_user_maintainers())
    }

    async fn list_all_group_maintainers(&self) -> Result<Vec<(String, i64)>, RepositoryError> {
        delegate!(self, list_all_group_maintainers())
    }

    async fn list_group_maintained_producers(
        &self,
        group_ids: &[i64],
    ) -> Result<Vec<String>, RepositoryError> {
        delegate!(self, list_group_maintained_producers(group_ids))
    }

    async fn maintains_producer(
        &self,
        user_id: i64,
        service_id: i64,
    ) -> Result<bool, RepositoryError> {
        delegate!(self, maintains_producer(user_id, service_id))
    }

    async fn list_maintained_producers(
        &self,
        user_id: i64,
    ) -> Result<Vec<String>, RepositoryError> {
        delegate!(self, list_maintained_producers(user_id))
    }

    async fn add_service_tags(
        &self,
        service_id: i64,
        tags: &[String],
    ) -> Result<(), RepositoryError> {
        delegate!(self, add_service_tags(service_id, tags))
    }

    async fn remove_service_tag(&self, service_id: i64, tag: &str) -> Result<(), RepositoryError> {
        delegate!(self, remove_service_tag(service_id, tag))
    }

    async fn close_trunk_pins_for_service_api(
        &self,
        service_id: i64,
        api_type: ApiType,
        now_iso: &str,
    ) -> Result<u64, RepositoryError> {
        delegate!(
            self,
            close_trunk_pins_for_service_api(service_id, api_type, now_iso)
        )
    }

    async fn get_all_service_tags(&self) -> Result<HashMap<String, Vec<String>>, RepositoryError> {
        delegate!(self, get_all_service_tags())
    }

    async fn insert_audit_log(
        &self,
        username: &str,
        log: NewAuditLog<'_>,
    ) -> Result<(), RepositoryError> {
        delegate!(self, insert_audit_log(username, log))
    }

    async fn get_audit_logs(
        &self,
        filter: AuditLogFilter,
    ) -> Result<Vec<AuditLogEntry>, RepositoryError> {
        delegate!(self, get_audit_logs(filter))
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

    async fn get_channel_message_contract(
        &self,
        channel: &str,
        message_name: &str,
    ) -> Result<Option<ChannelMessageContract>, RepositoryError> {
        delegate!(self, get_channel_message_contract(channel, message_name))
    }

    async fn upsert_channel_message_contract(
        &self,
        contract: &ChannelMessageContract,
    ) -> Result<(), RepositoryError> {
        delegate!(self, upsert_channel_message_contract(contract))
    }

    async fn delete_channel_message_contract(
        &self,
        channel: &str,
        message_name: &str,
    ) -> Result<(), RepositoryError> {
        delegate!(self, delete_channel_message_contract(channel, message_name))
    }

    async fn list_channel_message_contracts(
        &self,
    ) -> Result<Vec<ChannelMessageContract>, RepositoryError> {
        delegate!(self, list_channel_message_contracts())
    }

    async fn reconcile_harvested_subscriptions(
        &self,
        client_id: i64,
        client_name: &str,
        trunk: bool,
        subs: &[HarvestedSubInput],
        now_iso: &str,
    ) -> Result<(), RepositoryError> {
        delegate!(
            self,
            reconcile_harvested_subscriptions(client_id, client_name, trunk, subs, now_iso)
        )
    }

    async fn list_current_harvested_subscriptions(
        &self,
    ) -> Result<Vec<HarvestedSubscription>, RepositoryError> {
        delegate!(self, list_current_harvested_subscriptions())
    }

    async fn reconcile_periphery(
        &self,
        system: &str,
        trunk: bool,
        resources: &[String],
        now_iso: &str,
        outbox: Vec<NewWebhookEvent>,
    ) -> Result<bool, RepositoryError> {
        delegate!(
            self,
            reconcile_periphery(system, trunk, resources, now_iso, outbox)
        )
    }

    async fn upsert_system_context(
        &self,
        system: &str,
        context: &SystemContext,
        now_iso: &str,
    ) -> Result<bool, RepositoryError> {
        delegate!(self, upsert_system_context(system, context, now_iso))
    }

    async fn get_system_context(
        &self,
        system: &str,
    ) -> Result<Option<SystemContext>, RepositoryError> {
        delegate!(self, get_system_context(system))
    }

    async fn list_system_contexts(
        &self,
    ) -> Result<BTreeMap<String, SystemContext>, RepositoryError> {
        delegate!(self, list_system_contexts())
    }

    async fn delete_system_context(&self, system: &str) -> Result<(), RepositoryError> {
        delegate!(self, delete_system_context(system))
    }

    async fn list_periphery(
        &self,
        track: PeripheryTrack,
        at: Option<&str>,
    ) -> Result<Vec<PeripheralDependency>, RepositoryError> {
        delegate!(self, list_periphery(track, at))
    }

    async fn list_member_versions(
        &self,
        track: MemberTrack,
        at: Option<&str>,
    ) -> Result<Vec<MemberVersion>, RepositoryError> {
        delegate!(self, list_member_versions(track, at))
    }

    // --- Webhooks (#47) ---

    async fn insert_webhook_events(
        &self,
        events: &[NewWebhookEvent],
    ) -> Result<(), RepositoryError> {
        delegate!(self, insert_webhook_events(events))
    }

    async fn list_current_consumers_of(
        &self,
        service_id: i64,
    ) -> Result<Vec<String>, RepositoryError> {
        delegate!(self, list_current_consumers_of(service_id))
    }

    async fn create_webhook(&self, hook: NewWebhook<'_>) -> Result<i64, RepositoryError> {
        delegate!(self, create_webhook(hook))
    }

    async fn list_webhooks(
        &self,
        scope: Option<&WebhookScope>,
    ) -> Result<Vec<Webhook>, RepositoryError> {
        delegate!(self, list_webhooks(scope))
    }

    async fn get_webhook(&self, id: i64) -> Result<Option<Webhook>, RepositoryError> {
        delegate!(self, get_webhook(id))
    }

    async fn update_webhook(&self, id: i64, patch: &WebhookPatch) -> Result<bool, RepositoryError> {
        delegate!(self, update_webhook(id, patch))
    }

    async fn rotate_webhook_secret(
        &self,
        id: i64,
        secret: &str,
        secret_hint: &str,
    ) -> Result<bool, RepositoryError> {
        delegate!(self, rotate_webhook_secret(id, secret, secret_hint))
    }

    async fn delete_webhook(&self, id: i64) -> Result<bool, RepositoryError> {
        delegate!(self, delete_webhook(id))
    }

    async fn list_webhook_deliveries(
        &self,
        hook_id: i64,
        limit: u32,
    ) -> Result<Vec<WebhookDelivery>, RepositoryError> {
        delegate!(self, list_webhook_deliveries(hook_id, limit))
    }

    async fn get_webhook_delivery(
        &self,
        delivery_id: i64,
    ) -> Result<Option<WebhookDelivery>, RepositoryError> {
        delegate!(self, get_webhook_delivery(delivery_id))
    }

    async fn reset_webhook_delivery(
        &self,
        delivery_id: i64,
        now_iso: &str,
    ) -> Result<bool, RepositoryError> {
        delegate!(self, reset_webhook_delivery(delivery_id, now_iso))
    }

    async fn claim_due_webhook_deliveries(
        &self,
        now_iso: &str,
        lease_until_iso: &str,
        limit: u32,
    ) -> Result<Vec<DueWebhookDelivery>, RepositoryError> {
        delegate!(
            self,
            claim_due_webhook_deliveries(now_iso, lease_until_iso, limit)
        )
    }

    async fn record_webhook_attempt(
        &self,
        delivery_id: i64,
        now_iso: &str,
        outcome: &WebhookAttemptOutcome,
        lease_until_iso: &str,
    ) -> Result<bool, RepositoryError> {
        delegate!(
            self,
            record_webhook_attempt(delivery_id, now_iso, outcome, lease_until_iso)
        )
    }

    async fn count_pending_webhook_deliveries(&self) -> Result<u64, RepositoryError> {
        delegate!(self, count_pending_webhook_deliveries())
    }

    async fn delete_old_webhook_data(&self, cutoff_iso: &str) -> Result<u64, RepositoryError> {
        delegate!(self, delete_old_webhook_data(cutoff_iso))
    }
}
