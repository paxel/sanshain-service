use crate::domain::models::*;
use crate::domain::ports::{
    EndpointMap, NewAuditLog, RecordDependencyParams, RepositoryError, SpecRepository,
    UpdateEndpointParams,
};
use std::collections::HashMap;
use std::sync::{Mutex, PoisonError};

pub struct MockRepo {
    pub services: Mutex<HashMap<String, i64>>,
    pub branches: Mutex<HashMap<(i64, String), i64>>,
    pub endpoints: Mutex<HashMap<i64, Vec<EndpointRecord>>>,
    pub deleted_endpoints: Mutex<Vec<(i64, ApiType, String, String)>>,
    pub clients: Mutex<HashMap<String, i64>>,
    pub protected_branches: Mutex<Vec<String>>,
    pub next_id: Mutex<i64>,
    pub fallback_branches: Mutex<HashMap<String, String>>,
    pub users: Mutex<Vec<User>>,
    pub sessions: Mutex<Vec<Session>>,
    pub settings: Mutex<HashMap<String, String>>,
    pub api_tokens: Mutex<Vec<ApiToken>>,
    pub endpoint_versions: Mutex<Vec<EndpointVersion>>,
    pub service_tags: Mutex<HashMap<i64, Vec<String>>>,
    pub spec_versions: Mutex<HashMap<(i64, i64), (SemVer, String)>>,
    pub audit_logs: Mutex<Vec<AuditLogEntry>>,
    pub user_favorites: Mutex<Vec<(i64, String, String)>>,
    pub branch_timestamps: Mutex<HashMap<String, String>>,
    pub channel_message_contracts: Mutex<Vec<ChannelMessageContract>>,
    pub source_protected_branches: Mutex<HashMap<i64, String>>,
    pub user_roles: Mutex<Vec<(i64, String)>>,
    pub groups: Mutex<Vec<Group>>,
    pub group_members: Mutex<Vec<(i64, i64)>>,
    pub group_roles: Mutex<Vec<(i64, String)>>,
    pub user_maintainers: Mutex<Vec<(i64, i64)>>,
    pub group_maintainers: Mutex<Vec<(i64, i64)>>,
    pub onboarding: Mutex<Vec<i64>>,
    pub pending_specs: Mutex<Vec<PendingSpec>>,
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
            branches: Mutex::new(HashMap::new()),
            endpoints: Mutex::new(HashMap::new()),
            deleted_endpoints: Mutex::new(Vec::new()),
            clients: Mutex::new(HashMap::new()),
            protected_branches: Mutex::new(vec!["main".to_string(), "master".to_string()]),
            next_id: Mutex::new(1),
            fallback_branches: Mutex::new(HashMap::new()),
            users: Mutex::new(Vec::new()),
            sessions: Mutex::new(Vec::new()),
            settings: Mutex::new(settings),
            api_tokens: Mutex::new(Vec::new()),
            endpoint_versions: Mutex::new(Vec::new()),
            service_tags: Mutex::new(HashMap::new()),
            spec_versions: Mutex::new(HashMap::new()),
            audit_logs: Mutex::new(Vec::new()),
            user_favorites: Mutex::new(Vec::new()),
            branch_timestamps: Mutex::new(HashMap::new()),
            channel_message_contracts: Mutex::new(Vec::new()),
            source_protected_branches: Mutex::new(HashMap::new()),
            user_roles: Mutex::new(Vec::new()),
            groups: Mutex::new(Vec::new()),
            group_members: Mutex::new(Vec::new()),
            group_roles: Mutex::new(Vec::new()),
            user_maintainers: Mutex::new(Vec::new()),
            group_maintainers: Mutex::new(Vec::new()),
            onboarding: Mutex::new(Vec::new()),
            pending_specs: Mutex::new(Vec::new()),
        }
    }

    pub fn next_id(&self) -> i64 {
        let mut id = self.next_id.lock().unwrap_or_else(PoisonError::into_inner);
        let current = *id;
        *id += 1;
        current
    }
}

impl SpecRepository for MockRepo {
    async fn ping(&self) -> Result<(), RepositoryError> {
        Ok(())
    }

    async fn get_spec_version(
        &self,
        service_id: i64,
        branch_id: i64,
    ) -> Result<Option<(SemVer, String)>, RepositoryError> {
        let versions = self
            .spec_versions
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        Ok(versions.get(&(service_id, branch_id)).cloned())
    }

