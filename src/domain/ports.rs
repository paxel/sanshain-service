use crate::domain::models::*;
use std::collections::HashMap;
use std::future::Future;
use thiserror::Error;

/// Error type for authentication provider operations.
#[derive(Error, Debug)]
pub enum AuthProviderError {
    #[error("Invalid credentials")]
    InvalidCredentials,
    #[error("Connection failed: {0}")]
    ConnectionFailed(String),
    #[error("Internal error: {0}")]
    Internal(String),
}

/// Port trait for pluggable authentication providers.
pub trait AuthProvider: Send + Sync {
    /// Authenticate a user by username and password.
    fn authenticate(
        &self,
        username: &str,
        password: &str,
    ) -> impl Future<Output = Result<AuthenticatedUser, AuthProviderError>> + Send;

    /// Test connectivity to the auth backend.
    fn test_connection(&self) -> impl Future<Output = Result<(), AuthProviderError>> + Send;
}

/// Port for reading a user's group membership from the directory.
///
/// Separate from [`AuthProvider`] because it answers a different question at a
/// different time: authentication happens once, at login, with the user's
/// password; membership has to be re-read afterwards, without one, or a change
/// in the directory would never take effect.
pub trait DirectoryGroups: Send + Sync {
    fn groups_for(
        &self,
        username: &str,
    ) -> impl Future<Output = Result<Vec<String>, AuthProviderError>> + Send;
}

#[derive(Error, Debug)]
pub enum RepositoryError {
    #[error("Not Found")]
    NotFound,
    #[error("Conflict")]
    Conflict,
    #[error("Internal Error: {0}")]
    Internal(String),
}

/// Port for all persistence operations required by the application layer.
pub struct RecordDependencyParams<'a> {
    pub client_id: i64,
    pub spec_version_id: i64,
    pub api_type: ApiType,
    pub path: &'a str,
    pub normalized_path: &'a str,
    pub method: &'a str,
}

/// One trunk pin to record (ADR-0004). The version is by value — the trunk
/// store never references a `spec_versions` row id.
pub struct RecordTrunkPinParams<'a> {
    pub client_id: i64,
    pub service_id: i64,
    pub api_type: ApiType,
    pub version: crate::domain::models::SemVer,
    pub path: &'a str,
    pub normalized_path: &'a str,
    pub method: &'a str,
    pub now_iso: &'a str,
}

pub type EndpointDetails = (i64, String, bool);
pub type EndpointMap = HashMap<(String, String), EndpointDetails>;

/// Everything one successful Provide persists, atomically: the version-line
/// entry (inserted, overwritten, or promoted in place) and its full endpoint
/// set (replaced wholesale). The *decision* whether this write is allowed —
/// GA immutability, promotion, mislabel checks — is the application layer's;
/// the repository persists what it is handed.
pub struct UpsertSpecVersion<'a> {
    pub service_id: i64,
    pub api_type: ApiType,
    pub version: SemVer,
    pub stability: Stability,
    pub content: &'a str,
    pub content_hash: &'a str,
    /// The Actor to credit for this version's content. The application layer
    /// decides: the caller, or — on a same-content promotion — the snapshot's
    /// original provider.
    pub provided_by: &'a str,
    /// Compare-and-set: when `Some`, an overwrite only lands if the stored
    /// row still carries this content hash; a mismatch is a `Conflict`.
    /// `None` skips the check (ordinary provides).
    pub expected_prior_hash: Option<&'a str>,
    pub now_iso: &'a str,
    pub endpoints: Vec<EndpointRecord>,
}

