use crate::domain::models::*;
use crate::domain::ports::{
    EndpointMap, NewAuditLog, RecordDependencyParams, RecordTrunkPinParams, RepositoryError,
    SpecRepository, UpsertSpecVersion,
};
use std::collections::HashMap;
use std::sync::{Mutex, PoisonError};

/// One stored version-line entry: `SpecVersionMeta` plus the provided document
/// and its endpoint set, mirroring the `spec_versions` + `endpoints` tables.
#[derive(Clone, Debug)]
pub struct MockSpecVersion {
    pub id: i64,
    pub service_id: i64,
    pub api_type: ApiType,
    pub version: SemVer,
    pub stability: Stability,
    pub content: String,
    pub content_hash: String,
    pub provided_by: String,
    pub created_at: String,
    pub updated_at: String,
    pub last_required_at: Option<String>,
    pub trunk_provided_at: Option<String>,
    pub endpoints: Vec<EndpointRecord>,
}

impl MockSpecVersion {
    fn meta(&self) -> SpecVersionMeta {
        SpecVersionMeta {
            id: self.id,
            service_id: self.service_id,
            api_type: self.api_type,
            version: self.version,
            stability: self.stability,
            content_hash: self.content_hash.clone(),
            provided_by: self.provided_by.clone(),
            created_at: self.created_at.clone(),
            updated_at: self.updated_at.clone(),
            last_required_at: self.last_required_at.clone(),
            trunk_provided_at: self.trunk_provided_at.clone(),
        }
    }
}

/// One trunk pin record, mirroring a `trunk_dependencies` row (append-only).
#[derive(Clone, Debug)]
pub struct MockTrunkPin {
    pub client_id: i64,
    pub service_id: i64,
    pub api_type: ApiType,
    pub version: SemVer,
    pub path: String,
    pub normalized_path: String,
    pub method: String,
    pub valid_from: String,
    pub last_required_at: String,
    pub valid_to: Option<String>,
}

/// One recorded Pin, mirroring a `dependencies` row.
#[derive(Clone, Debug)]
pub struct MockDependency {
    pub client_id: i64,
    pub spec_version_id: i64,
    pub api_type: ApiType,
    pub path: String,
    pub normalized_path: String,
    pub method: String,
    pub last_seen_at: String,
}

/// Producer metadata as stored: (icon, domain).
type ProducerMetadata = (Option<String>, Option<String>);

/// A branch member version as stored:
/// (branch_id, service_id, api_type, version, valid_from, valid_to).
type BranchMemberVersion = (i64, i64, ApiType, SemVer, String, Option<String>);

pub struct MockRepo {
    pub services: Mutex<HashMap<String, i64>>,
    pub producer_metadata: Mutex<HashMap<String, ProducerMetadata>>,
    pub spec_versions: Mutex<Vec<MockSpecVersion>>,
    pub dependencies: Mutex<Vec<MockDependency>>,
    pub trunk_pins: Mutex<Vec<MockTrunkPin>>,
    pub branches: Mutex<Vec<BranchInfo>>,
    pub branch_pins: Mutex<Vec<(i64, MockTrunkPin)>>,
    pub branch_member_versions: Mutex<Vec<BranchMemberVersion>>,
    pub clients: Mutex<HashMap<String, i64>>,
    pub next_id: Mutex<i64>,
    pub users: Mutex<Vec<User>>,
    pub sessions: Mutex<Vec<Session>>,
    pub settings: Mutex<HashMap<String, String>>,
    pub service_tags: Mutex<HashMap<i64, Vec<String>>>,
    pub audit_logs: Mutex<Vec<AuditLogEntry>>,
    pub user_favorites: Mutex<Vec<(i64, String, String)>>,
    pub channel_message_contracts: Mutex<Vec<ChannelMessageContract>>,
    pub user_roles: Mutex<Vec<(i64, String)>>,
    pub groups: Mutex<Vec<Group>>,
    pub group_members: Mutex<Vec<(i64, i64)>>,
    pub group_roles: Mutex<Vec<(i64, String)>>,
    pub user_maintainers: Mutex<Vec<(i64, i64)>>,
    pub group_maintainers: Mutex<Vec<(i64, i64)>>,
}

impl Default for MockRepo {
    fn default() -> Self {
        Self::new()
    }
}

impl MockRepo {
    pub fn new() -> Self {
        let mut settings = HashMap::new();
        settings.insert("dev_mode".to_string(), "false".to_string());
        Self {
            services: Mutex::new(HashMap::new()),
            producer_metadata: Mutex::new(HashMap::new()),
            spec_versions: Mutex::new(Vec::new()),
            dependencies: Mutex::new(Vec::new()),
            trunk_pins: Mutex::new(Vec::new()),
            branches: Mutex::new(Vec::new()),
            branch_pins: Mutex::new(Vec::new()),
            branch_member_versions: Mutex::new(Vec::new()),
            clients: Mutex::new(HashMap::new()),
            next_id: Mutex::new(1),
            users: Mutex::new(Vec::new()),
            sessions: Mutex::new(Vec::new()),
            settings: Mutex::new(settings),
            service_tags: Mutex::new(HashMap::new()),
            audit_logs: Mutex::new(Vec::new()),
            user_favorites: Mutex::new(Vec::new()),
            channel_message_contracts: Mutex::new(Vec::new()),
            user_roles: Mutex::new(Vec::new()),
            groups: Mutex::new(Vec::new()),
            group_members: Mutex::new(Vec::new()),
            group_roles: Mutex::new(Vec::new()),
            user_maintainers: Mutex::new(Vec::new()),
            group_maintainers: Mutex::new(Vec::new()),
        }
    }

    pub fn next_id(&self) -> i64 {
        let mut id = self.next_id.lock().unwrap_or_else(PoisonError::into_inner);
        let current = *id;
        *id += 1;
        current
    }

    fn service_name(&self, service_id: i64) -> Option<String> {
        self.services
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .find(|&(_, &id)| id == service_id)
            .map(|(name, _)| name.clone())
    }

    fn client_name(&self, client_id: i64) -> Option<String> {
        self.clients
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .find(|&(_, &id)| id == client_id)
            .map(|(name, _)| name.clone())
    }
}

/// Sort key matching the SQL `ORDER BY api_type, major, minor, patch` — the
/// api_type column sorts as text, so compare the wire string.
fn version_sort_key(entry: &MockSpecVersion) -> (&'static str, SemVer) {
    (entry.api_type.as_str(), entry.version)
}

impl SpecRepository for MockRepo {
    async fn ping(&self) -> Result<(), RepositoryError> {
        Ok(())
    }