    async fn increment_spec_version(
        &self,
        service_id: i64,
        branch_id: i64,
        content_hash: &str,
        impact: Impact,
    ) -> Result<SemVer, RepositoryError> {
        let mut versions = self
            .spec_versions
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let (version, _) = versions
            .entry((service_id, branch_id))
            .or_insert((SemVer::default(), String::new()));
        *version = if version.major == 0 {
            SemVer::initial()
        } else {
            version.increment(impact)
        };
        let current_version = *version;
        versions.insert(
            (service_id, branch_id),
            (current_version, content_hash.to_string()),
        );
        Ok(current_version)
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
        let services = self.services.lock().unwrap_or_else(PoisonError::into_inner);
        Ok(services
            .iter()
            .find(|&(_, &id)| id == service_id)
            .map(|(name, _)| name.clone()))
    }
    async fn ensure_branch(
        &self,
        service_id: i64,
        branch_name: &str,
    ) -> Result<i64, RepositoryError> {
        let mut branches = self.branches.lock().unwrap_or_else(PoisonError::into_inner);
        let key = (service_id, branch_name.to_string());
        if let Some(&id) = branches.get(&key) {
            return Ok(id);
        }
        let id = self.next_id();
        branches.insert(key, id);
        Ok(id)
    }
    async fn find_branch(
        &self,
        service_id: i64,
        branch_name: &str,
    ) -> Result<Option<i64>, RepositoryError> {
        let branches = self.branches.lock().unwrap_or_else(PoisonError::into_inner);
        let key = (service_id, branch_name.to_string());
        Ok(branches.get(&key).copied())
    }

    async fn get_endpoints_for_branch(
        &self,
        branch_id: i64,
    ) -> Result<Vec<EndpointRecord>, RepositoryError> {
        let endpoints = self
            .endpoints
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let deleted = self
            .deleted_endpoints
            .lock()
            .unwrap_or_else(PoisonError::into_inner);

        Ok(endpoints
            .get(&branch_id)
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .filter(|ep| {
                !deleted.contains(&(branch_id, ep.api_type, ep.path.clone(), ep.method.clone()))
            })
            .collect())
    }

    async fn insert_endpoint(
        &self,
        branch_id: i64,
        endpoint: &EndpointRecord,
    ) -> Result<(), RepositoryError> {
        let mut endpoints = self
            .endpoints
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let mut record = endpoint.clone();
        record.id = Some(self.next_id());
        endpoints.entry(branch_id).or_default().push(record);
        Ok(())
    }

