use crate::domain::models::*;
use crate::domain::ports::{
    EndpointMap, NewApiToken, NewAuditLog, NewWebhook, RecordDependencyParams,
    RecordTrunkPinParams, RepositoryError, RewriteSpecText, SpecRepository, UpsertSpecVersion,
};
use std::collections::{BTreeMap, HashMap};
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

/// One trunk member-version record, mirroring a `trunk_member_versions` row.
#[derive(Clone, Debug)]
pub struct MockTrunkMember {
    pub service_id: i64,
    pub service_name: String,
    pub api_type: ApiType,
    pub version: SemVer,
    pub valid_from: String,
    pub last_provided_at: String,
    pub valid_to: Option<String>,
}

pub struct MockRepo {
    pub services: Mutex<HashMap<String, i64>>,
    pub producer_metadata: Mutex<HashMap<String, ProducerMetadata>>,
    pub spec_versions: Mutex<Vec<MockSpecVersion>>,
    pub dependencies: Mutex<Vec<MockDependency>>,
    pub trunk_pins: Mutex<Vec<MockTrunkPin>>,
    pub branches: Mutex<Vec<BranchInfo>>,
    pub branch_pins: Mutex<Vec<(i64, MockTrunkPin)>>,
    pub branch_member_versions: Mutex<Vec<BranchMemberVersion>>,
    pub trunk_members: Mutex<Vec<MockTrunkMember>>,
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
    pub harvested_subscriptions: Mutex<Vec<MockHarvestedSub>>,
    pub periphery: Mutex<Vec<MockPeripheryRow>>,
    /// In-memory `system_context` rows (#48), by system name.
    pub system_contexts: Mutex<BTreeMap<String, SystemContext>>,
    pub api_tokens: Mutex<Vec<ApiToken>>,
    pub webhooks: Mutex<Vec<Webhook>>,
    pub webhook_events: Mutex<Vec<MockWebhookEvent>>,
    pub webhook_deliveries: Mutex<Vec<WebhookDelivery>>,
}

/// One outbox row (#47): the event and its stored `data`.
#[derive(Clone, Debug)]
pub struct MockWebhookEvent {
    pub id: String,
    pub event_type: WebhookEventType,
    pub producer: Option<String>,
    pub occurred_at: String,
    pub data: serde_json::Value,
}

/// In-memory `peripheral_dependencies` row (#46). Append-only: the open row
/// (`valid_to` is `None`) per (track, system, resource) is current.
#[derive(Clone)]
pub struct MockPeripheryRow {
    pub track: PeripheryTrack,
    pub system: String,
    pub resource: String,
    pub valid_from: String,
    pub valid_to: Option<String>,
}

/// In-memory harvested subscription row (ai/improvements.md #6). Append-only:
/// the open row (`valid_to` is `None`) per (client_id, channel, message) is
/// current.
#[derive(Clone)]
pub struct MockHarvestedSub {
    pub client_id: i64,
    pub client_name: String,
    pub channel: String,
    pub message_name: String,
    pub owner_service_id: Option<i64>,
    pub owner_name: Option<String>,
    pub trunk: bool,
    pub valid_from: String,
    pub valid_to: Option<String>,
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
            trunk_members: Mutex::new(Vec::new()),
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
            harvested_subscriptions: Mutex::new(Vec::new()),
            periphery: Mutex::new(Vec::new()),
            system_contexts: Mutex::new(BTreeMap::new()),
            api_tokens: Mutex::new(Vec::new()),
            webhooks: Mutex::new(Vec::new()),
            webhook_events: Mutex::new(Vec::new()),
            webhook_deliveries: Mutex::new(Vec::new()),
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

    /// Copy the periphery rows of `source` valid at `as_of` into a branch's
    /// set, opened at `now_iso` — the periphery half of a branch cut (#46).
    fn copy_periphery(&self, source: PeripheryTrack, branch_id: i64, as_of: &str, now_iso: &str) {
        let mut rows = self
            .periphery
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let copied: Vec<MockPeripheryRow> = rows
            .iter()
            .filter(|r| {
                r.track == source
                    && r.valid_from.as_str() <= as_of
                    && r.valid_to.as_deref().is_none_or(|to| to > as_of)
            })
            .map(|r| MockPeripheryRow {
                track: PeripheryTrack::Branch(branch_id),
                system: r.system.clone(),
                resource: r.resource.clone(),
                valid_from: now_iso.to_string(),
                valid_to: None,
            })
            .collect();
        rows.extend(copied);
    }