/// Content of a new audit-log entry — everything except the acting username,
/// which the caller resolves from the request context.
pub struct NewAuditLog<'a> {
    pub action: &'a str,
    pub details: &'a str,
    pub service: Option<&'a str>,
    pub version: Option<&'a str>,
    pub action_type: Option<&'a str>,
    pub diff: Option<&'a str>,
    /// The declared stream (ADR-0005): `trunk`, a sanshain-branch's tag
    /// name, or `None` for calls outside the stream model. The *label* the
    /// call was made with — see `branch_id` for the identity.
    pub stream: Option<&'a str>,
    /// The sanshain-branch this entry belongs to, when the stream names one.
    /// Identity, so a rename does not orphan the row and a reused name does
    /// not absorb a dead branch's history. `None` for `trunk` and for calls
    /// outside the stream model.
    pub branch_id: Option<i64>,
}

pub trait SpecRepository: Send + Sync {
    /// Liveness/readiness check against the backing store: runs a trivial query
    /// (`SELECT 1`) to confirm the connection pool can reach the database.
    fn ping(&self) -> impl Future<Output = Result<(), RepositoryError>> + Send;

    /// Persist one Provide atomically: insert the version-line entry, or
    /// overwrite/promote the existing row for the same
    /// `(service, api_type, version)` — keeping its `created_at` and identity —
    /// and replace its endpoint set wholesale. Returns the entry's id.
    fn upsert_spec_version(
        &self,
        params: UpsertSpecVersion<'_>,
    ) -> impl Future<Output = Result<i64, RepositoryError>> + Send;

    /// The version-line entry for `(service, api_type, version)`, if any.
    fn find_spec_version(
        &self,
        service_id: i64,
        api_type: ApiType,
        version: SemVer,
    ) -> impl Future<Output = Result<Option<SpecVersionMeta>, RepositoryError>> + Send;

    /// Every version-line entry of one Producer, all API types.
    fn list_spec_versions(
        &self,
        service_id: i64,
    ) -> impl Future<Output = Result<Vec<SpecVersionMeta>, RepositoryError>> + Send;

    /// Every version-line entry instance-wide, with its Producer's name and
    /// its endpoint count. Feeds the producers listing and the graph.
    fn list_all_spec_versions(
        &self,
    ) -> impl Future<Output = Result<Vec<(String, SpecVersionMeta, i64)>, RepositoryError>> + Send;

    /// The full provided document stored for a version-line entry.
    fn get_spec_content(
        &self,
        spec_version_id: i64,
    ) -> impl Future<Output = Result<Option<String>, RepositoryError>> + Send;

    /// Delete one version-line entry (endpoints and dependencies cascade).
    /// Returns whether it existed.
    fn delete_spec_version(
        &self,
        spec_version_id: i64,
    ) -> impl Future<Output = Result<bool, RepositoryError>> + Send;

    /// Record that a version was successfully required (use-based snapshot
    /// expiry counts requires as use).
    fn touch_spec_version_required(
        &self,
        spec_version_id: i64,
        now_iso: &str,
    ) -> impl Future<Output = Result<(), RepositoryError>> + Send;

    /// Record that a version was re-provided with identical content — the
    /// idempotent no-op still counts as "provided" for use-based snapshot
    /// expiry, so only `updated_at` moves.
    fn touch_spec_version_provided(
        &self,
        spec_version_id: i64,
        now_iso: &str,
    ) -> impl Future<Output = Result<(), RepositoryError>> + Send;

    /// Stamp the entry as trunk's current version (ADR-0004): a real
    /// trunk-flagged Provide — including the idempotent no-op — refreshes
    /// `trunk_provided_at`.
    fn touch_spec_version_trunk(
        &self,
        spec_version_id: i64,
        now_iso: &str,
    ) -> impl Future<Output = Result<(), RepositoryError>> + Send;

    /// Record trunk pins, append-only (ADR-0004/0005): per pin key, a
    /// different version closes the open record and inserts a new one; the
    /// same version only refreshes the open record's `last_required_at`.
    /// Nothing is ever overwritten or deleted — closed records are the
    /// timeline.
    fn record_trunk_pins(
        &self,
        pins: Vec<RecordTrunkPinParams<'_>>,
    ) -> impl Future<Output = Result<(), RepositoryError>> + Send;