    async fn upsert_spec_version(
        &self,
        params: UpsertSpecVersion<'_>,
    ) -> Result<i64, RepositoryError> {
        let mut endpoints = params.endpoints;
        for endpoint in &mut endpoints {
            endpoint.id = Some(self.next_id());
        }

        let mut versions = self
            .spec_versions
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if let Some(existing) = versions.iter_mut().find(|v| {
            v.service_id == params.service_id
                && v.api_type == params.api_type
                && v.version == params.version
        }) {
            // Mirrors the SQL upserts' WHERE guard: a GA row is immutable
            // even against a racing writer, and a compare-and-set hash that
            // no longer matches means the row moved since the caller read it.
            if existing.stability == Stability::Ga {
                return Err(RepositoryError::Conflict);
            }
            if let Some(expected) = params.expected_prior_hash
                && existing.content_hash != expected
            {
                return Err(RepositoryError::Conflict);
            }
            // Overwrite/promotion keeps the row's identity and `created_at`;
            // the endpoint set is replaced wholesale.
            existing.stability = params.stability;
            existing.content = params.content.to_string();
            existing.content_hash = params.content_hash.to_string();
            existing.provided_by = params.provided_by.to_string();
            existing.updated_at = params.now_iso.to_string();
            existing.endpoints = endpoints;
            return Ok(existing.id);
        }

        // Mirrors the SQL CAS UPDATE matching zero rows: a CAS caller asserted
        // a prior row exists — if it vanished (concurrent delete/expiry),
        // refuse rather than resurrect it.
        if params.expected_prior_hash.is_some() {
            return Err(RepositoryError::Conflict);
        }

        let id = self.next_id();
        versions.push(MockSpecVersion {
            id,
            service_id: params.service_id,
            api_type: params.api_type,
            version: params.version,
            stability: params.stability,
            content: params.content.to_string(),
            content_hash: params.content_hash.to_string(),
            provided_by: params.provided_by.to_string(),
            created_at: params.now_iso.to_string(),
            updated_at: params.now_iso.to_string(),
            last_required_at: None,
            trunk_provided_at: None,
            endpoints,
        });
        Ok(id)
    }

    async fn find_spec_version(
        &self,
        service_id: i64,
        api_type: ApiType,
        version: SemVer,
    ) -> Result<Option<SpecVersionMeta>, RepositoryError> {
        let versions = self
            .spec_versions
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        Ok(versions
            .iter()
            .find(|v| v.service_id == service_id && v.api_type == api_type && v.version == version)
            .map(MockSpecVersion::meta))
    }

    async fn list_spec_versions(
        &self,
        service_id: i64,
    ) -> Result<Vec<SpecVersionMeta>, RepositoryError> {
        let versions = self
            .spec_versions
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let mut entries: Vec<&MockSpecVersion> = versions
            .iter()
            .filter(|v| v.service_id == service_id)
            .collect();
        entries.sort_by_key(|v| version_sort_key(v));
        Ok(entries.into_iter().map(MockSpecVersion::meta).collect())
    }

    async fn list_all_spec_versions(
        &self,
    ) -> Result<Vec<(String, SpecVersionMeta, i64)>, RepositoryError> {
        let versions = self
            .spec_versions
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let mut result: Vec<(String, SpecVersionMeta, i64)> = versions
            .iter()
            .map(|v| {
                (
                    self.service_name(v.service_id).unwrap_or_default(),
                    v.meta(),
                    v.endpoints.len() as i64,
                )
            })
            .collect();
        result.sort_by(|a, b| {
            a.0.cmp(&b.0)
                .then_with(|| a.1.api_type.as_str().cmp(b.1.api_type.as_str()))
                .then_with(|| a.1.version.cmp(&b.1.version))
        });
        Ok(result)
    }

    async fn get_spec_content(
        &self,
        spec_version_id: i64,
    ) -> Result<Option<String>, RepositoryError> {
        let versions = self
            .spec_versions
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        Ok(versions
            .iter()
            .find(|v| v.id == spec_version_id)
            .map(|v| v.content.clone()))
    }

    async fn delete_spec_version(&self, spec_version_id: i64) -> Result<bool, RepositoryError> {
        let mut versions = self
            .spec_versions
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let before = versions.len();
        versions.retain(|v| v.id != spec_version_id);
        let existed = versions.len() != before;
        if existed {
            // Endpoints live inside the entry; dependencies cascade explicitly.
            self.dependencies
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .retain(|d| d.spec_version_id != spec_version_id);
        }
        Ok(existed)
    }

    async fn touch_spec_version_required(
        &self,
        spec_version_id: i64,
        now_iso: &str,
    ) -> Result<(), RepositoryError> {
        let mut versions = self
            .spec_versions
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if let Some(entry) = versions.iter_mut().find(|v| v.id == spec_version_id) {
            entry.last_required_at = Some(now_iso.to_string());
        }
        Ok(())
    }

    async fn touch_spec_version_provided(
        &self,
        spec_version_id: i64,
        now_iso: &str,
    ) -> Result<(), RepositoryError> {
        let mut versions = self
            .spec_versions
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if let Some(entry) = versions.iter_mut().find(|v| v.id == spec_version_id) {
            entry.updated_at = now_iso.to_string();
        }
        Ok(())
    }

    async fn touch_spec_version_trunk(
        &self,
        spec_version_id: i64,
        now_iso: &str,
    ) -> Result<(), RepositoryError> {
        let mut versions = self
            .spec_versions
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if let Some(entry) = versions.iter_mut().find(|v| v.id == spec_version_id) {
            entry.trunk_provided_at = Some(now_iso.to_string());
        }
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
        let mut branches = self.branches.lock().unwrap_or_else(PoisonError::into_inner);
        if branches.iter().any(|b| b.name == name) {
            return Err(RepositoryError::Conflict);
        }
        let id = self.next_id();
        branches.push(BranchInfo {
            id,
            name: name.to_string(),
            created_at: created_at.to_string(),
            created_by: created_by.to_string(),
            source: source.to_string(),
            as_of: as_of.to_string(),
        });
        Ok(id)
    }

    async fn find_branch(&self, name: &str) -> Result<Option<BranchInfo>, RepositoryError> {
        Ok(self
            .branches
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .find(|b| b.name == name)
            .cloned())
    }

    async fn list_branches(&self) -> Result<Vec<BranchInfo>, RepositoryError> {
        let mut branches = self
            .branches
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        branches.sort_by(|a, b| b.created_at.cmp(&a.created_at).then(b.id.cmp(&a.id)));
        Ok(branches)
    }

    async fn copy_trunk_graph_to_branch(
        &self,
        branch_id: i64,
        as_of: &str,
        now_iso: &str,
    ) -> Result<(), RepositoryError> {
        let source: Vec<MockTrunkPin> = self
            .trunk_pins
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .filter(|r| {
                r.valid_from.as_str() <= as_of && r.valid_to.as_deref().is_none_or(|to| to > as_of)
            })
            .cloned()
            .collect();
        let mut pins = self
            .branch_pins
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        for mut row in source {
            row.valid_from = now_iso.to_string();
            row.last_required_at = now_iso.to_string();
            row.valid_to = None;
            pins.push((branch_id, row));
        }
        Ok(())
    }