    /// The in-memory twin of the repositories' `write_outbox` (#47): one
    /// event row plus one pending delivery per hook id.
    fn write_outbox(&self, events: Vec<NewWebhookEvent>) {
        for event in events {
            let mut deliveries = self
                .webhook_deliveries
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            for hook_id in &event.hook_ids {
                deliveries.push(WebhookDelivery {
                    id: self.next_id(),
                    hook_id: *hook_id,
                    event_id: event.id.clone(),
                    event_type: event.event_type,
                    occurred_at: event.occurred_at.clone(),
                    status: WebhookDeliveryStatus::Pending,
                    attempts: 0,
                    next_attempt_at: event.occurred_at.clone(),
                    last_response_code: None,
                    last_error: None,
                    delivered_at: None,
                    updated_at: event.occurred_at.clone(),
                });
            }
            drop(deliveries);
            self.webhook_events
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(MockWebhookEvent {
                    id: event.id,
                    event_type: event.event_type,
                    producer: event.producer,
                    occurred_at: event.occurred_at,
                    data: event.data,
                });
        }
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
            let id = existing.id;
            drop(versions);
            self.write_outbox(params.outbox);
            return Ok(id);
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
        drop(versions);
        self.write_outbox(params.outbox);
        Ok(id)
    }

    async fn rewrite_spec_text(&self, params: RewriteSpecText<'_>) -> Result<(), RepositoryError> {
        let mut versions = self
            .spec_versions
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let Some(entry) = versions.iter_mut().find(|v| v.id == params.spec_version_id) else {
            return Ok(());
        };
        entry.content = params.content.to_string();
        entry.content_hash = params.content_hash.to_string();
        for (endpoint_id, text) in params.endpoint_texts {
            if let Some(endpoint) = entry
                .endpoints
                .iter_mut()
                .find(|e| e.id == Some(*endpoint_id))
            {
                endpoint.yaml_content = text.clone();
            }
        }
        Ok(())
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
        let Some(entry) = versions.iter_mut().find(|v| v.id == spec_version_id) else {
            return Ok(());
        };
        entry.trunk_provided_at = Some(now_iso.to_string());
        let (service_id, api_type, version) = (entry.service_id, entry.api_type, entry.version);
        drop(versions);
        let service_name = self.service_name(service_id).unwrap_or_default();
        let mut members = self
            .trunk_members
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if let Some(open) = members
            .iter_mut()
            .find(|m| m.service_id == service_id && m.api_type == api_type && m.valid_to.is_none())
        {
            if open.version == version {
                open.last_provided_at = now_iso.to_string();
                return Ok(());
            }
            open.valid_to = Some(now_iso.to_string());
        }
        members.push(MockTrunkMember {
            service_id,
            service_name,
            api_type,
            version,
            valid_from: now_iso.to_string(),
            last_provided_at: now_iso.to_string(),
            valid_to: None,
        });
        Ok(())
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
        drop(branches);
        self.write_outbox(outbox);
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
        self.copy_periphery(PeripheryTrack::Trunk, branch_id, as_of, now_iso);
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
        drop(pins);
        self.copy_periphery(
            PeripheryTrack::Branch(source_branch_id),
            target_branch_id,
            as_of,
            now_iso,
        );
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
        let track = match branch_id {
            None => PeripheryTrack::Trunk,
            Some(bid) => PeripheryTrack::Branch(bid),
        };
        dates.extend(
            self.periphery
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .iter()
                .filter(|r| r.track == track)
                .flat_map(|r| std::iter::once(r.valid_from.clone()).chain(r.valid_to.clone())),
        );
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
        for member in self
            .trunk_members
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter_mut()
        {
            if member.valid_to.is_none() && member.last_provided_at.as_str() < cutoff_iso {
                member.valid_to = Some(now_iso.to_string());
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
        self.periphery
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .retain(|r| r.track != PeripheryTrack::Branch(branch_id));
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
        self.record_dependencies_bulk(vec![params], Vec::new())
            .await
    }

    async fn record_dependencies_bulk(
        &self,
        params: Vec<RecordDependencyParams<'_>>,
        outbox: Vec<NewWebhookEvent>,
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
        drop(dependencies);
        self.write_outbox(outbox);
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
            harvested_subscriptions: Vec::new(),
            peripheral_dependencies: Vec::new(),
            systems: BTreeMap::new(),
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
        self.periphery
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clear();
        self.system_contexts
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clear();
        self.webhooks
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clear();
        self.webhook_events
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clear();
        self.webhook_deliveries
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

    async fn search_endpoints(
        &self,
        search: &EndpointSearch,
    ) -> Result<Vec<EndpointSearchHit>, RepositoryError> {
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
        let wanted_method = search.method.as_ref().map(|m| m.to_uppercase());
        let mut hits = Vec::new();
        for v in &versions {
            if search.api_type.is_some_and(|t| t != v.api_type) {
                continue;
            }
            let superseded = versions.iter().any(|o| {
                o.service_id == v.service_id && o.api_type == v.api_type && o.version > v.version
            });
            if !search.all_versions && superseded {
                continue;
            }
            let producer = self.service_name(v.service_id).unwrap_or_default();
            for e in &v.endpoints {
                if wanted_method
                    .as_ref()
                    .is_some_and(|m| *m != e.method.to_uppercase())
                {
                    continue;
                }
                if !glob_contains(&e.path.to_lowercase(), &search.query)
                    && !glob_contains(&e.method.to_lowercase(), &search.query)
                {
                    continue;
                }
                let consumers: std::collections::BTreeSet<i64> = dependencies
                    .iter()
                    .filter(|d| {
                        d.spec_version_id == v.id
                            && d.api_type == e.api_type
                            && d.normalized_path == e.normalized_path
                            && d.method == e.method
                    })
                    .map(|d| d.client_id)
                    .collect();
                hits.push(EndpointSearchHit {
                    producer: producer.clone(),
                    api_type: v.api_type,
                    version: v.version,
                    stability: v.stability,
                    method: e.method.clone(),
                    path: e.path.clone(),
                    deprecated: e.deprecated,
                    consumer_count: consumers.len() as i64,
                });
            }
        }
        hits.sort_by(|a, b| {
            (
                &a.producer,
                a.api_type.as_str(),
                a.version,
                &a.path,
                &a.method,
            )
                .cmp(&(
                    &b.producer,
                    b.api_type.as_str(),
                    b.version,
                    &b.path,
                    &b.method,
                ))
        });
        Ok(hits)
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

    async fn delete_expired_credentials(&self, now_iso: &str) -> Result<u64, RepositoryError> {
        let mut sessions = self.sessions.lock().unwrap_or_else(PoisonError::into_inner);
        let before = sessions.len();
        sessions.retain(|s| s.expires_at.as_str() >= now_iso);
        Ok((before - sessions.len()) as u64)
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

    async fn create_api_token(&self, token: NewApiToken<'_>) -> Result<(), RepositoryError> {
        let mut tokens = self
            .api_tokens
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if tokens.iter().any(|t| {
            t.token_hash == token.token_hash || (t.user_id == token.user_id && t.name == token.name)
        }) {
            return Err(RepositoryError::Conflict);
        }
        tokens.push(ApiToken {
            id: token.id.to_string(),
            user_id: token.user_id,
            name: token.name.to_string(),
            token_hash: token.token_hash.to_string(),
            scope: token.scope,
            created_at: token.created_at.to_string(),
            expires_at: token.expires_at.to_string(),
            last_used_at: None,
        });
        Ok(())
    }

    async fn list_api_tokens(&self, user_id: i64) -> Result<Vec<ApiToken>, RepositoryError> {
        Ok(self
            .api_tokens
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .filter(|t| t.user_id == user_id)
            .cloned()
            .collect())
    }

    async fn delete_api_token(
        &self,
        token_id: &str,
        user_id: i64,
    ) -> Result<bool, RepositoryError> {
        let mut tokens = self
            .api_tokens
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let before = tokens.len();
        tokens.retain(|t| !(t.id == token_id && t.user_id == user_id));
        Ok(tokens.len() < before)
    }

    async fn validate_api_token(
        &self,
        token_hash: &str,
    ) -> Result<Option<ApiTokenIdentity>, RepositoryError> {
        let now = chrono::Utc::now().to_rfc3339();
        let token = self
            .api_tokens
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .find(|t| t.token_hash == token_hash && t.expires_at > now)
            .cloned();
        let Some(token) = token else {
            return Ok(None);
        };
        let user = self
            .users
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .find(|u| u.id == token.user_id)
            .cloned();
        Ok(user.map(|user| ApiTokenIdentity {
            token_id: token.id,
            scope: token.scope,
            last_used_at: token.last_used_at,
            user,
        }))
    }

    async fn touch_api_token(&self, token_id: &str, used_at: &str) -> Result<(), RepositoryError> {
        let mut tokens = self
            .api_tokens
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if let Some(token) = tokens.iter_mut().find(|t| t.id == token_id) {
            token.last_used_at = Some(used_at.to_string());
        }
        Ok(())
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

    async fn remove_service_tag(&self, service_id: i64, tag: &str) -> Result<(), RepositoryError> {
        let mut st = self
            .service_tags
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if let Some(tags) = st.get_mut(&service_id) {
            tags.retain(|t| t != tag);
        }
        Ok(())
    }

    async fn close_trunk_pins_for_service_api(
        &self,
        service_id: i64,
        api_type: ApiType,
        now_iso: &str,
    ) -> Result<u64, RepositoryError> {
        let mut pins = self
            .trunk_pins
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let mut n = 0u64;
        for p in pins.iter_mut() {
            if p.service_id == service_id && p.api_type == api_type && p.valid_to.is_none() {
                p.valid_to = Some(now_iso.to_string());
                n += 1;
            }
        }
        Ok(n)
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

    async fn reconcile_harvested_subscriptions(
        &self,
        client_id: i64,
        client_name: &str,
        trunk: bool,
        subs: &[HarvestedSubInput],
        now_iso: &str,
    ) -> Result<(), RepositoryError> {
        let mut rows = self
            .harvested_subscriptions
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        // Close open rows for this client that are gone or changed.
        for row in rows.iter_mut() {
            if row.client_id != client_id || row.valid_to.is_some() {
                continue;
            }
            let keep = subs.iter().any(|s| {
                s.channel == row.channel
                    && s.message_name == row.message_name
                    && s.owner_service_id == row.owner_service_id
                    && trunk == row.trunk
            });
            if !keep {
                row.valid_to = Some(now_iso.to_string());
            }
        }
        // Open a fresh row for each desired sub without an unchanged open row.
        for s in subs {
            let unchanged = rows.iter().any(|r| {
                r.valid_to.is_none()
                    && r.client_id == client_id
                    && r.channel == s.channel
                    && r.message_name == s.message_name
                    && r.owner_service_id == s.owner_service_id
                    && r.trunk == trunk
            });
            if unchanged {
                continue;
            }
            rows.push(MockHarvestedSub {
                client_id,
                client_name: client_name.to_string(),
                channel: s.channel.clone(),
                message_name: s.message_name.clone(),
                owner_service_id: s.owner_service_id,
                owner_name: s.owner_name.clone(),
                trunk,
                valid_from: now_iso.to_string(),
                valid_to: None,
            });
        }
        Ok(())
    }

    async fn list_current_harvested_subscriptions(
        &self,
    ) -> Result<Vec<HarvestedSubscription>, RepositoryError> {
        let rows = self
            .harvested_subscriptions
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let mut result: Vec<HarvestedSubscription> = rows
            .iter()
            .filter(|r| r.valid_to.is_none())
            .map(|r| HarvestedSubscription {
                client: r.client_name.clone(),
                channel: r.channel.clone(),
                message_name: r.message_name.clone(),
                owner: r.owner_name.clone(),
                trunk: r.trunk,
            })
            .collect();
        result.sort_by(|a, b| {
            a.client
                .cmp(&b.client)
                .then_with(|| a.channel.cmp(&b.channel))
                .then_with(|| a.message_name.cmp(&b.message_name))
        });
        Ok(result)
    }

    async fn reconcile_periphery(
        &self,
        system: &str,
        trunk: bool,
        resources: &[String],
        now_iso: &str,
        outbox: Vec<NewWebhookEvent>,
    ) -> Result<bool, RepositoryError> {
        let mut tracks = vec![PeripheryTrack::Dev];
        if trunk {
            tracks.push(PeripheryTrack::Trunk);
        }
        let mut changed = false;
        let mut rows = self
            .periphery
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        for track in tracks {
            for row in rows.iter_mut() {
                if row.track == track
                    && row.system == system
                    && row.valid_to.is_none()
                    && !resources.contains(&row.resource)
                {
                    row.valid_to = Some(now_iso.to_string());
                    changed = true;
                }
            }
            for resource in resources {
                let open = rows.iter().any(|r| {
                    r.track == track
                        && r.system == system
                        && &r.resource == resource
                        && r.valid_to.is_none()
                });
                if !open {
                    rows.push(MockPeripheryRow {
                        track,
                        system: system.to_string(),
                        resource: resource.clone(),
                        valid_from: now_iso.to_string(),
                        valid_to: None,
                    });
                    changed = true;
                }
            }
        }
        drop(rows);
        self.write_outbox(outbox);
        Ok(changed)
    }

    async fn upsert_system_context(
        &self,
        system: &str,
        context: &SystemContext,
        _now_iso: &str,
    ) -> Result<bool, RepositoryError> {
        let mut rows = self
            .system_contexts
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if rows.get(system) == Some(context) {
            return Ok(false);
        }
        rows.insert(system.to_string(), context.clone());
        Ok(true)
    }

    async fn get_system_context(
        &self,
        system: &str,
    ) -> Result<Option<SystemContext>, RepositoryError> {
        Ok(self
            .system_contexts
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(system)
            .cloned())
    }

    async fn list_system_contexts(
        &self,
    ) -> Result<BTreeMap<String, SystemContext>, RepositoryError> {
        Ok(self
            .system_contexts
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone())
    }

    async fn delete_system_context(&self, system: &str) -> Result<(), RepositoryError> {
        self.system_contexts
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(system);
        Ok(())
    }

    async fn list_periphery(
        &self,
        track: PeripheryTrack,
        at: Option<&str>,
    ) -> Result<Vec<PeripheralDependency>, RepositoryError> {
        let rows = self
            .periphery
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let mut result: Vec<PeripheralDependency> = rows
            .iter()
            .filter(|r| r.track == track)
            .filter(|r| match at {
                Some(at) => {
                    r.valid_from.as_str() <= at && r.valid_to.as_deref().is_none_or(|to| to > at)
                }
                None => r.valid_to.is_none(),
            })
            .map(|r| PeripheralDependency {
                system: r.system.clone(),
                resource: r.resource.clone(),
            })
            .collect();
        result.sort();
        Ok(result)
    }

    async fn list_member_versions(
        &self,
        track: MemberTrack,
        at: Option<&str>,
    ) -> Result<Vec<MemberVersion>, RepositoryError> {
        let current = |from: &str, to: Option<&str>| match at {
            Some(at) => from <= at && to.is_none_or(|to| to > at),
            None => to.is_none(),
        };
        let mut result: Vec<MemberVersion> = match track {
            MemberTrack::Trunk => self
                .trunk_members
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .iter()
                .filter(|m| current(&m.valid_from, m.valid_to.as_deref()))
                .map(|m| MemberVersion {
                    service: m.service_name.clone(),
                    api_type: m.api_type,
                    version: m.version,
                    valid_from: m.valid_from.clone(),
                    last_provided_at: Some(m.last_provided_at.clone()),
                })
                .collect(),
            MemberTrack::Branch(branch_id) => self
                .branch_member_versions
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .iter()
                .filter(|r| r.0 == branch_id && current(&r.4, r.5.as_deref()))
                .map(|r| MemberVersion {
                    service: self.service_name(r.1).unwrap_or_default(),
                    api_type: r.2,
                    version: r.3,
                    valid_from: r.4.clone(),
                    last_provided_at: None,
                })
                .collect(),
        };
        result.sort_by(|a, b| {
            (&a.service, a.api_type.as_str()).cmp(&(&b.service, b.api_type.as_str()))
        });
        Ok(result)
    }

    // --- Webhooks (#47) ---

    async fn insert_webhook_events(
        &self,
        events: &[NewWebhookEvent],
    ) -> Result<(), RepositoryError> {
        self.write_outbox(events.to_vec());
        Ok(())
    }

    async fn list_current_consumers_of(
        &self,
        service_id: i64,
    ) -> Result<Vec<String>, RepositoryError> {
        let version_ids: Vec<i64> = self
            .spec_versions
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .filter(|v| v.service_id == service_id)
            .map(|v| v.id)
            .collect();
        let mut client_ids: Vec<i64> = self
            .dependencies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .filter(|d| version_ids.contains(&d.spec_version_id))
            .map(|d| d.client_id)
            .collect();
        client_ids.extend(
            self.trunk_pins
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .iter()
                .filter(|p| p.service_id == service_id && p.valid_to.is_none())
                .map(|p| p.client_id),
        );
        let mut names: Vec<String> = client_ids
            .into_iter()
            .filter_map(|id| self.client_name(id))
            .collect();
        names.sort();
        names.dedup();
        Ok(names)
    }

    async fn create_webhook(&self, hook: NewWebhook<'_>) -> Result<i64, RepositoryError> {
        let id = self.next_id();
        self.webhooks
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(Webhook {
                id,
                scope: hook.scope.clone(),
                url: hook.url.to_string(),
                format: hook.format,
                event_types: hook.event_types.to_vec(),
                producer_glob: hook.producer_glob.map(str::to_string),
                secret_hint: hook.secret_hint.to_string(),
                paused: false,
                allow_list_exempt: hook.allow_list_exempt,
                created_by: hook.created_by.to_string(),
                created_at: hook.created_at.to_string(),
                secret: hook.secret.to_string(),
            });
        Ok(id)
    }

    async fn list_webhooks(
        &self,
        scope: Option<&WebhookScope>,
    ) -> Result<Vec<Webhook>, RepositoryError> {
        Ok(self
            .webhooks
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .filter(|h| scope.is_none_or(|s| &h.scope == s))
            .cloned()
            .collect())
    }

    async fn get_webhook(&self, id: i64) -> Result<Option<Webhook>, RepositoryError> {
        Ok(self
            .webhooks
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .find(|h| h.id == id)
            .cloned())
    }

    async fn update_webhook(&self, id: i64, patch: &WebhookPatch) -> Result<bool, RepositoryError> {
        let mut hooks = self.webhooks.lock().unwrap_or_else(PoisonError::into_inner);
        let Some(hook) = hooks.iter_mut().find(|h| h.id == id) else {
            return Ok(false);
        };
        if let Some(paused) = patch.paused {
            hook.paused = paused;
        }
        if let Some(types) = &patch.event_types {
            hook.event_types = types.clone();
        }
        if let Some(glob) = &patch.producer_glob {
            hook.producer_glob = glob.clone();
        }
        Ok(true)
    }

    async fn rotate_webhook_secret(
        &self,
        id: i64,
        secret: &str,
        secret_hint: &str,
    ) -> Result<bool, RepositoryError> {
        let mut hooks = self.webhooks.lock().unwrap_or_else(PoisonError::into_inner);
        let Some(hook) = hooks.iter_mut().find(|h| h.id == id) else {
            return Ok(false);
        };
        hook.secret = secret.to_string();
        hook.secret_hint = secret_hint.to_string();
        Ok(true)
    }

    async fn delete_webhook(&self, id: i64) -> Result<bool, RepositoryError> {
        let mut hooks = self.webhooks.lock().unwrap_or_else(PoisonError::into_inner);
        let before = hooks.len();
        hooks.retain(|h| h.id != id);
        self.webhook_deliveries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .retain(|d| d.hook_id != id);
        Ok(hooks.len() < before)
    }

    async fn list_webhook_deliveries(
        &self,
        hook_id: i64,
        limit: u32,
    ) -> Result<Vec<WebhookDelivery>, RepositoryError> {
        let mut rows: Vec<WebhookDelivery> = self
            .webhook_deliveries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .filter(|d| d.hook_id == hook_id)
            .cloned()
            .collect();
        rows.sort_by_key(|d| std::cmp::Reverse(d.id));
        rows.truncate(limit as usize);
        Ok(rows)
    }

    async fn get_webhook_delivery(
        &self,
        delivery_id: i64,
    ) -> Result<Option<WebhookDelivery>, RepositoryError> {
        Ok(self
            .webhook_deliveries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .find(|d| d.id == delivery_id)
            .cloned())
    }

    async fn reset_webhook_delivery(
        &self,
        delivery_id: i64,
        now_iso: &str,
    ) -> Result<bool, RepositoryError> {
        let mut rows = self
            .webhook_deliveries
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let Some(row) = rows.iter_mut().find(|d| d.id == delivery_id) else {
            return Ok(false);
        };
        row.status = WebhookDeliveryStatus::Pending;
        row.next_attempt_at = now_iso.to_string();
        row.updated_at = now_iso.to_string();
        Ok(true)
    }

    async fn claim_due_webhook_deliveries(
        &self,
        now_iso: &str,
        lease_until_iso: &str,
        limit: u32,
    ) -> Result<Vec<DueWebhookDelivery>, RepositoryError> {
        let hooks = self
            .webhooks
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        let events = self
            .webhook_events
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        let mut rows = self
            .webhook_deliveries
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let mut due: Vec<&mut WebhookDelivery> = rows
            .iter_mut()
            .filter(|d| {
                matches!(
                    d.status,
                    WebhookDeliveryStatus::Pending | WebhookDeliveryStatus::Failed
                ) && d.next_attempt_at.as_str() <= now_iso
                    && hooks.iter().any(|h| h.id == d.hook_id && !h.paused)
            })
            .collect();
        due.sort_by(|a, b| (&a.next_attempt_at, a.id).cmp(&(&b.next_attempt_at, b.id)));
        due.truncate(limit as usize);
        let mut claimed = Vec::new();
        for delivery in due {
            delivery.next_attempt_at = lease_until_iso.to_string();
            let Some(hook) = hooks.iter().find(|h| h.id == delivery.hook_id) else {
                continue;
            };
            let Some(event) = events.iter().find(|e| e.id == delivery.event_id) else {
                continue;
            };
            claimed.push(DueWebhookDelivery {
                delivery: delivery.clone(),
                hook: hook.clone(),
                producer: event.producer.clone(),
                data: event.data.clone(),
            });
        }
        Ok(claimed)
    }

    async fn record_webhook_attempt(
        &self,
        delivery_id: i64,
        now_iso: &str,
        outcome: &WebhookAttemptOutcome,
    ) -> Result<(), RepositoryError> {
        let mut rows = self
            .webhook_deliveries
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let Some(row) = rows.iter_mut().find(|d| d.id == delivery_id) else {
            return Ok(());
        };
        row.updated_at = now_iso.to_string();
        match outcome {
            WebhookAttemptOutcome::Delivered { code } => {
                row.status = WebhookDeliveryStatus::Delivered;
                row.attempts += 1;
                row.last_response_code = Some(*code);
                row.last_error = None;
                row.delivered_at = Some(now_iso.to_string());
            }
            WebhookAttemptOutcome::Failed {
                code,
                error,
                next_attempt_at,
            } => {
                row.status = WebhookDeliveryStatus::Failed;
                row.attempts += 1;
                row.last_response_code = *code;
                row.last_error = Some(error.clone());
                row.next_attempt_at = next_attempt_at.clone();
            }
            WebhookAttemptOutcome::Dead { code, error } => {
                row.status = WebhookDeliveryStatus::Dead;
                row.attempts += 1;
                row.last_response_code = *code;
                row.last_error = Some(error.clone());
            }
            WebhookAttemptOutcome::Expired { error } => {
                row.status = WebhookDeliveryStatus::Dead;
                row.last_response_code = None;
                row.last_error = Some(error.clone());
            }
        }
        Ok(())
    }

    async fn count_pending_webhook_deliveries(&self) -> Result<u64, RepositoryError> {
        Ok(self
            .webhook_deliveries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .filter(|d| {
                matches!(
                    d.status,
                    WebhookDeliveryStatus::Pending | WebhookDeliveryStatus::Failed
                )
            })
            .count() as u64)
    }

    async fn delete_old_webhook_data(&self, cutoff_iso: &str) -> Result<u64, RepositoryError> {
        let mut rows = self
            .webhook_deliveries
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let before = rows.len();
        rows.retain(|d| {
            !(matches!(
                d.status,
                WebhookDeliveryStatus::Delivered | WebhookDeliveryStatus::Dead
            ) && d.updated_at.as_str() < cutoff_iso)
        });
        let removed = (before - rows.len()) as u64;
        let referenced: Vec<String> = rows.iter().map(|d| d.event_id.clone()).collect();
        drop(rows);
        self.webhook_events
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .retain(|e| e.occurred_at.as_str() >= cutoff_iso || referenced.contains(&e.id));
        Ok(removed)
    }
}

/// Case-insensitive "contains" with `*` matching anything — the in-memory
/// twin of the SQL `LIKE` pattern the real repositories build.
fn glob_contains(haystack: &str, query: &str) -> bool {
    let query = query.trim().to_lowercase();
    let mut rest = haystack;
    for part in query.split('*') {
        let Some(at) = rest.find(part) else {
            return false;
        };
        rest = &rest[at + part.len()..];
    }
    true
}