    /// The current trunk pin set — one entry per open record, joined to
    /// client/service names. The main graph's edges.
    fn list_current_trunk_pins(
        &self,
    ) -> impl Future<Output = Result<Vec<TrunkPinInfo>, RepositoryError>> + Send;

    /// Create a sanshain-branch record (ADR-0005). `Conflict` when the name
    /// is already taken by a live branch.
    fn insert_branch(
        &self,
        name: &str,
        created_at: &str,
        created_by: &str,
        source: &str,
        as_of: &str,
    ) -> impl Future<Output = Result<i64, RepositoryError>> + Send;

    fn find_branch(
        &self,
        name: &str,
    ) -> impl Future<Output = Result<Option<BranchInfo>, RepositoryError>> + Send;

    fn list_branches(
        &self,
    ) -> impl Future<Output = Result<Vec<BranchInfo>, RepositoryError>> + Send;

    /// Copy the trunk graph as it was at `as_of` into the branch as its
    /// initial open rows (`valid_from = now_iso`). A row was current at
    /// `as_of` iff `valid_from <= as_of` and (`valid_to` is NULL or
    /// `valid_to > as_of`).
    fn copy_trunk_graph_to_branch(
        &self,
        branch_id: i64,
        as_of: &str,
        now_iso: &str,
    ) -> impl Future<Output = Result<(), RepositoryError>> + Send;

    /// Same as [`SpecRepository::copy_trunk_graph_to_branch`], sourcing from
    /// another branch's graph at `as_of`.
    fn copy_branch_graph_to_branch(
        &self,
        target_branch_id: i64,
        source_branch_id: i64,
        as_of: &str,
        now_iso: &str,
    ) -> impl Future<Output = Result<(), RepositoryError>> + Send;

    /// A branch's pin set: the rows current at `at`, or the open rows when
    /// `at` is `None`.
    fn list_branch_pins(
        &self,
        branch_id: i64,
        at: Option<&str>,
    ) -> impl Future<Output = Result<Vec<TrunkPinInfo>, RepositoryError>> + Send;

    /// The trunk pin set as it was at `at` (rows current at that instant).
    fn list_trunk_pins_at(
        &self,
        at: &str,
    ) -> impl Future<Output = Result<Vec<TrunkPinInfo>, RepositoryError>> + Send;

    /// The instants a graph changed — every open/close event of its pin
    /// store, distinct and ascending. `None` = trunk; `Some` = a branch.
    /// The timeline slider's markers (ADR-0005).
    fn list_graph_change_dates(
        &self,
        branch_id: Option<i64>,
    ) -> impl Future<Output = Result<Vec<String>, RepositoryError>> + Send;

    /// Reverse lookup (ADR-0005): every (api_type, version, branch) a
    /// producer is referenced by across all branches' open records.
    fn list_branch_memberships_for_service(
        &self,
        service_id: i64,
    ) -> impl Future<Output = Result<Vec<BranchMembership>, RepositoryError>> + Send;

    /// Names of branches whose graph or member versions reference this
    /// (service, api_type, version) by value in an open record — surfaced in
    /// the delete-version warning (ADR-0005).
    fn list_branches_referencing(
        &self,
        service_id: i64,
        api_type: ApiType,
        version: SemVer,
    ) -> impl Future<Output = Result<Vec<String>, RepositoryError>> + Send;

    /// Trunk TTL cleanup (ADR-0004/0005): close open trunk pins not
    /// re-required since the cutoff and clear stale trunk version markers.
    /// Rows are closed, never deleted — history is the timeline. Returns how
    /// many pins were closed.
    fn close_expired_trunk_data(
        &self,
        cutoff_iso: &str,
        now_iso: &str,
    ) -> impl Future<Output = Result<u64, RepositoryError>> + Send;