    async fn copy_branch_graph_to_branch(
        &self,
        target_branch_id: i64,
        source_branch_id: i64,
        as_of: &str,
        now_iso: &str,
    ) -> Result<(), RepositoryError> {
        let mut pins = self
            .branch_pins
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let source: Vec<MockTrunkPin> = pins
            .iter()
            .filter(|(b, r)| {
                *b == source_branch_id
                    && r.valid_from.as_str() <= as_of
                    && r.valid_to.as_deref().is_none_or(|to| to > as_of)
            })
            .map(|(_, r)| r.clone())
            .collect();
        for mut row in source {
            row.valid_from = now_iso.to_string();
            row.last_required_at = now_iso.to_string();
            row.valid_to = None;
            pins.push((target_branch_id, row));
        }
        Ok(())
    }

    async fn list_branch_pins(
        &self,
        branch_id: i64,
        at: Option<&str>,
    ) -> Result<Vec<TrunkPinInfo>, RepositoryError> {
        let pins = self
            .branch_pins
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        Ok(pins
            .iter()
            .filter(|(b, r)| {
                *b == branch_id
                    && match at {
                        Some(at) => {
                            r.valid_from.as_str() <= at
                                && r.valid_to.as_deref().is_none_or(|to| to > at)
                        }
                        None => r.valid_to.is_none(),
                    }
            })
            .map(|(_, r)| TrunkPinInfo {
                client: self.client_name(r.client_id).unwrap_or_default(),
                service: self.service_name(r.service_id).unwrap_or_default(),
                api_type: r.api_type,
                version: r.version,
                path: r.path.clone(),
                normalized_path: r.normalized_path.clone(),
                method: r.method.clone(),
                valid_from: r.valid_from.clone(),
                last_required_at: r.last_required_at.clone(),
                dangling: false,
            })
            .collect())
    }

    async fn list_trunk_pins_at(&self, at: &str) -> Result<Vec<TrunkPinInfo>, RepositoryError> {
        let rows = self
            .trunk_pins
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        Ok(rows
            .iter()
            .filter(|r| {
                r.valid_from.as_str() <= at && r.valid_to.as_deref().is_none_or(|to| to > at)
            })
            .map(|r| TrunkPinInfo {
                client: self.client_name(r.client_id).unwrap_or_default(),
                service: self.service_name(r.service_id).unwrap_or_default(),
                api_type: r.api_type,
                version: r.version,
                path: r.path.clone(),
                normalized_path: r.normalized_path.clone(),
                method: r.method.clone(),
                valid_from: r.valid_from.clone(),
                last_required_at: r.last_required_at.clone(),
                dangling: false,
            })
            .collect())
    }

    async fn list_graph_change_dates(
        &self,
        branch_id: Option<i64>,
    ) -> Result<Vec<String>, RepositoryError> {
        let mut dates: Vec<String> = match branch_id {
            None => self
                .trunk_pins
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .iter()
                .flat_map(|r| std::iter::once(r.valid_from.clone()).chain(r.valid_to.clone()))
                .collect(),
            Some(bid) => self
                .branch_pins
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .iter()
                .filter(|(b, _)| *b == bid)
                .flat_map(|(_, r)| std::iter::once(r.valid_from.clone()).chain(r.valid_to.clone()))
                .collect(),
        };
        dates.sort();
        dates.dedup();
        Ok(dates)
    }

    async fn list_branch_memberships_for_service(
        &self,
        service_id: i64,
    ) -> Result<Vec<BranchMembership>, RepositoryError> {
        let branches = self.branches.lock().unwrap_or_else(PoisonError::into_inner);
        let pins = self
            .branch_pins
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let members = self
            .branch_member_versions
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let name_of = |id: i64| {
            branches
                .iter()
                .find(|b| b.id == id)
                .map(|b| b.name.clone())
                .unwrap_or_default()
        };
        let mut rows: Vec<BranchMembership> = pins
            .iter()
            .filter(|(_, r)| r.service_id == service_id && r.valid_to.is_none())
            .map(|(bid, r)| BranchMembership {
                api_type: r.api_type,
                version: r.version,
                branch: name_of(*bid),
            })
            .chain(
                members
                    .iter()
                    .filter(|m| m.1 == service_id && m.5.is_none())
                    .map(|m| BranchMembership {
                        api_type: m.2,
                        version: m.3,
                        branch: name_of(m.0),
                    }),
            )
            .collect();
        rows.sort_by(|a, b| a.branch.cmp(&b.branch).then(a.version.cmp(&b.version)));
        rows.dedup_by(|a, b| {
            a.branch == b.branch && a.api_type == b.api_type && a.version == b.version
        });
        Ok(rows)
    }

    async fn list_branches_referencing(
        &self,
        service_id: i64,
        api_type: ApiType,
        version: SemVer,
    ) -> Result<Vec<String>, RepositoryError> {
        let branches = self.branches.lock().unwrap_or_else(PoisonError::into_inner);
        let pins = self
            .branch_pins
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let members = self
            .branch_member_versions
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let mut names: Vec<String> = branches
            .iter()
            .filter(|b| {
                pins.iter().any(|(bid, r)| {
                    *bid == b.id
                        && r.valid_to.is_none()
                        && r.service_id == service_id
                        && r.api_type == api_type
                        && r.version == version
                }) || members.iter().any(|m| {
                    m.0 == b.id
                        && m.5.is_none()
                        && m.1 == service_id
                        && m.2 == api_type
                        && m.3 == version
                })
            })
            .map(|b| b.name.clone())
            .collect();
        names.sort();
        Ok(names)
    }

    async fn close_expired_trunk_data(
        &self,
        cutoff_iso: &str,
        now_iso: &str,
    ) -> Result<u64, RepositoryError> {
        let mut closed = 0;
        for row in self
            .trunk_pins
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter_mut()
        {
            if row.valid_to.is_none() && row.last_required_at.as_str() < cutoff_iso {
                row.valid_to = Some(now_iso.to_string());
                closed += 1;
            }
        }
        for entry in self
            .spec_versions
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter_mut()
        {
            if entry
                .trunk_provided_at
                .as_deref()
                .is_some_and(|t| t < cutoff_iso)
            {
                entry.trunk_provided_at = None;
            }
        }
        Ok(closed)
    }

    async fn rename_branch(&self, branch_id: i64, new_name: &str) -> Result<(), RepositoryError> {
        let mut branches = self.branches.lock().unwrap_or_else(PoisonError::into_inner);
        if branches
            .iter()
            .any(|b| b.name == new_name && b.id != branch_id)
        {
            return Err(RepositoryError::Conflict);
        }
        if let Some(branch) = branches.iter_mut().find(|b| b.id == branch_id) {
            branch.name = new_name.to_string();
        }
        Ok(())
    }

    async fn delete_branch(&self, branch_id: i64) -> Result<(), RepositoryError> {
        self.branches
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .retain(|b| b.id != branch_id);
        self.branch_pins
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .retain(|(b, _)| *b != branch_id);
        self.branch_member_versions
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .retain(|r| r.0 != branch_id);
        Ok(())
    }

