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
        let pb = self
            .protected_branches
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        Ok(pb.contains(&branch_name.to_string()))
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
        let mut users = self.users.lock().unwrap_or_else(PoisonError::into_inner);
        let initial_count = users.len() as u64;
        users.retain(|u| u.is_admin);
        Ok(initial_count - users.len() as u64)
    }

    async fn nuke_database(&self, _keep_user_id: Option<i64>) -> Result<(), RepositoryError> {
        self.delete_all_services().await?;
        self.delete_all_clients().await?;
        Ok(())
    }

    async fn delete_service(&self, name: &str) -> Result<bool, RepositoryError> {
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

    async fn delete_client(&self, name: &str) -> Result<bool, RepositoryError> {
        let mut clients = self.clients.lock().unwrap_or_else(PoisonError::into_inner);
        Ok(clients.remove(name).is_some())
    }

    async fn list_services(&self) -> Result<Vec<String>, RepositoryError> {
        Ok(self
            .services
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .keys()
            .cloned()
            .collect())
    }

    async fn list_services_detailed(&self) -> Result<Vec<ServiceSummary>, RepositoryError> {
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
            result.push(ServiceSummary {
                name: name.clone(),
                fallback_branch: fallback_branches.get(name).cloned(),
                branches: svc_branches,
                branches_last_published: std::collections::HashMap::new(),
                branches_expire_at: std::collections::HashMap::new(),
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

    async fn update_service_metadata(
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

    async fn list_branches(&self, _service_name: &str) -> Result<Vec<String>, RepositoryError> {
        Ok(Vec::new())
    }

    async fn list_all_branches(&self) -> Result<Vec<String>, RepositoryError> {
        Ok(Vec::new())
    }

    async fn list_clients(&self) -> Result<Vec<String>, RepositoryError> {
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

    async fn list_client_branches(
        &self,
        _client_name: &str,
    ) -> Result<Vec<String>, RepositoryError> {
        Ok(Vec::new())
    }

    async fn list_client_endpoints(
        &self,
        _client_name: &str,
        _branch: &str,
    ) -> Result<Vec<ClientEndpointInfo>, RepositoryError> {
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
        is_admin: bool,
        approved: bool,
    ) -> Result<User, RepositoryError> {
        let user = User {
            id: self.next_id(),
            username: username.to_string(),
            password_hash: password_hash.to_string(),
            is_admin,
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
}