    /// Rename a branch — identity is the id, so membership, timeline and
    /// audit stamps survive. `Conflict` when the new name is already live.
    fn rename_branch(
        &self,
        branch_id: i64,
        new_name: &str,
    ) -> impl Future<Output = Result<(), RepositoryError>> + Send;

    /// Delete a branch and (cascading) its graph rows; frees the name.
    fn delete_branch(
        &self,
        branch_id: i64,
    ) -> impl Future<Output = Result<(), RepositoryError>> + Send;

    /// Record pins on a branch's graph, with exactly the append semantics of
    /// [`SpecRepository::record_trunk_pins`] — the hotfix act (ADR-0005).
    fn record_branch_pins(
        &self,
        branch_id: i64,
        pins: Vec<RecordTrunkPinParams<'_>>,
    ) -> impl Future<Output = Result<(), RepositoryError>> + Send;

    /// Mark the producer's member version within a branch (tag-flagged
    /// Provide): a different version closes the open record and inserts; the
    /// same version refreshes nothing (already current). Append-only.
    fn record_branch_member_version(
        &self,
        branch_id: i64,
        service_id: i64,
        api_type: ApiType,
        version: SemVer,
        now_iso: &str,
    ) -> impl Future<Output = Result<(), RepositoryError>> + Send;

    /// Delete snapshot entries that were neither provided nor required since
    /// the cutoff — GA entries are never age-culled. Returns how many died.
    fn delete_expired_snapshots(
        &self,
        cutoff_iso: &str,
    ) -> impl Future<Output = Result<u64, RepositoryError>> + Send;

    /// Names of the Consumers currently recorded as depending on a version —
    /// shown before a delete-version is confirmed.
    fn list_version_dependents(
        &self,
        spec_version_id: i64,
    ) -> impl Future<Output = Result<Vec<String>, RepositoryError>> + Send;

    /// Ensure a service exists and return its ID.
    fn ensure_service(
        &self,
        name: &str,
    ) -> impl Future<Output = Result<i64, RepositoryError>> + Send;

    /// Find a service by name (read-only, does not create). Returns None if not found.
    fn find_service(
        &self,
        name: &str,
    ) -> impl Future<Output = Result<Option<i64>, RepositoryError>> + Send;

    /// Get a service name by its ID. Returns None if not found.
    fn get_service_name_by_id(
        &self,
        service_id: i64,
    ) -> impl Future<Output = Result<Option<String>, RepositoryError>> + Send;

    /// Fetch all endpoints of one version-line entry.
    fn get_endpoints_for_version(
        &self,
        spec_version_id: i64,
    ) -> impl Future<Output = Result<Vec<EndpointRecord>, RepositoryError>> + Send;

    /// Ensure a client exists and return its ID.
    fn ensure_client(
        &self,
        name: &str,
    ) -> impl Future<Output = Result<i64, RepositoryError>> + Send;

    /// Find an endpoint of a version-line entry by path and method (lenient
    /// path matching via the normalized path). Returns
    /// (endpoint_id, yaml_content, deprecated).
    fn find_endpoint(
        &self,
        spec_version_id: i64,
        api_type: ApiType,
        path: &str,
        method: &str,
    ) -> impl Future<Output = Result<Option<(i64, String, bool)>, RepositoryError>> + Send;

    /// Find multiple endpoints of a version-line entry by path and method.
    /// Returns a map of (path, method) to (endpoint_id, yaml_content).
    fn find_endpoints_bulk(
        &self,
        spec_version_id: i64,
        api_type: ApiType,
        endpoints: &[(String, String)],
    ) -> impl Future<Output = Result<EndpointMap, RepositoryError>> + Send;

    /// Record a client dependency on an endpoint.
    fn record_dependency(
        &self,
        params: RecordDependencyParams,
    ) -> impl Future<Output = Result<(), RepositoryError>> + Send;