    async fn reset_branch_history(
        &self,
        _service_name: &str,
        _branch_name: &str,
    ) -> Result<bool, RepositoryError> {
        Ok(true)
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

    async fn record_dependency(
        &self,
        _params: RecordDependencyParams<'_>,
    ) -> Result<(), RepositoryError> {
        Ok(())
    }

    async fn record_dependencies_bulk(
        &self,
        _params: Vec<RecordDependencyParams<'_>>,
    ) -> Result<(), RepositoryError> {
        Ok(())
    }

    async fn find_endpoint(
        &self,
        _service_id: i64,
        _branch_name: &str,
        _api_type: ApiType,
        _path: &str,
        _method: &str,
    ) -> Result<Option<(i64, String, bool, bool)>, RepositoryError> {
        Ok(None)
    }

    async fn find_endpoints_bulk(
        &self,
        _service_id: i64,
        _branch_name: &str,
        _api_type: ApiType,
        _endpoints: &[(String, String)],
    ) -> Result<EndpointMap, RepositoryError> {
        Ok(HashMap::new())
    }

    async fn get_report(&self, branch: &str) -> Result<DependencyReport, RepositoryError> {
        Ok(DependencyReport {
            branch: branch.to_string(),
            unused_endpoints: Vec::new(),
            missing_endpoints: Vec::new(),
            dependency_graph: Vec::new(),
            service_tags: HashMap::new(),
        })
    }

    async fn is_branch_protected(&self, branch_name: &str) -> Result<bool, RepositoryError> {
        // Uses the same shared matcher as the SQL repositories, so wildcard
        // patterns behave identically in mock-based tests.
        let pb = self
            .protected_branches
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        Ok(crate::domain::branch_pattern::branch_matches_any(
            branch_name,
            &pb,
        ))
    }

    async fn add_protected_branch(&self, pattern: &str) -> Result<(), RepositoryError> {
        let mut pb = self
            .protected_branches
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if !pb.contains(&pattern.to_string()) {
            pb.push(pattern.to_string());
        }
        Ok(())
    }

    async fn remove_protected_branch(&self, pattern: &str) -> Result<bool, RepositoryError> {
        let mut pb = self
            .protected_branches
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if let Some(pos) = pb.iter().position(|p| p == pattern) {
            pb.remove(pos);
            Ok(true)
        } else {
            Ok(false)
        }
    }

    async fn list_protected_branches(&self) -> Result<Vec<String>, RepositoryError> {
        Ok(self
            .protected_branches
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone())
    }

    async fn update_endpoint(
        &self,
        _params: UpdateEndpointParams<'_>,
    ) -> Result<(), RepositoryError> {
        Ok(())
    }

    async fn soft_delete_endpoint(
        &self,
        branch_id: i64,
        api_type: ApiType,
        path: &str,
        method: &str,
    ) -> Result<(), RepositoryError> {
        let mut deleted = self
            .deleted_endpoints
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        deleted.push((branch_id, api_type, path.to_string(), method.to_string()));
        Ok(())
    }

    async fn hard_delete_endpoint(
        &self,
        branch_id: i64,
        api_type: ApiType,
        path: &str,
        method: &str,
    ) -> Result<(), RepositoryError> {
        let mut endpoints = self
            .endpoints
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if let Some(list) = endpoints.get_mut(&branch_id) {
            list.retain(|ep| !(ep.api_type == api_type && ep.path == path && ep.method == method));
        }
        Ok(())
    }

    async fn is_endpoint_deleted(
        &self,
        branch_id: i64,
        api_type: ApiType,
        path: &str,
        method: &str,
    ) -> Result<bool, RepositoryError> {
        let deleted = self
            .deleted_endpoints
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        Ok(deleted.contains(&(branch_id, api_type, path.to_string(), method.to_string())))
    }

    async fn delete_all_services(&self) -> Result<u64, RepositoryError> {
        let count = self
            .services
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len() as u64;
        self.services
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clear();
        self.branches
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clear();
        self.endpoints
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clear();
        Ok(count)
    }

    async fn delete_all_clients(&self) -> Result<u64, RepositoryError> {
        let count = self
            .clients
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len() as u64;
        self.clients
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clear();
        Ok(count)
    }

    async fn delete_all_non_admin_users(&self) -> Result<u64, RepositoryError> {
        let admins: Vec<i64> = self
            .user_roles
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .filter(|(_, role)| role == "admin")
            .map(|(user_id, _)| *user_id)
            .collect();
        let mut users = self.users.lock().unwrap_or_else(PoisonError::into_inner);
        let initial_count = users.len() as u64;
        users.retain(|u| admins.contains(&u.id));
        Ok(initial_count - users.len() as u64)
    }

    async fn nuke_database(&self, _keep_user_id: Option<i64>) -> Result<(), RepositoryError> {
        self.delete_all_services().await?;
        self.delete_all_clients().await?;
        Ok(())
    }

    async fn delete_producer(&self, name: &str) -> Result<bool, RepositoryError> {
        let mut services = self.services.lock().unwrap_or_else(PoisonError::into_inner);
        Ok(services.remove(name).is_some())
    }

    async fn delete_branch(
        &self,
        _service_name: &str,
        _branch_name: &str,
    ) -> Result<bool, RepositoryError> {
        Ok(true)
    }

    async fn delete_consumer(&self, name: &str) -> Result<bool, RepositoryError> {
        let mut clients = self.clients.lock().unwrap_or_else(PoisonError::into_inner);
        Ok(clients.remove(name).is_some())
    }

    async fn list_producers(&self) -> Result<Vec<String>, RepositoryError> {
        Ok(self
            .services
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .keys()
            .cloned()
            .collect())
    }

    async fn list_producers_detailed(&self) -> Result<Vec<ProducerSummary>, RepositoryError> {
        let services = self.services.lock().unwrap_or_else(PoisonError::into_inner);
        let branches = self.branches.lock().unwrap_or_else(PoisonError::into_inner);
        let fallback_branches = self
            .fallback_branches
            .lock()
            .unwrap_or_else(PoisonError::into_inner);

        let mut result = Vec::new();
        for (name, id) in services.iter() {
            let mut svc_branches = Vec::new();
            for ((bid, bname), _) in branches.iter() {
                if *bid == *id {
                    svc_branches.push(bname.clone());
                }
            }
            svc_branches.sort();
            result.push(ProducerSummary {
                name: name.clone(),
                fallback_branch: fallback_branches.get(name).cloned(),
                branches: svc_branches,
                branches_last_published: std::collections::HashMap::new(),
                branches_expire_at: std::collections::HashMap::new(),
                branches_endpoint_count: std::collections::HashMap::new(),
                is_favorite: false,
                icon: None,
                domain: None,
            });
        }
        result.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(result)
    }

    async fn set_fallback_branch(
        &self,
        service_name: &str,
        branch: Option<&str>,
    ) -> Result<(), RepositoryError> {
        let mut fb = self
            .fallback_branches
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if let Some(b) = branch {
            fb.insert(service_name.to_string(), b.to_string());
        } else {
            fb.remove(service_name);
        }
        Ok(())
    }

    async fn update_producer_metadata(
        &self,
        _service_name: &str,
        _icon: Option<&str>,
        _domain: Option<&str>,
    ) -> Result<(), RepositoryError> {
        Ok(())
    }

    async fn get_fallback_branch(
        &self,
        service_name: &str,
    ) -> Result<Option<String>, RepositoryError> {
        Ok(self
            .fallback_branches
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(service_name)
            .cloned())
    }

    async fn set_source_protected_branch_if_unset(
        &self,
        branch_id: i64,
        value: &str,
    ) -> Result<bool, RepositoryError> {
        let mut spb = self
            .source_protected_branches
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if spb.contains_key(&branch_id) {
            return Ok(false);
        }
        spb.insert(branch_id, value.to_string());
        Ok(true)
    }

    async fn get_source_protected_branch(
        &self,
        branch_id: i64,
    ) -> Result<Option<String>, RepositoryError> {
        Ok(self
            .source_protected_branches
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&branch_id)
            .cloned())
    }