    async fn record_branch_pins(
        &self,
        branch_id: i64,
        pins: Vec<RecordTrunkPinParams<'_>>,
    ) -> Result<(), RepositoryError> {
        let mut rows = self
            .branch_pins
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        for p in pins {
            let same_key = |r: &&mut (i64, MockTrunkPin)| {
                r.0 == branch_id
                    && r.1.client_id == p.client_id
                    && r.1.service_id == p.service_id
                    && r.1.api_type == p.api_type
                    && r.1.normalized_path == p.normalized_path
                    && r.1.method == p.method
                    && r.1.valid_to.is_none()
            };
            if let Some(open) = rows.iter_mut().find(|r| same_key(r)) {
                if open.1.version == p.version {
                    open.1.last_required_at = p.now_iso.to_string();
                    continue;
                }
                open.1.valid_to = Some(p.now_iso.to_string());
            }
            rows.push((
                branch_id,
                MockTrunkPin {
                    client_id: p.client_id,
                    service_id: p.service_id,
                    api_type: p.api_type,
                    version: p.version,
                    path: p.path.to_string(),
                    normalized_path: p.normalized_path.to_string(),
                    method: p.method.to_string(),
                    valid_from: p.now_iso.to_string(),
                    last_required_at: p.now_iso.to_string(),
                    valid_to: None,
                },
            ));
        }
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
        let mut rows = self
            .branch_member_versions
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if let Some(open) = rows
            .iter_mut()
            .find(|r| r.0 == branch_id && r.1 == service_id && r.2 == api_type && r.5.is_none())
        {
            if open.3 == version {
                return Ok(());
            }
            open.5 = Some(now_iso.to_string());
        }
        rows.push((
            branch_id,
            service_id,
            api_type,
            version,
            now_iso.to_string(),
            None,
        ));
        Ok(())
    }

    async fn record_trunk_pins(
        &self,
        pins: Vec<RecordTrunkPinParams<'_>>,
    ) -> Result<(), RepositoryError> {
        let mut rows = self
            .trunk_pins
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        for p in pins {
            let same_key = |r: &&mut MockTrunkPin| {
                r.client_id == p.client_id
                    && r.service_id == p.service_id
                    && r.api_type == p.api_type
                    && r.normalized_path == p.normalized_path
                    && r.method == p.method
                    && r.valid_to.is_none()
            };
            if let Some(open) = rows.iter_mut().find(|r| same_key(r)) {
                if open.version == p.version {
                    open.last_required_at = p.now_iso.to_string();
                    continue;
                }
                open.valid_to = Some(p.now_iso.to_string());
            }
            rows.push(MockTrunkPin {
                client_id: p.client_id,
                service_id: p.service_id,
                api_type: p.api_type,
                version: p.version,
                path: p.path.to_string(),
                normalized_path: p.normalized_path.to_string(),
                method: p.method.to_string(),
                valid_from: p.now_iso.to_string(),
                last_required_at: p.now_iso.to_string(),
                valid_to: None,
            });
        }
        Ok(())
    }

    async fn list_current_trunk_pins(&self) -> Result<Vec<TrunkPinInfo>, RepositoryError> {
        let rows = self
            .trunk_pins
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let clients = self.clients.lock().unwrap_or_else(PoisonError::into_inner);
        let services = self.services.lock().unwrap_or_else(PoisonError::into_inner);
        Ok(rows
            .iter()
            .filter(|r| r.valid_to.is_none())
            .map(|r| TrunkPinInfo {
                client: clients
                    .iter()
                    .find(|(_, id)| **id == r.client_id)
                    .map(|(name, _)| name.clone())
                    .unwrap_or_default(),
                service: services
                    .iter()
                    .find(|(_, id)| **id == r.service_id)
                    .map(|(name, _)| name.clone())
                    .unwrap_or_default(),
                api_type: r.api_type,
                version: r.version,
                path: r.path.clone(),
                normalized_path: r.normalized_path.clone(),
                method: r.method.clone(),
                valid_from: r.valid_from.clone(),
                last_required_at: r.last_required_at.clone(),
                dangling: false,
            })
            .collect())
    }

    async fn delete_expired_snapshots(&self, cutoff_iso: &str) -> Result<u64, RepositoryError> {
        let mut versions = self
            .spec_versions
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let mut deleted_ids = Vec::new();
        versions.retain(|v| {
            let expired = v.stability == Stability::Snapshot
                && v.updated_at.as_str() < cutoff_iso
                && v.last_required_at
                    .as_deref()
                    .is_none_or(|required| required < cutoff_iso);
            if expired {
                deleted_ids.push(v.id);
            }
            !expired
        });
        if !deleted_ids.is_empty() {
            self.dependencies
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .retain(|d| !deleted_ids.contains(&d.spec_version_id));
        }
        Ok(deleted_ids.len() as u64)
    }

    async fn list_version_dependents(
        &self,
        spec_version_id: i64,
    ) -> Result<Vec<String>, RepositoryError> {
        let dependencies = self
            .dependencies
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let client_ids: Vec<i64> = dependencies
            .iter()
            .filter(|d| d.spec_version_id == spec_version_id)
            .map(|d| d.client_id)
            .collect();
        drop(dependencies);
        let mut names: Vec<String> = client_ids
            .into_iter()
            .filter_map(|id| self.client_name(id))
            .collect();
        names.sort();
        names.dedup();
        Ok(names)
    }

    async fn ensure_service(&self, name: &str) -> Result<i64, RepositoryError> {
        let mut services = self.services.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(&id) = services.get(name) {
            return Ok(id);
        }
        let id = self.next_id();
        services.insert(name.to_string(), id);
        Ok(id)
    }

    async fn find_service(&self, name: &str) -> Result<Option<i64>, RepositoryError> {
        let services = self.services.lock().unwrap_or_else(PoisonError::into_inner);
        Ok(services.get(name).copied())
    }

    async fn get_service_name_by_id(
        &self,
        service_id: i64,
    ) -> Result<Option<String>, RepositoryError> {
        Ok(self.service_name(service_id))
    }

    async fn get_endpoints_for_version(
        &self,
        spec_version_id: i64,
    ) -> Result<Vec<EndpointRecord>, RepositoryError> {
        let versions = self
            .spec_versions
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        Ok(versions
            .iter()
            .find(|v| v.id == spec_version_id)
            .map(|v| v.endpoints.clone())
            .unwrap_or_default())
    }

    async fn ensure_client(&self, name: &str) -> Result<i64, RepositoryError> {
        let mut clients = self.clients.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(&id) = clients.get(name) {
            return Ok(id);
        }
        let id = self.next_id();
        clients.insert(name.to_string(), id);
        Ok(id)
    }

    async fn find_endpoint(
        &self,
        spec_version_id: i64,
        api_type: ApiType,
        path: &str,
        method: &str,
    ) -> Result<Option<(i64, String, bool)>, RepositoryError> {
        let normalized_path = crate::openapi::lookup_path(api_type, path);
        let versions = self
            .spec_versions
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        Ok(versions
            .iter()
            .find(|v| v.id == spec_version_id)
            .and_then(|v| {
                v.endpoints.iter().find(|e| {
                    e.api_type == api_type
                        && e.normalized_path == normalized_path
                        && e.method == method
                })
            })
            .map(|e| {
                (
                    e.id.unwrap_or_default(),
                    e.yaml_content.clone(),
                    e.deprecated,
                )
            }))
    }