    /// Record multiple client dependencies at once.
    fn record_dependencies_bulk(
        &self,
        params: Vec<RecordDependencyParams>,
    ) -> impl Future<Output = Result<(), RepositoryError>> + Send;

    /// Get the full, instance-wide dependency report.
    fn get_report(&self) -> impl Future<Output = Result<DependencyReport, RepositoryError>> + Send;

    /// Delete all services and their versions, endpoints, and related dependencies.
    fn delete_all_services(&self) -> impl Future<Output = Result<u64, RepositoryError>> + Send;

    /// Delete all clients and their dependencies.
    fn delete_all_clients(&self) -> impl Future<Output = Result<u64, RepositoryError>> + Send;

    /// Delete all users who are not *effective* administrators — spared are
    /// users with a direct `admin` grant, users holding `admin` through a
    /// stored group membership, and the configuration-held root usernames in
    /// `spare_usernames` (who need no stored grant at all).
    fn delete_all_non_admin_users(
        &self,
        spare_usernames: &[String],
    ) -> impl Future<Output = Result<u64, RepositoryError>> + Send;

    /// Nuke the entire database: delete all data from all tables (except settings and the calling admin user).
    fn nuke_database(
        &self,
        keep_user_id: Option<i64>,
    ) -> impl Future<Output = Result<(), RepositoryError>> + Send;

    /// Delete a service and all its versions, endpoints, and related dependencies.
    fn delete_producer(
        &self,
        name: &str,
    ) -> impl Future<Output = Result<bool, RepositoryError>> + Send;

    /// Delete a client and all its dependencies.
    fn delete_consumer(
        &self,
        name: &str,
    ) -> impl Future<Output = Result<bool, RepositoryError>> + Send;

    /// List all services.
    fn list_producers(&self) -> impl Future<Output = Result<Vec<String>, RepositoryError>> + Send;

    /// List all services with metadata (icon, domain). Version lines are
    /// populated by the application layer from `list_all_spec_versions`.
    fn list_producers_detailed(
        &self,
    ) -> impl Future<Output = Result<Vec<ProducerSummary>, RepositoryError>> + Send;

    /// Update service metadata (icon, domain).
    fn update_producer_metadata(
        &self,
        service_name: &str,
        icon: Option<&str>,
        domain: Option<&str>,
    ) -> impl Future<Output = Result<(), RepositoryError>> + Send;

    /// List all clients.
    fn list_consumers(&self) -> impl Future<Output = Result<Vec<String>, RepositoryError>> + Send;

    /// Every endpoint a client is recorded as depending on, with the pinned
    /// version and its current stability.
    fn list_consumer_endpoints(
        &self,
        client_name: &str,
    ) -> impl Future<Output = Result<Vec<ConsumerEndpointInfo>, RepositoryError>> + Send;

    // --- Auth ---

    /// Count total users.
    fn user_count(&self) -> impl Future<Output = Result<i64, RepositoryError>> + Send;

    /// Find a user by username.
    fn find_user(
        &self,
        username: &str,
    ) -> impl Future<Output = Result<Option<User>, RepositoryError>> + Send;

    /// Create a new user.
    fn create_user(
        &self,
        username: &str,
        password_hash: &str,
        approved: bool,
    ) -> impl Future<Output = Result<User, RepositoryError>> + Send;

    /// Update a user's password hash.
    fn update_password(
        &self,
        user_id: i64,
        new_hash: &str,
    ) -> impl Future<Output = Result<(), RepositoryError>> + Send;

    /// List all users (id, username, approved).
    fn list_users(&self) -> impl Future<Output = Result<Vec<User>, RepositoryError>> + Send;

    /// Approve a user by ID.
    fn approve_user(
        &self,
        user_id: i64,
    ) -> impl Future<Output = Result<bool, RepositoryError>> + Send;

    /// Delete a user by ID.
    fn delete_user(
        &self,
        user_id: i64,
    ) -> impl Future<Output = Result<bool, RepositoryError>> + Send;