    async fn admin_set_source_protected_branch(
        &self,
        branch_id: i64,
        value: Option<&str>,
    ) -> Result<(), RepositoryError> {
        let mut spb = self
            .source_protected_branches
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        match value {
            Some(v) => {
                spb.insert(branch_id, v.to_string());
            }
            None => {
                spb.remove(&branch_id);
            }
        }
        Ok(())
    }

    async fn list_branches(&self, _service_name: &str) -> Result<Vec<String>, RepositoryError> {
        Ok(Vec::new())
    }

    async fn list_all_branches(&self) -> Result<Vec<String>, RepositoryError> {
        Ok(Vec::new())
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

    async fn list_consumer_branches(
        &self,
        _client_name: &str,
    ) -> Result<Vec<String>, RepositoryError> {
        Ok(Vec::new())
    }

    async fn list_consumer_endpoints(
        &self,
        _client_name: &str,
        _branch: &str,
    ) -> Result<Vec<ConsumerEndpointInfo>, RepositoryError> {
        // This is a simplified mock implementation
        // Real implementation joins dependencies, services, and endpoints
        Ok(Vec::new())
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

    async fn delete_stale_branches(&self, _cutoff: &str) -> Result<u64, RepositoryError> {
        Ok(0)
    }

    async fn delete_stale_dependencies(&self, _cutoff: &str) -> Result<u64, RepositoryError> {
        Ok(0)
    }

    async fn get_endpoint_id(
        &self,
        _branch_id: i64,
        _api_type: ApiType,
        _path: &str,
        _method: &str,
    ) -> Result<Option<i64>, RepositoryError> {
        Ok(None)
    }

    async fn insert_endpoint_version(
        &self,
        _endpoint_id: i64,
        _version: i32,
        _yaml: &str,
        _diff: Option<&str>,
        _created: &str,
    ) -> Result<(), RepositoryError> {
        Ok(())
    }

    async fn get_latest_endpoint_version(&self, _endpoint_id: i64) -> Result<i32, RepositoryError> {
        Ok(0)
    }

    async fn get_endpoint_versions(
        &self,
        _endpoint_id: i64,
    ) -> Result<Vec<EndpointVersion>, RepositoryError> {
        Ok(Vec::new())
    }

    async fn get_global_endpoint_versions(
        &self,
        _limit: u32,
    ) -> Result<Vec<EndpointVersion>, RepositoryError> {
        Ok(Vec::new())
    }

    async fn apply_spec_changes(
        &self,
        branch_id: i64,
        changes: Vec<SpecChange>,
        is_protected: bool,
        _username: Option<&str>,
        _source_branch: Option<&str>,
    ) -> Result<(), RepositoryError> {
        let mut endpoints = self
            .endpoints
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let mut deleted = self
            .deleted_endpoints
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let list = endpoints.entry(branch_id).or_default();

        for change in changes {
            match change {
                SpecChange::Insert {
                    api_type,
                    path,
                    normalized_path,
                    method,
                    yaml_content,
                    deprecated,
                    external,
                } => {
                    list.push(EndpointRecord {
                        id: Some(self.next_id()),
                        api_type,
                        path,
                        normalized_path,
                        method,
                        yaml_content,
                        deprecated,
                        external,
                    });
                }
                SpecChange::Update {
                    api_type,
                    path,
                    normalized_path,
                    method,
                    yaml_content,
                    deprecated,
                    external,
                } => {
                    if let Some(ep) = list.iter_mut().find(|e| {
                        e.api_type == api_type
                            && e.normalized_path == normalized_path
                            && e.method == method
                    }) {
                        ep.path = path;
                        ep.yaml_content = yaml_content;
                        ep.deprecated = deprecated;
                        ep.external = external;
                    }
                }
                SpecChange::Delete {
                    api_type,
                    path,
                    method,
                    soft_delete,
                } => {
                    if soft_delete || is_protected {
                        deleted.push((branch_id, api_type, path, method));
                    } else {
                        list.retain(|e| {
                            !(e.api_type == api_type && e.path == path && e.method == method)
                        });
                    }
                }
            }
        }
        Ok(())
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
        let mut next = self.next_id.lock().unwrap_or_else(PoisonError::into_inner);
        let id = *next;
        *next += 1;
        let group = Group {
            id,
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

    // --- Pending Specs ---

    async fn upsert_pending_spec(
        &self,
        service_id: i64,
        branch: &str,
        api_type: ApiType,
        content: &str,
        reason: &str,
        submitted_by: &str,
    ) -> Result<i64, RepositoryError> {
        let producer = self
            .services
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .find(|(_, sid)| **sid == service_id)
            .map(|(name, _)| name.clone())
            .unwrap_or_default();

        let mut pending = self
            .pending_specs
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if let Some(existing) = pending
            .iter_mut()
            .find(|p| p.producer == producer && p.branch == branch && p.api_type == api_type)
        {
            existing.content = content.to_string();
            existing.reason = reason.to_string();
            existing.submitted_by = submitted_by.to_string();
            return Ok(existing.id);
        }
        let mut next = self.next_id.lock().unwrap_or_else(PoisonError::into_inner);
        let id = *next;
        *next += 1;
        pending.push(PendingSpec {
            id,
            producer,
            branch: branch.to_string(),
            api_type,
            content: content.to_string(),
            reason: reason.to_string(),
            submitted_by: submitted_by.to_string(),
            created_at: "2026-07-31T00:00:00Z".to_string(),
        });
        Ok(id)
    }

    async fn get_pending_spec(&self, id: i64) -> Result<Option<PendingSpec>, RepositoryError> {
        Ok(self
            .pending_specs
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .find(|p| p.id == id)
            .cloned())
    }

    async fn list_pending_specs(&self) -> Result<Vec<PendingSpec>, RepositoryError> {
        Ok(self
            .pending_specs
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone())
    }

    async fn delete_pending_spec(&self, id: i64) -> Result<bool, RepositoryError> {
        let mut pending = self
            .pending_specs
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let before = pending.len();
        pending.retain(|p| p.id != id);
        Ok(pending.len() != before)
    }

    async fn clear_pending_spec(
        &self,
        service_id: i64,
        branch: &str,
        api_type: ApiType,
    ) -> Result<bool, RepositoryError> {
        let producer = self
            .services
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .find(|(_, sid)| **sid == service_id)
            .map(|(name, _)| name.clone())
            .unwrap_or_default();
        let mut pending = self
            .pending_specs
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let before = pending.len();
        pending
            .retain(|p| !(p.producer == producer && p.branch == branch && p.api_type == api_type));
        Ok(pending.len() != before)
    }

    // --- Producer Onboarding ---

    async fn set_producer_onboarding(
        &self,
        service_id: i64,
        onboarding: bool,
    ) -> Result<(), RepositoryError> {
        let mut o = self
            .onboarding
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if onboarding {
            if !o.contains(&service_id) {
                o.push(service_id);
            }
        } else {
            o.retain(|id| *id != service_id);
        }
        Ok(())
    }

    async fn is_producer_onboarding(&self, service_id: i64) -> Result<bool, RepositoryError> {
        Ok(self
            .onboarding
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .contains(&service_id))
    }

    async fn list_onboarding_producers(&self) -> Result<Vec<String>, RepositoryError> {
        let o = self
            .onboarding
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let services = self.services.lock().unwrap_or_else(PoisonError::into_inner);
        let mut out: Vec<String> = services
            .iter()
            .filter(|(_, sid)| o.contains(sid))
            .map(|(name, _)| name.clone())
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

    async fn get_channel_message_contract(
        &self,
        branch_name: &str,
        channel: &str,
        message_name: &str,
    ) -> Result<Option<ChannelMessageContract>, RepositoryError> {
        let contracts = self
            .channel_message_contracts
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        Ok(contracts
            .iter()
            .find(|c| {
                c.branch_name == branch_name
                    && c.channel == channel
                    && c.message_name == message_name
            })
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
        match contracts.iter_mut().find(|c| {
            c.branch_name == contract.branch_name
                && c.channel == contract.channel
                && c.message_name == contract.message_name
        }) {
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
        branch_name: &str,
        channel: &str,
        message_name: &str,
    ) -> Result<(), RepositoryError> {
        let mut contracts = self
            .channel_message_contracts
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        contracts.retain(|c| {
            !(c.branch_name == branch_name
                && c.channel == channel
                && c.message_name == message_name)
        });
        Ok(())
    }

    async fn list_channel_message_contracts(
        &self,
        branch_name: &str,
    ) -> Result<Vec<ChannelMessageContract>, RepositoryError> {
        let contracts = self
            .channel_message_contracts
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let mut result: Vec<ChannelMessageContract> = contracts
            .iter()
            .filter(|c| c.branch_name == branch_name)
            .cloned()
            .collect();
        result.sort_by(|a, b| {
            a.channel
                .cmp(&b.channel)
                .then_with(|| a.message_name.cmp(&b.message_name))
        });
        Ok(result)
    }

    async fn delete_orphaned_channel_message_contracts(
        &self,
        live_branches: &[String],
    ) -> Result<u64, RepositoryError> {
        let mut contracts = self
            .channel_message_contracts
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let before = contracts.len();
        contracts.retain(|c| live_branches.iter().any(|b| b == &c.branch_name));
        Ok((before - contracts.len()) as u64)
    }

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
            branch: log.branch.map(|b| b.to_string()),
            action_type: log.action_type.map(|t| t.to_string()),
            diff: log.diff.map(|d| d.to_string()),
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

    async fn list_branches_with_metadata(&self) -> Result<Vec<BranchMetadata>, RepositoryError> {
        let branches = self.branches.lock().unwrap_or_else(PoisonError::into_inner);
        let timestamps = self
            .branch_timestamps
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let mut result = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for ((_, bname), _) in branches.iter() {
            if seen.insert(bname.clone()) {
                let last_modified = timestamps
                    .get(bname)
                    .cloned()
                    .unwrap_or_else(|| chrono::Utc::now().to_rfc3339());
                result.push(BranchMetadata {
                    name: bname.clone(),
                    last_modified,
                });
            }
        }
        Ok(result)
    }

    async fn list_branch_last_published(
        &self,
    ) -> Result<Vec<(String, String, String)>, RepositoryError> {
        // MockRepo does not track per-branch publish times, so recency ordering is
        // exercised by the sqlite-backed integration tests instead.
        Ok(Vec::new())
    }

    async fn list_branch_endpoint_counts(
        &self,
    ) -> Result<Vec<(String, String, i64)>, RepositoryError> {
        // MockRepo does not track per-branch endpoint counts; exercised by the
        // sqlite-backed integration tests instead.
        Ok(Vec::new())
    }
}