    async fn find_endpoints_bulk(
        &self,
        spec_version_id: i64,
        api_type: ApiType,
        endpoints: &[(String, String)],
    ) -> Result<EndpointMap, RepositoryError> {
        let versions = self
            .spec_versions
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let mut result = EndpointMap::new();
        let Some(version) = versions.iter().find(|v| v.id == spec_version_id) else {
            return Ok(result);
        };
        // Key the result by what the caller asked for, matched via the
        // normalized path, so lenient path matching works for bundles too.
        for (path, method) in endpoints {
            let normalized = crate::openapi::lookup_path(api_type, path);
            if let Some(found) = version.endpoints.iter().find(|e| {
                e.api_type == api_type && e.normalized_path == normalized && e.method == *method
            }) {
                result.insert(
                    (path.clone(), method.clone()),
                    (
                        found.id.unwrap_or_default(),
                        found.yaml_content.clone(),
                        found.deprecated,
                    ),
                );
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

    async fn record_dependencies_bulk(
        &self,
        params: Vec<RecordDependencyParams<'_>>,
    ) -> Result<(), RepositoryError> {
        let now = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();
        let mut dependencies = self
            .dependencies
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        for p in params {
            if let Some(existing) = dependencies.iter_mut().find(|d| {
                d.client_id == p.client_id
                    && d.spec_version_id == p.spec_version_id
                    && d.api_type == p.api_type
                    && d.path == p.path
                    && d.method == p.method
            }) {
                existing.last_seen_at = now.clone();
            } else {
                dependencies.push(MockDependency {
                    client_id: p.client_id,
                    spec_version_id: p.spec_version_id,
                    api_type: p.api_type,
                    path: p.path.to_string(),
                    normalized_path: p.normalized_path.to_string(),
                    method: p.method.to_string(),
                    last_seen_at: now.clone(),
                });
            }
        }
        Ok(())
    }

    async fn get_report(&self) -> Result<DependencyReport, RepositoryError> {
        let versions = self
            .spec_versions
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        let dependencies = self
            .dependencies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();

        let mut dependency_graph = Vec::new();
        let mut missing_endpoints = Vec::new();
        for dep in &dependencies {
            let Some(version) = versions.iter().find(|v| v.id == dep.spec_version_id) else {
                continue;
            };
            let client = self.client_name(dep.client_id).unwrap_or_default();
            let service = self.service_name(version.service_id).unwrap_or_default();
            let endpoint = version.endpoints.iter().find(|e| {
                e.api_type == dep.api_type
                    && e.normalized_path == dep.normalized_path
                    && e.method == dep.method
            });
            match endpoint {
                Some(endpoint) => dependency_graph.push(DependencyInfo {
                    api_type: dep.api_type,
                    client,
                    service,
                    version: version.version,
                    stability: version.stability,
                    path: dep.path.clone(),
                    method: dep.method.clone(),
                    deprecated: endpoint.deprecated,
                }),
                None => missing_endpoints.push(MissingEndpointInfo {
                    api_type: dep.api_type,
                    client,
                    service,
                    version: version.version,
                    path: dep.path.clone(),
                    method: dep.method.clone(),
                }),
            }
        }

        let mut unused_endpoints = Vec::new();
        for version in &versions {
            let service = self.service_name(version.service_id).unwrap_or_default();
            for endpoint in &version.endpoints {
                let required = dependencies.iter().any(|d| {
                    d.spec_version_id == version.id
                        && d.api_type == endpoint.api_type
                        && d.normalized_path == endpoint.normalized_path
                        && d.method == endpoint.method
                });
                if !required {
                    unused_endpoints.push(EndpointInfo {
                        api_type: endpoint.api_type,
                        service: service.clone(),
                        version: version.version,
                        stability: version.stability,
                        path: endpoint.path.clone(),
                        method: endpoint.method.clone(),
                        deprecated: endpoint.deprecated,
                    });
                }
            }
        }

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
        let mut services = self.services.lock().unwrap_or_else(PoisonError::into_inner);
        let count = services.len() as u64;
        services.clear();
        drop(services);
        self.spec_versions
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clear();
        self.dependencies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clear();
        Ok(count)
    }

    async fn delete_all_clients(&self) -> Result<u64, RepositoryError> {
        let mut clients = self.clients.lock().unwrap_or_else(PoisonError::into_inner);
        let count = clients.len() as u64;
        clients.clear();
        drop(clients);
        self.dependencies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clear();
        Ok(count)
    }

    async fn delete_all_non_admin_users(
        &self,
        spare_usernames: &[String],
    ) -> Result<u64, RepositoryError> {
        let mut admins: Vec<i64> = self
            .user_roles
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .filter(|(_, role)| role == "admin")
            .map(|(user_id, _)| *user_id)
            .collect();
        let admin_groups: Vec<i64> = self
            .group_roles
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .filter(|(_, role)| role == "admin")
            .map(|(group_id, _)| *group_id)
            .collect();
        admins.extend(
            self.group_members
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .iter()
                .filter(|(group_id, _)| admin_groups.contains(group_id))
                .map(|(_, user_id)| *user_id),
        );
        let mut users = self.users.lock().unwrap_or_else(PoisonError::into_inner);
        let initial_count = users.len() as u64;
        users.retain(|u| admins.contains(&u.id) || spare_usernames.contains(&u.username));
        Ok(initial_count - users.len() as u64)
    }

    async fn nuke_database(&self, _keep_user_id: Option<i64>) -> Result<(), RepositoryError> {
        self.delete_all_services().await?;
        self.delete_all_clients().await?;
        self.channel_message_contracts
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clear();
        Ok(())
    }

    async fn delete_producer(&self, name: &str) -> Result<bool, RepositoryError> {
        let mut services = self.services.lock().unwrap_or_else(PoisonError::into_inner);
        let Some(service_id) = services.remove(name) else {
            return Ok(false);
        };
        drop(services);
        let mut versions = self
            .spec_versions
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let deleted_ids: Vec<i64> = versions
            .iter()
            .filter(|v| v.service_id == service_id)
            .map(|v| v.id)
            .collect();
        versions.retain(|v| v.service_id != service_id);
        drop(versions);
        self.dependencies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .retain(|d| !deleted_ids.contains(&d.spec_version_id));
        Ok(true)
    }

    async fn delete_consumer(&self, name: &str) -> Result<bool, RepositoryError> {
        let mut clients = self.clients.lock().unwrap_or_else(PoisonError::into_inner);
        let Some(client_id) = clients.remove(name) else {
            return Ok(false);
        };
        drop(clients);
        self.dependencies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .retain(|d| d.client_id != client_id);
        Ok(true)
    }

    async fn list_producers(&self) -> Result<Vec<String>, RepositoryError> {
        let mut names: Vec<String> = self
            .services
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .keys()
            .cloned()
            .collect();
        names.sort();
        Ok(names)
    }

    async fn list_producers_detailed(&self) -> Result<Vec<ProducerSummary>, RepositoryError> {
        let services = self.services.lock().unwrap_or_else(PoisonError::into_inner);
        let metadata = self
            .producer_metadata
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let mut result: Vec<ProducerSummary> = services
            .keys()
            .map(|name| {
                let (icon, domain) = metadata.get(name).cloned().unwrap_or((None, None));
                ProducerSummary {
                    name: name.clone(),
                    versions: Vec::new(),
                    is_favorite: false,
                    icon,
                    domain,
                }
            })
            .collect();
        result.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(result)
    }

    async fn update_producer_metadata(
        &self,
        service_name: &str,
        icon: Option<&str>,
        domain: Option<&str>,
    ) -> Result<(), RepositoryError> {
        self.producer_metadata
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(
                service_name.to_string(),
                (icon.map(|i| i.to_string()), domain.map(|d| d.to_string())),
            );
        Ok(())
    }

    async fn list_consumers(&self) -> Result<Vec<String>, RepositoryError> {
        let mut clients: Vec<String> = self
            .clients
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .keys()
            .cloned()
            .collect();
        clients.sort();
        Ok(clients)
    }

    async fn list_consumer_endpoints(
        &self,
        client_name: &str,
    ) -> Result<Vec<ConsumerEndpointInfo>, RepositoryError> {
        let Some(client_id) = self
            .clients
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(client_name)
            .copied()
        else {
            return Ok(Vec::new());
        };
        let dependencies = self
            .dependencies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        let versions = self
            .spec_versions
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();

        let mut result = Vec::new();
        for dep in dependencies.iter().filter(|d| d.client_id == client_id) {
            let Some(version) = versions.iter().find(|v| v.id == dep.spec_version_id) else {
                continue;
            };
            let endpoint = version.endpoints.iter().find(|e| {
                e.api_type == dep.api_type
                    && e.normalized_path == dep.normalized_path
                    && e.method == dep.method
            });
            result.push(ConsumerEndpointInfo {
                api_type: dep.api_type,
                service: self.service_name(version.service_id).unwrap_or_default(),
                version: version.version,
                stability: version.stability,
                path: dep.path.clone(),
                method: dep.method.clone(),
                yaml_content: endpoint.map(|e| e.yaml_content.clone()),
                deprecated: endpoint.map(|e| e.deprecated).unwrap_or(false),
            });
        }
        result.sort_by(|a, b| {
            a.service
                .cmp(&b.service)
                .then_with(|| a.path.cmp(&b.path))
                .then_with(|| a.method.cmp(&b.method))
        });
        Ok(result)
    }

    async fn user_count(&self) -> Result<i64, RepositoryError> {
        Ok(self
            .users
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len() as i64)
    }

    async fn find_user(&self, username: &str) -> Result<Option<User>, RepositoryError> {
        Ok(self
            .users
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .find(|u| u.username == username)
            .cloned())
    }

    async fn create_user(
        &self,
        username: &str,
        password_hash: &str,
        approved: bool,
    ) -> Result<User, RepositoryError> {
        let user = User {
            id: self.next_id(),
            username: username.to_string(),
            password_hash: password_hash.to_string(),
            approved,
        };
        self.users
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(user.clone());
        Ok(user)
    }

    async fn update_password(&self, user_id: i64, new_hash: &str) -> Result<(), RepositoryError> {
        let mut users = self.users.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(u) = users.iter_mut().find(|u| u.id == user_id) {
            u.password_hash = new_hash.to_string();
        }
        Ok(())
    }

    async fn list_users(&self) -> Result<Vec<User>, RepositoryError> {
        Ok(self
            .users
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone())
    }

    async fn approve_user(&self, user_id: i64) -> Result<bool, RepositoryError> {
        let mut users = self.users.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(u) = users.iter_mut().find(|u| u.id == user_id) {
            u.approved = true;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    async fn delete_user(&self, user_id: i64) -> Result<bool, RepositoryError> {
        let mut users = self.users.lock().unwrap_or_else(PoisonError::into_inner);
        let initial_len = users.len();
        users.retain(|u| u.id != user_id);
        Ok(users.len() < initial_len)
    }

    async fn create_session(
        &self,
        user_id: i64,
        expires_at: &str,
    ) -> Result<Session, RepositoryError> {
        let session = Session {
            user_id,
            token: "mock-token".to_string(),
            expires_at: expires_at.to_string(),
        };
        self.sessions
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(session.clone());
        Ok(session)
    }

    async fn create_session_with_token(
        &self,
        user_id: i64,
        token: &str,
        expires_at: &str,
    ) -> Result<Session, RepositoryError> {
        let session = Session {
            user_id,
            token: token.to_string(),
            expires_at: expires_at.to_string(),
        };
        self.sessions
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(session.clone());
        Ok(session)
    }

    async fn validate_session(
        &self,
        token: &str,
    ) -> Result<Option<(User, Session)>, RepositoryError> {
        let sessions = self.sessions.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(s) = sessions.iter().find(|s| s.token == token) {
            let users = self.users.lock().unwrap_or_else(PoisonError::into_inner);
            if let Some(u) = users.iter().find(|u| u.id == s.user_id) {
                return Ok(Some((u.clone(), s.clone())));
            }
        }
        Ok(None)
    }

    async fn delete_session(&self, token: &str) -> Result<(), RepositoryError> {
        let mut sessions = self.sessions.lock().unwrap_or_else(PoisonError::into_inner);
        sessions.retain(|s| s.token != token);
        Ok(())
    }

    async fn get_setting(&self, key: &str) -> Result<Option<String>, RepositoryError> {
        Ok(self
            .settings
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(key)
            .cloned())
    }

    async fn set_setting(&self, key: &str, value: &str) -> Result<(), RepositoryError> {
        self.settings
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(key.to_string(), value.to_string());
        Ok(())
    }

    async fn create_api_token(
        &self,
        _id: &str,
        _user_id: i64,
        _name: &str,
        _hash: &str,
        _created: &str,
        _expires: &str,
    ) -> Result<(), RepositoryError> {
        Ok(())
    }

    async fn list_api_tokens(&self, _user_id: i64) -> Result<Vec<ApiToken>, RepositoryError> {
        Ok(Vec::new())
    }

    async fn delete_api_token(
        &self,
        _token_id: &str,
        _user_id: i64,
    ) -> Result<bool, RepositoryError> {
        Ok(true)
    }

    async fn validate_api_token(&self, _hash: &str) -> Result<Option<User>, RepositoryError> {
        Ok(None)
    }

    async fn delete_stale_dependencies(&self, cutoff_iso: &str) -> Result<u64, RepositoryError> {
        let mut dependencies = self
            .dependencies
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let before = dependencies.len();
        dependencies.retain(|d| d.last_seen_at.as_str() >= cutoff_iso);
        Ok((before - dependencies.len()) as u64)
    }

    // --- Roles and Groups ---

    async fn grant_user_role(&self, user_id: i64, role: &str) -> Result<(), RepositoryError> {
        let mut roles = self
            .user_roles
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let entry = (user_id, role.to_string());
        if !roles.contains(&entry) {
            roles.push(entry);
        }
        Ok(())
    }

    async fn revoke_user_role(&self, user_id: i64, role: &str) -> Result<bool, RepositoryError> {
        let mut roles = self
            .user_roles
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let before = roles.len();
        roles.retain(|(id, r)| !(*id == user_id && r == role));
        Ok(roles.len() != before)
    }

    async fn list_user_roles(&self, user_id: i64) -> Result<Vec<String>, RepositoryError> {
        let roles = self
            .user_roles
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let mut out: Vec<String> = roles
            .iter()
            .filter(|(id, _)| *id == user_id)
            .map(|(_, r)| r.clone())
            .collect();
        out.sort();
        Ok(out)
    }

    async fn effective_stored_roles(&self, user_id: i64) -> Result<Vec<String>, RepositoryError> {
        let mut out: Vec<String> = self.list_user_roles(user_id).await?;
        let members = self
            .group_members
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let group_roles = self
            .group_roles
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        for (group_id, member_id) in members.iter() {
            if *member_id != user_id {
                continue;
            }
            for (gid, role) in group_roles.iter() {
                if gid == group_id && !out.contains(role) {
                    out.push(role.clone());
                }
            }
        }
        out.sort();
        out.dedup();
        Ok(out)
    }

    async fn create_group(
        &self,
        name: &str,
        source: GroupSource,
    ) -> Result<Group, RepositoryError> {
        let mut groups = self.groups.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(existing) = groups.iter().find(|g| g.name == name && g.source == source) {
            return Ok(existing.clone());
        }
        let group = Group {
            id: self.next_id(),
            name: name.to_string(),
            source,
        };
        groups.push(group.clone());
        Ok(group)
    }

    async fn rename_group(&self, group_id: i64, name: &str) -> Result<bool, RepositoryError> {
        let mut groups = self.groups.lock().unwrap_or_else(PoisonError::into_inner);
        match groups.iter_mut().find(|g| g.id == group_id) {
            Some(group) => {
                group.name = name.to_string();
                Ok(true)
            }
            None => Ok(false),
        }
    }

    async fn delete_group(&self, group_id: i64) -> Result<bool, RepositoryError> {
        let mut groups = self.groups.lock().unwrap_or_else(PoisonError::into_inner);
        let before = groups.len();
        groups.retain(|g| g.id != group_id);
        self.group_members
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .retain(|(gid, _)| *gid != group_id);
        self.group_roles
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .retain(|(gid, _)| *gid != group_id);
        Ok(groups.len() != before)
    }

    async fn list_groups(&self) -> Result<Vec<Group>, RepositoryError> {
        let groups = self.groups.lock().unwrap_or_else(PoisonError::into_inner);
        Ok(groups.clone())
    }

    async fn set_group_roles(
        &self,
        group_id: i64,
        roles: &[String],
    ) -> Result<(), RepositoryError> {
        let mut group_roles = self
            .group_roles
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        group_roles.retain(|(gid, _)| *gid != group_id);
        for role in roles {
            group_roles.push((group_id, role.clone()));
        }
        Ok(())
    }

    async fn list_group_roles(&self, group_id: i64) -> Result<Vec<String>, RepositoryError> {
        let group_roles = self
            .group_roles
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let mut out: Vec<String> = group_roles
            .iter()
            .filter(|(gid, _)| *gid == group_id)
            .map(|(_, r)| r.clone())
            .collect();
        out.sort();
        Ok(out)
    }

    async fn add_group_member(&self, group_id: i64, user_id: i64) -> Result<(), RepositoryError> {
        let mut members = self
            .group_members
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if !members.contains(&(group_id, user_id)) {
            members.push((group_id, user_id));
        }
        Ok(())
    }

    async fn remove_group_member(
        &self,
        group_id: i64,
        user_id: i64,
    ) -> Result<bool, RepositoryError> {
        let mut members = self
            .group_members
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let before = members.len();
        members.retain(|entry| *entry != (group_id, user_id));
        Ok(members.len() != before)
    }

    async fn list_group_member_ids(&self, group_id: i64) -> Result<Vec<i64>, RepositoryError> {
        let members = self
            .group_members
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let mut out: Vec<i64> = members
            .iter()
            .filter(|(gid, _)| *gid == group_id)
            .map(|(_, uid)| *uid)
            .collect();
        out.sort();
        Ok(out)
    }

    // --- Maintainer Scope ---

    async fn add_user_maintainer(
        &self,
        service_id: i64,
        user_id: i64,
    ) -> Result<(), RepositoryError> {
        let mut m = self
            .user_maintainers
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if !m.contains(&(service_id, user_id)) {
            m.push((service_id, user_id));
        }
        Ok(())
    }

    async fn remove_user_maintainer(
        &self,
        service_id: i64,
        user_id: i64,
    ) -> Result<bool, RepositoryError> {
        let mut m = self
            .user_maintainers
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let before = m.len();
        m.retain(|entry| *entry != (service_id, user_id));
        Ok(m.len() != before)
    }

    async fn add_group_maintainer(
        &self,
        service_id: i64,
        group_id: i64,
    ) -> Result<(), RepositoryError> {
        let mut m = self
            .group_maintainers
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if !m.contains(&(service_id, group_id)) {
            m.push((service_id, group_id));
        }
        Ok(())
    }

    async fn remove_group_maintainer(
        &self,
        service_id: i64,
        group_id: i64,
    ) -> Result<bool, RepositoryError> {
        let mut m = self
            .group_maintainers
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let before = m.len();
        m.retain(|entry| *entry != (service_id, group_id));
        Ok(m.len() != before)
    }

    async fn list_user_maintainer_ids(&self, service_id: i64) -> Result<Vec<i64>, RepositoryError> {
        let m = self
            .user_maintainers
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let mut out: Vec<i64> = m
            .iter()
            .filter(|(sid, _)| *sid == service_id)
            .map(|(_, uid)| *uid)
            .collect();
        out.sort();
        Ok(out)
    }

    async fn list_group_maintainer_ids(
        &self,
        service_id: i64,
    ) -> Result<Vec<i64>, RepositoryError> {
        let m = self
            .group_maintainers
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let mut out: Vec<i64> = m
            .iter()
            .filter(|(sid, _)| *sid == service_id)
            .map(|(_, gid)| *gid)
            .collect();
        out.sort();
        Ok(out)
    }

    async fn list_all_user_maintainers(&self) -> Result<Vec<(String, i64)>, RepositoryError> {
        let m = self
            .user_maintainers
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let mut out: Vec<(String, i64)> = m
            .iter()
            .filter_map(|(sid, uid)| self.service_name(*sid).map(|name| (name, *uid)))
            .collect();
        out.sort();
        Ok(out)
    }

    async fn list_all_group_maintainers(&self) -> Result<Vec<(String, i64)>, RepositoryError> {
        let m = self
            .group_maintainers
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let mut out: Vec<(String, i64)> = m
            .iter()
            .filter_map(|(sid, gid)| self.service_name(*sid).map(|name| (name, *gid)))
            .collect();
        out.sort();
        Ok(out)
    }

    async fn list_group_maintained_producers(
        &self,
        group_ids: &[i64],
    ) -> Result<Vec<String>, RepositoryError> {
        let maintainers = self
            .group_maintainers
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let mut out: Vec<String> = maintainers
            .iter()
            .filter(|(_, gid)| group_ids.contains(gid))
            .filter_map(|(sid, _)| self.service_name(*sid))
            .collect();
        out.sort();
        out.dedup();
        Ok(out)
    }

    async fn maintains_producer(
        &self,
        user_id: i64,
        service_id: i64,
    ) -> Result<bool, RepositoryError> {
        if self
            .user_maintainers
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .contains(&(service_id, user_id))
        {
            return Ok(true);
        }
        let group_maintainers = self
            .group_maintainers
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let members = self
            .group_members
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        Ok(group_maintainers
            .iter()
            .any(|(sid, gid)| *sid == service_id && members.contains(&(*gid, user_id))))
    }

    async fn list_maintained_producers(
        &self,
        user_id: i64,
    ) -> Result<Vec<String>, RepositoryError> {
        let services = self.services.lock().unwrap_or_else(PoisonError::into_inner);
        let mut out = Vec::new();
        for (name, sid) in services.iter() {
            let maintained = {
                let direct = self
                    .user_maintainers
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .contains(&(*sid, user_id));
                if direct {
                    true
                } else {
                    let group_maintainers = self
                        .group_maintainers
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner);
                    let members = self
                        .group_members
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner);
                    group_maintainers
                        .iter()
                        .any(|(msid, gid)| msid == sid && members.contains(&(*gid, user_id)))
                }
            };
            if maintained {
                out.push(name.clone());
            }
        }
        out.sort();
        Ok(out)
    }

    // --- Service Tags ---

    async fn add_service_tags(
        &self,
        service_id: i64,
        tags: &[String],
    ) -> Result<(), RepositoryError> {
        let mut st = self
            .service_tags
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        st.entry(service_id)
            .or_default()
            .extend(tags.iter().cloned());
        Ok(())
    }

    async fn get_all_service_tags(&self) -> Result<HashMap<String, Vec<String>>, RepositoryError> {
        Ok(HashMap::new())
    }

    // --- Audit Logs ---

    async fn insert_audit_log(
        &self,
        username: &str,
        log: NewAuditLog<'_>,
    ) -> Result<(), RepositoryError> {
        let mut logs = self
            .audit_logs
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let id = self.next_id();
        let timestamp = chrono::Utc::now().to_rfc3339();
        logs.push(AuditLogEntry {
            id,
            timestamp,
            username: username.to_string(),
            action: log.action.to_string(),
            details: log.details.to_string(),
            service: log.service.map(|s| s.to_string()),
            version: log.version.map(|v| v.to_string()),
            action_type: log.action_type.map(|t| t.to_string()),
            diff: log.diff.map(|d| d.to_string()),
            stream: log.stream.map(|s| s.to_string()),
        });
        Ok(())
    }

    async fn get_audit_logs(
        &self,
        _filter: AuditLogFilter,
    ) -> Result<Vec<AuditLogEntry>, RepositoryError> {
        let logs = self
            .audit_logs
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let mut cloned = logs.clone();
        cloned.reverse(); // id DESC order (newest first)
        // Simplified mock filtering could be added here if needed for tests
        Ok(cloned)
    }

    async fn get_recent_audit_logs(
        &self,
        limit: u32,
    ) -> Result<Vec<AuditLogEntry>, RepositoryError> {
        let logs = self
            .audit_logs
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let mut cloned = logs.clone();
        cloned.reverse(); // id DESC order (newest first)
        cloned.truncate(limit as usize);
        Ok(cloned)
    }

    // --- User Favorites ---

    async fn get_user_favorites(
        &self,
        user_id: i64,
        item_type: &str,
    ) -> Result<Vec<String>, RepositoryError> {
        let favorites = self
            .user_favorites
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let mut result: Vec<String> = favorites
            .iter()
            .filter(|(uid, t, _)| *uid == user_id && t == item_type)
            .map(|(_, _, name)| name.clone())
            .collect();
        result.sort();
        Ok(result)
    }

    async fn add_user_favorite(
        &self,
        user_id: i64,
        item_type: &str,
        item_name: &str,
    ) -> Result<(), RepositoryError> {
        let mut favorites = self
            .user_favorites
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if !favorites
            .iter()
            .any(|(uid, t, name)| *uid == user_id && t == item_type && name == item_name)
        {
            favorites.push((user_id, item_type.to_string(), item_name.to_string()));
        }
        Ok(())
    }

    async fn remove_user_favorite(
        &self,
        user_id: i64,
        item_type: &str,
        item_name: &str,
    ) -> Result<(), RepositoryError> {
        let mut favorites = self
            .user_favorites
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        favorites
            .retain(|(uid, t, name)| !(*uid == user_id && t == item_type && name == item_name));
        Ok(())
    }

    // --- AsyncAPI Channel Message Contracts ---

    async fn get_channel_message_contract(
        &self,
        channel: &str,
        message_name: &str,
    ) -> Result<Option<ChannelMessageContract>, RepositoryError> {
        let contracts = self
            .channel_message_contracts
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        Ok(contracts
            .iter()
            .find(|c| c.channel == channel && c.message_name == message_name)
            .cloned())
    }

    async fn upsert_channel_message_contract(
        &self,
        contract: &ChannelMessageContract,
    ) -> Result<(), RepositoryError> {
        let mut contracts = self
            .channel_message_contracts
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        match contracts
            .iter_mut()
            .find(|c| c.channel == contract.channel && c.message_name == contract.message_name)
        {
            Some(existing) => {
                existing.owner_service_id = contract.owner_service_id;
                existing.payload_yaml = contract.payload_yaml.clone();
            }
            None => contracts.push(contract.clone()),
        }
        Ok(())
    }

    async fn delete_channel_message_contract(
        &self,
        channel: &str,
        message_name: &str,
    ) -> Result<(), RepositoryError> {
        let mut contracts = self
            .channel_message_contracts
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        contracts.retain(|c| !(c.channel == channel && c.message_name == message_name));
        Ok(())
    }

    async fn list_channel_message_contracts(
        &self,
    ) -> Result<Vec<ChannelMessageContract>, RepositoryError> {
        let contracts = self
            .channel_message_contracts
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let mut result: Vec<ChannelMessageContract> = contracts.clone();
        result.sort_by(|a, b| {
            a.channel
                .cmp(&b.channel)
                .then_with(|| a.message_name.cmp(&b.message_name))
        });
        Ok(result)
    }
}