    /// Create a session for a user, returning the session with a generated token.
    fn create_session(
        &self,
        user_id: i64,
        expires_at: &str,
    ) -> impl Future<Output = Result<Session, RepositoryError>> + Send;

    /// Create a session for a user with a specific token.
    fn create_session_with_token(
        &self,
        user_id: i64,
        token: &str,
        expires_at: &str,
    ) -> impl Future<Output = Result<Session, RepositoryError>> + Send;

    /// Validate a session token, returning the user and session if valid and not expired.
    fn validate_session(
        &self,
        token: &str,
    ) -> impl Future<Output = Result<Option<(User, Session)>, RepositoryError>> + Send;

    /// Delete a session (logout).
    fn delete_session(
        &self,
        token: &str,
    ) -> impl Future<Output = Result<(), RepositoryError>> + Send;

    /// Get a setting value by key.
    fn get_setting(
        &self,
        key: &str,
    ) -> impl Future<Output = Result<Option<String>, RepositoryError>> + Send;

    /// Set a setting value.
    fn set_setting(
        &self,
        key: &str,
        value: &str,
    ) -> impl Future<Output = Result<(), RepositoryError>> + Send;

    // --- API Tokens ---

    /// Create an API token (stores the hash, not the raw token).
    fn create_api_token(
        &self,
        id: &str,
        user_id: i64,
        name: &str,
        token_hash: &str,
        created_at: &str,
        expires_at: &str,
    ) -> impl Future<Output = Result<(), RepositoryError>> + Send;

    /// List all API tokens for a user (never includes raw token).
    fn list_api_tokens(
        &self,
        user_id: i64,
    ) -> impl Future<Output = Result<Vec<ApiToken>, RepositoryError>> + Send;

    /// Delete an API token by ID (only if owned by user_id). Returns whether a
    /// token was actually deleted, so the caller can answer 404 rather than
    /// reporting success for a token that never existed.
    ///
    /// Deliberately does *not* return the token's name: audit entries for token
    /// revocation must not record it (a name its creator considered private
    /// cannot be redacted from a permanent audit log afterwards), so the name
    /// is not handed to callers who might log it.
    fn delete_api_token(
        &self,
        token_id: &str,
        user_id: i64,
    ) -> impl Future<Output = Result<bool, RepositoryError>> + Send;

    /// Validate an API token hash, returning the user if valid and not expired. Also updates last_used_at.
    fn validate_api_token(
        &self,
        token_hash: &str,
    ) -> impl Future<Output = Result<Option<User>, RepositoryError>> + Send;

    /// Delete dependency rows whose `last_seen_at` is older than the given cutoff ISO timestamp.
    /// Returns the number of deleted rows.
    fn delete_stale_dependencies(
        &self,
        cutoff_iso: &str,
    ) -> impl Future<Output = Result<u64, RepositoryError>> + Send;

    // --- Roles and Groups ---

    /// Grant a role to a user. Granting a role already held is a no-op.
    fn grant_user_role(
        &self,
        user_id: i64,
        role: &str,
    ) -> impl Future<Output = Result<(), RepositoryError>> + Send;

    /// Revoke a role from a user. Returns whether a grant was actually removed.
    fn revoke_user_role(
        &self,
        user_id: i64,
        role: &str,
    ) -> impl Future<Output = Result<bool, RepositoryError>> + Send;

    /// Roles granted directly to a user, ignoring any held via a group.
    fn list_user_roles(
        &self,
        user_id: i64,
    ) -> impl Future<Output = Result<Vec<String>, RepositoryError>> + Send;

    /// Every role name a user holds from stored grants — directly, or through a
    /// group whose membership Sanshain stores.
    ///
    /// Directory-sourced groups are deliberately absent: their membership is not
    /// stored, so it is resolved above the repository where the directory can be
    /// consulted.
    fn effective_stored_roles(
        &self,
        user_id: i64,
    ) -> impl Future<Output = Result<Vec<String>, RepositoryError>> + Send;

    /// Create a group, or return the existing one with the same name and source.
    fn create_group(
        &self,
        name: &str,
        source: GroupSource,
    ) -> impl Future<Output = Result<Group, RepositoryError>> + Send;

    /// Rename a group. Returns whether the group existed.
    fn rename_group(
        &self,
        group_id: i64,
        name: &str,
    ) -> impl Future<Output = Result<bool, RepositoryError>> + Send;

    /// Delete a group along with its membership and role grants. Returns whether
    /// the group existed.
    fn delete_group(
        &self,
        group_id: i64,
    ) -> impl Future<Output = Result<bool, RepositoryError>> + Send;

    /// All groups, of every source.
    fn list_groups(&self) -> impl Future<Output = Result<Vec<Group>, RepositoryError>> + Send;

    /// Replace a group's role grants with exactly `roles`.
    fn set_group_roles(
        &self,
        group_id: i64,
        roles: &[String],
    ) -> impl Future<Output = Result<(), RepositoryError>> + Send;

    /// Roles granted to a group.
    fn list_group_roles(
        &self,
        group_id: i64,
    ) -> impl Future<Output = Result<Vec<String>, RepositoryError>> + Send;

    /// Add a user to a group. Adding an existing member is a no-op.
    fn add_group_member(
        &self,
        group_id: i64,
        user_id: i64,
    ) -> impl Future<Output = Result<(), RepositoryError>> + Send;

    /// Remove a user from a group. Returns whether they were a member.
    fn remove_group_member(
        &self,
        group_id: i64,
        user_id: i64,
    ) -> impl Future<Output = Result<bool, RepositoryError>> + Send;

    /// Ids of the users stored as members of a group. Empty for a
    /// directory-sourced group, which stores no membership.
    fn list_group_member_ids(
        &self,
        group_id: i64,
    ) -> impl Future<Output = Result<Vec<i64>, RepositoryError>> + Send;

    // --- Maintainer Scope ---

    /// Assign a user as maintainer of a Producer. Reassigning is a no-op.
    fn add_user_maintainer(
        &self,
        service_id: i64,
        user_id: i64,
    ) -> impl Future<Output = Result<(), RepositoryError>> + Send;

    /// Remove a user's maintainership. Returns whether they held it.
    fn remove_user_maintainer(
        &self,
        service_id: i64,
        user_id: i64,
    ) -> impl Future<Output = Result<bool, RepositoryError>> + Send;

    /// Assign a group as maintainer of a Producer. Reassigning is a no-op.
    fn add_group_maintainer(
        &self,
        service_id: i64,
        group_id: i64,
    ) -> impl Future<Output = Result<(), RepositoryError>> + Send;

    /// Remove a group's maintainership. Returns whether it held it.
    fn remove_group_maintainer(
        &self,
        service_id: i64,
        group_id: i64,
    ) -> impl Future<Output = Result<bool, RepositoryError>> + Send;

    /// Users assigned directly as maintainers of a Producer.
    fn list_user_maintainer_ids(
        &self,
        service_id: i64,
    ) -> impl Future<Output = Result<Vec<i64>, RepositoryError>> + Send;

    /// Groups assigned as maintainers of a Producer.
    fn list_group_maintainer_ids(
        &self,
        service_id: i64,
    ) -> impl Future<Output = Result<Vec<i64>, RepositoryError>> + Send;

    /// Names of every Producer any of `group_ids` maintains. Used to expand a
    /// caller's directory groups into the Producers they confer.
    fn list_group_maintained_producers(
        &self,
        group_ids: &[i64],
    ) -> impl Future<Output = Result<Vec<String>, RepositoryError>> + Send;

    /// Every (producer name, maintainer user id) pair, instance-wide. Feeds
    /// the aggregate maintainers listing in one query instead of one per
    /// Producer.
    fn list_all_user_maintainers(
        &self,
    ) -> impl Future<Output = Result<Vec<(String, i64)>, RepositoryError>> + Send;

    /// Every (producer name, maintainer group id) pair, instance-wide.
    fn list_all_group_maintainers(
        &self,
    ) -> impl Future<Output = Result<Vec<(String, i64)>, RepositoryError>> + Send;

    /// Whether a user maintains a Producer, directly or through a group whose
    /// membership Sanshain stores.
    ///
    /// Directory-sourced groups are resolved above the repository, where the
    /// directory can be consulted, and combined with this answer.
    fn maintains_producer(
        &self,
        user_id: i64,
        service_id: i64,
    ) -> impl Future<Output = Result<bool, RepositoryError>> + Send;

    /// Names of the Producers a user maintains, directly or through a stored
    /// group membership.
    fn list_maintained_producers(
        &self,
        user_id: i64,
    ) -> impl Future<Output = Result<Vec<String>, RepositoryError>> + Send;

    // --- Service Tags ---

    /// Set tags for a service (replaces existing tags). Tags are additive with existing ones.
    fn add_service_tags(
        &self,
        service_id: i64,
        tags: &[String],
    ) -> impl Future<Output = Result<(), RepositoryError>> + Send;

    /// Get all service tags as a map of service_name -> Vec<tag>.
    fn get_all_service_tags(
        &self,
    ) -> impl Future<Output = Result<HashMap<String, Vec<String>>, RepositoryError>> + Send;

    // --- Audit Logs ---

    /// Insert an audit log record
    fn insert_audit_log(
        &self,
        username: &str,
        log: NewAuditLog<'_>,
    ) -> impl Future<Output = Result<(), RepositoryError>> + Send;

    /// Get audit log records with filtering
    fn get_audit_logs(
        &self,
        filter: AuditLogFilter,
    ) -> impl Future<Output = Result<Vec<AuditLogEntry>, RepositoryError>> + Send;

    /// Get the recent audit log records, ordered by timestamp DESC
    fn get_recent_audit_logs(
        &self,
        limit: u32,
    ) -> impl Future<Output = Result<Vec<AuditLogEntry>, RepositoryError>> + Send;

    // --- User Favorites ---

    /// Get user favorites by item type ('service' or 'client')
    fn get_user_favorites(
        &self,
        user_id: i64,
        item_type: &str,
    ) -> impl Future<Output = Result<Vec<String>, RepositoryError>> + Send;

    /// Add a user favorite
    fn add_user_favorite(
        &self,
        user_id: i64,
        item_type: &str,
        item_name: &str,
    ) -> impl Future<Output = Result<(), RepositoryError>> + Send;

    /// Remove a user favorite
    fn remove_user_favorite(
        &self,
        user_id: i64,
        item_type: &str,
        item_name: &str,
    ) -> impl Future<Output = Result<(), RepositoryError>> + Send;

    // --- AsyncAPI Channel Message Contracts (item #20) ---

    /// Get the message-level channel contract for `(channel, message)`, if any.
    fn get_channel_message_contract(
        &self,
        channel: &str,
        message_name: &str,
    ) -> impl Future<Output = Result<Option<ChannelMessageContract>, RepositoryError>> + Send;

    /// Insert or replace a channel message contract (owner service and payload).
    fn upsert_channel_message_contract(
        &self,
        contract: &ChannelMessageContract,
    ) -> impl Future<Output = Result<(), RepositoryError>> + Send;

    /// Delete the channel message contract for `(channel, message)`.
    fn delete_channel_message_contract(
        &self,
        channel: &str,
        message_name: &str,
    ) -> impl Future<Output = Result<(), RepositoryError>> + Send;

    /// List all channel message contracts (sorted by channel, message).
    fn list_channel_message_contracts(
        &self,
    ) -> impl Future<Output = Result<Vec<ChannelMessageContract>, RepositoryError>> + Send;
}
