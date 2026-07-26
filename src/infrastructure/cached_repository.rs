use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use moka::future::Cache;

use crate::domain::models::*;
use crate::domain::ports::{
    EndpointMap, NewAuditLog, RecordDependencyParams, RepositoryError, SpecRepository,
    UpdateEndpointParams,
};
use crate::infrastructure::database::DatabaseRepo;

type EndpointKey = (i64, String, String, String, String); // (service_id, branch, api_type, norm_path, method)

#[derive(Clone)]
pub struct CachedSpecRepository {
    inner: Box<DatabaseRepo>,
    // Endpoint data (60% of budget)
    endpoint_cache: Cache<EndpointKey, Arc<(i64, String, bool, bool)>>,
    branch_endpoints_cache: Cache<i64, Arc<Vec<EndpointRecord>>>,
    // Reports (20%)
    report_cache: Cache<String, Arc<DependencyReport>>,
    services_cache: Cache<String, Arc<Vec<ServiceSummary>>>,
    // ID lookups (10%)
    service_id_cache: Cache<String, i64>,
    branch_id_cache: Cache<(i64, String), i64>,
    // Metadata (10%)
    protected_branches_cache: Cache<String, Arc<Vec<String>>>,
    fallback_branch_cache: Cache<String, Option<String>>,
    branch_protected_cache: Cache<String, bool>,
    services_list_cache: Cache<String, Arc<Vec<String>>>,
    branches_list_cache: Cache<String, Arc<Vec<String>>>,
    clients_list_cache: Cache<String, Arc<Vec<String>>>,
    // Stats
    hits: Arc<AtomicU64>,
    misses: Arc<AtomicU64>,
    memory_limit_mb: Arc<AtomicU64>,
}

fn mb_to_bytes(mb: u64) -> u64 {
    mb * 1024 * 1024
}

struct RepoCaches {
    endpoint_cache: Cache<EndpointKey, Arc<(i64, String, bool, bool)>>,
    branch_endpoints_cache: Cache<i64, Arc<Vec<EndpointRecord>>>,
    report_cache: Cache<String, Arc<DependencyReport>>,
    services_cache: Cache<String, Arc<Vec<ServiceSummary>>>,
    service_id_cache: Cache<String, i64>,
    branch_id_cache: Cache<(i64, String), i64>,
    protected_branches_cache: Cache<String, Arc<Vec<String>>>,
    fallback_branch_cache: Cache<String, Option<String>>,
    branch_protected_cache: Cache<String, bool>,
    services_list_cache: Cache<String, Arc<Vec<String>>>,
    branches_list_cache: Cache<String, Arc<Vec<String>>>,
    clients_list_cache: Cache<String, Arc<Vec<String>>>,
}

impl CachedSpecRepository {
    pub fn new(inner: DatabaseRepo, memory_limit_mb: u64) -> Self {
        let caches = Self::build_caches(memory_limit_mb);
        Self {
            inner: Box::new(inner),
            endpoint_cache: caches.endpoint_cache,
            branch_endpoints_cache: caches.branch_endpoints_cache,
            report_cache: caches.report_cache,
            services_cache: caches.services_cache,
            service_id_cache: caches.service_id_cache,
            branch_id_cache: caches.branch_id_cache,
            protected_branches_cache: caches.protected_branches_cache,
            fallback_branch_cache: caches.fallback_branch_cache,
            branch_protected_cache: caches.branch_protected_cache,
            services_list_cache: caches.services_list_cache,
            branches_list_cache: caches.branches_list_cache,
            clients_list_cache: caches.clients_list_cache,
            hits: Arc::new(AtomicU64::new(0)),
            misses: Arc::new(AtomicU64::new(0)),
            memory_limit_mb: Arc::new(AtomicU64::new(memory_limit_mb)),
        }
    }

    fn build_caches(memory_limit_mb: u64) -> RepoCaches {
        let total = mb_to_bytes(memory_limit_mb);
        let endpoint_budget = total * 60 / 100;
        let report_budget = total * 20 / 100;
        let id_budget = total * 10 / 100;
        let meta_budget = total * 10 / 100;

        // Endpoint data caches (60%)
        let endpoint_cache = Cache::builder()
            .max_capacity(endpoint_budget / 2)
            .weigher(|_k: &EndpointKey, v: &Arc<(i64, String, bool, bool)>| (v.1.len() + 80) as u32)
            .build();

        let branch_endpoints_cache = Cache::builder()
            .max_capacity(endpoint_budget / 2)
            .weigher(|_k: &i64, v: &Arc<Vec<EndpointRecord>>| {
                let size: usize = v
                    .iter()
                    .map(|e| {
                        e.path.len()
                            + e.normalized_path.len()
                            + e.method.len()
                            + e.yaml_content.len()
                            + 80
                    })
                    .sum();
                (size + 24) as u32
            })
            .build();

        // Report caches (20%)
        let report_cache = Cache::builder()
            .max_capacity(report_budget / 2)
            .weigher(|_k: &String, v: &Arc<DependencyReport>| {
                let size = v.branch.len()
                    + v.unused_endpoints.len() * 120
                    + v.missing_endpoints.len() * 150
                    + v.dependency_graph.len() * 150
                    + 80;
                size as u32
            })
            .build();

        let services_cache = Cache::builder()
            .max_capacity(report_budget / 2)
            .weigher(|_k: &String, v: &Arc<Vec<ServiceSummary>>| {
                let size: usize = v
                    .iter()
                    .map(|s| {
                        s.name.len()
                            + s.fallback_branch.as_ref().map_or(0, |b| b.len())
                            + s.branches.iter().map(|b| b.len() + 24).sum::<usize>()
                            + s.icon.as_ref().map_or(0, |i| i.len())
                            + s.domain.as_ref().map_or(0, |d| d.len())
                            + 100
                    })
                    .sum();
                (size + 24) as u32
            })
            .build();

        // ID lookup caches (10%)
        let service_id_cache = Cache::builder()
            .max_capacity(id_budget / 2)
            .weigher(|k: &String, _v: &i64| (k.len() + 32) as u32)
            .build();

        let branch_id_cache = Cache::builder()
            .max_capacity(id_budget / 2)
            .weigher(|k: &(i64, String), _v: &i64| (k.1.len() + 40) as u32)
            .build();

        // Metadata caches (10%)
        let meta_each = meta_budget / 6;

        let protected_branches_cache = Cache::builder()
            .max_capacity(meta_each)
            .weigher(|_k: &String, v: &Arc<Vec<String>>| {
                let size: usize = v.iter().map(|s| s.len() + 24).sum();
                (size + 24) as u32
            })
            .build();

        let fallback_branch_cache = Cache::builder()
            .max_capacity(meta_each)
            .weigher(|k: &String, v: &Option<String>| {
                (k.len() + v.as_ref().map_or(0, |s| s.len()) + 32) as u32
            })
            .build();

        let branch_protected_cache = Cache::builder()
            .max_capacity(meta_each)
            .weigher(|k: &String, _v: &bool| (k.len() + 25) as u32)
            .build();

        let services_list_cache = Cache::builder()
            .max_capacity(meta_each)
            .weigher(|_k: &String, v: &Arc<Vec<String>>| {
                let size: usize = v.iter().map(|s| s.len() + 24).sum();
                (size + 24) as u32
            })
            .build();

        let branches_list_cache = Cache::builder()
            .max_capacity(meta_each)
            .weigher(|_k: &String, v: &Arc<Vec<String>>| {
                let size: usize = v.iter().map(|s| s.len() + 24).sum();
                (size + 24) as u32
            })
            .build();

        let clients_list_cache = Cache::builder()
            .max_capacity(meta_each)
            .weigher(|_k: &String, v: &Arc<Vec<String>>| {
                let size: usize = v.iter().map(|s| s.len() + 24).sum();
                (size + 24) as u32
            })
            .build();

        RepoCaches {
            endpoint_cache,
            branch_endpoints_cache,
            report_cache,
            services_cache,
            service_id_cache,
            branch_id_cache,
            protected_branches_cache,
            fallback_branch_cache,
            branch_protected_cache,
            services_list_cache,
            branches_list_cache,
            clients_list_cache,
        }
    }

    pub async fn cache_stats(&self) -> CacheStats {
        // Run maintenance tasks to get accurate stats
        self.endpoint_cache.run_pending_tasks().await;
        self.branch_endpoints_cache.run_pending_tasks().await;
        self.report_cache.run_pending_tasks().await;
        self.services_cache.run_pending_tasks().await;
        self.service_id_cache.run_pending_tasks().await;
        self.branch_id_cache.run_pending_tasks().await;
        self.protected_branches_cache.run_pending_tasks().await;
        self.fallback_branch_cache.run_pending_tasks().await;
        self.branch_protected_cache.run_pending_tasks().await;
        self.services_list_cache.run_pending_tasks().await;
        self.branches_list_cache.run_pending_tasks().await;
        self.clients_list_cache.run_pending_tasks().await;

        let hits = self.hits.load(Ordering::Relaxed);
        let misses = self.misses.load(Ordering::Relaxed);
        let total = hits + misses;
        let hit_rate = if total > 0 {
            hits as f64 / total as f64 * 100.0
        } else {
            0.0
        };
        let limit_mb = self.memory_limit_mb.load(Ordering::Relaxed);

        let estimated_bytes = self.endpoint_cache.weighted_size()
            + self.branch_endpoints_cache.weighted_size()
            + self.report_cache.weighted_size()
            + self.services_cache.weighted_size()
            + self.service_id_cache.weighted_size()
            + self.branch_id_cache.weighted_size()
            + self.protected_branches_cache.weighted_size()
            + self.fallback_branch_cache.weighted_size()
            + self.branch_protected_cache.weighted_size()
            + self.services_list_cache.weighted_size()
            + self.branches_list_cache.weighted_size()
            + self.clients_list_cache.weighted_size();

        let entry_count = self.endpoint_cache.entry_count()
            + self.branch_endpoints_cache.entry_count()
            + self.report_cache.entry_count()
            + self.services_cache.entry_count()
            + self.service_id_cache.entry_count()
            + self.branch_id_cache.entry_count()
            + self.protected_branches_cache.entry_count()
            + self.fallback_branch_cache.entry_count()
            + self.branch_protected_cache.entry_count()
            + self.services_list_cache.entry_count()
            + self.branches_list_cache.entry_count()
            + self.clients_list_cache.entry_count();

        CacheStats {
            enabled: limit_mb > 0,
            memory_limit_mb: limit_mb,
            estimated_memory_used_bytes: estimated_bytes,
            entry_count,
            hit_count: hits,
            miss_count: misses,
            hit_rate_percent: hit_rate,
        }
    }

    pub fn rebuild_caches(&self, new_limit_mb: u64) {
        self.memory_limit_mb.store(new_limit_mb, Ordering::Relaxed);
        // Invalidate all existing caches
        self.endpoint_cache.invalidate_all();
        self.branch_endpoints_cache.invalidate_all();
        self.report_cache.invalidate_all();
        self.services_cache.invalidate_all();
        self.service_id_cache.invalidate_all();
        self.branch_id_cache.invalidate_all();
        self.protected_branches_cache.invalidate_all();
        self.fallback_branch_cache.invalidate_all();
        self.branch_protected_cache.invalidate_all();
        self.services_list_cache.invalidate_all();
        self.branches_list_cache.invalidate_all();
        self.clients_list_cache.invalidate_all();
        self.hits.store(0, Ordering::Relaxed);
        self.misses.store(0, Ordering::Relaxed);
    }

    pub fn inner(&self) -> &DatabaseRepo {
        &self.inner
    }

    pub fn backend_name(&self) -> &'static str {
        self.inner.backend_name()
    }

    fn is_disabled(&self) -> bool {
        self.memory_limit_mb.load(Ordering::Relaxed) == 0
    }

    fn record_hit(&self) {
        self.hits.fetch_add(1, Ordering::Relaxed);
    }

    fn record_miss(&self) {
        self.misses.fetch_add(1, Ordering::Relaxed);
    }

    async fn invalidate_all_caches(&self) {
        self.service_id_cache.invalidate_all();
        self.branch_id_cache.invalidate_all();
        self.branch_endpoints_cache.invalidate_all();
        self.endpoint_cache.invalidate_all();
        self.report_cache.invalidate_all();
        self.services_list_cache.invalidate_all();
        self.services_cache.invalidate_all();
        self.branches_list_cache.invalidate_all();
        self.clients_list_cache.invalidate_all();
        self.protected_branches_cache.invalidate_all();
        self.fallback_branch_cache.invalidate_all();
        self.branch_protected_cache.invalidate_all();
    }
}

impl SpecRepository for CachedSpecRepository {
    // --- Cached reads with write-through invalidation ---

    async fn ping(&self) -> Result<(), RepositoryError> {
        self.inner.ping().await
    }

    async fn get_spec_version(
        &self,
        service_id: i64,
        branch_id: i64,
    ) -> Result<Option<(SemVer, String)>, RepositoryError> {
        self.inner.get_spec_version(service_id, branch_id).await
    }

    async fn increment_spec_version(
        &self,
        service_id: i64,
        branch_id: i64,
        content_hash: &str,
        impact: Impact,
    ) -> Result<SemVer, RepositoryError> {
        self.inner
            .increment_spec_version(service_id, branch_id, content_hash, impact)
            .await
    }

    async fn ensure_service(&self, name: &str) -> Result<i64, RepositoryError> {
        let id = self.inner.ensure_service(name).await?;
        if !self.is_disabled() {
            self.service_id_cache.insert(name.to_string(), id).await;
            self.services_list_cache.invalidate_all();
            self.services_cache.invalidate_all();
        }
        Ok(id)
    }

    async fn find_service(&self, name: &str) -> Result<Option<i64>, RepositoryError> {
        if !self.is_disabled()
            && let Some(id) = self.service_id_cache.get(&name.to_string()).await
        {
            self.record_hit();
            return Ok(Some(id));
        }
        self.record_miss();
        let result = self.inner.find_service(name).await?;
        if !self.is_disabled()
            && let Some(id) = result
        {
            self.service_id_cache.insert(name.to_string(), id).await;
        }
        Ok(result)
    }

    async fn get_service_name_by_id(
        &self,
        service_id: i64,
    ) -> Result<Option<String>, RepositoryError> {
        self.inner.get_service_name_by_id(service_id).await
    }

    async fn ensure_branch(
        &self,
        service_id: i64,
        branch_name: &str,
    ) -> Result<i64, RepositoryError> {
        let id = self.inner.ensure_branch(service_id, branch_name).await?;
        if !self.is_disabled() {
            self.branch_id_cache
                .insert((service_id, branch_name.to_string()), id)
                .await;
            self.branches_list_cache.invalidate_all();
        }
        Ok(id)
    }

    async fn find_branch(
        &self,
        service_id: i64,
        branch_name: &str,
    ) -> Result<Option<i64>, RepositoryError> {
        if !self.is_disabled() {
            let key = (service_id, branch_name.to_string());
            if let Some(id) = self.branch_id_cache.get(&key).await {
                self.record_hit();
                return Ok(Some(id));
            }
        }
        self.record_miss();
        let result = self.inner.find_branch(service_id, branch_name).await?;
        if !self.is_disabled()
            && let Some(id) = result
        {
            self.branch_id_cache
                .insert((service_id, branch_name.to_string()), id)
                .await;
        }
        Ok(result)
    }

    async fn get_endpoints_for_branch(
        &self,
        branch_id: i64,
    ) -> Result<Vec<EndpointRecord>, RepositoryError> {
        if !self.is_disabled()
            && let Some(cached) = self.branch_endpoints_cache.get(&branch_id).await
        {
            self.record_hit();
            return Ok((*cached).clone());
        }
        self.record_miss();
        let result = self.inner.get_endpoints_for_branch(branch_id).await?;
        if !self.is_disabled() {
            self.branch_endpoints_cache
                .insert(branch_id, Arc::new(result.clone()))
                .await;
        }
        Ok(result)
    }

    async fn insert_endpoint(
        &self,
        branch_id: i64,
        endpoint: &EndpointRecord,
    ) -> Result<(), RepositoryError> {
        self.inner.insert_endpoint(branch_id, endpoint).await?;
        if !self.is_disabled() {
            self.branch_endpoints_cache.invalidate(&branch_id).await;
            self.report_cache.invalidate_all();
        }
        Ok(())
    }

    async fn reset_branch_history(
        &self,
        service_name: &str,
        branch_name: &str,
    ) -> Result<bool, RepositoryError> {
        let res = self
            .inner
            .reset_branch_history(service_name, branch_name)
            .await?;
        if res && !self.is_disabled() {
            // Invalidate caches
            if let Ok(Some(service_id)) = self.find_service(service_name).await
                && let Ok(Some(branch_id)) = self.find_branch(service_id, branch_name).await
            {
                self.branch_endpoints_cache.invalidate(&branch_id).await;
            }
            self.report_cache.invalidate(&branch_name.to_string()).await;
        }
        Ok(res)
    }

    async fn ensure_client(&self, name: &str) -> Result<i64, RepositoryError> {
        let id = self.inner.ensure_client(name).await?;
        if !self.is_disabled() {
            self.clients_list_cache.invalidate_all();
        }
        Ok(id)
    }

    async fn find_endpoint(
        &self,
        service_id: i64,
        branch_name: &str,
        api_type: ApiType,
        path: &str,
        method: &str,
    ) -> Result<Option<(i64, String, bool, bool)>, RepositoryError> {
        if !self.is_disabled() {
            let key = (
                service_id,
                branch_name.to_string(),
                format!("{:?}", api_type),
                path.to_string(),
                method.to_string(),
            );
            if let Some(cached) = self.endpoint_cache.get(&key).await {
                self.record_hit();
                return Ok(Some((cached.0, cached.1.clone(), cached.2, cached.3)));
            }
        }
        self.record_miss();
        let result = self
            .inner
            .find_endpoint(service_id, branch_name, api_type, path, method)
            .await?;
        if !self.is_disabled()
            && let Some(ref val) = result
        {
            let key = (
                service_id,
                branch_name.to_string(),
                format!("{:?}", api_type),
                path.to_string(),
                method.to_string(),
            );
            self.endpoint_cache.insert(key, Arc::new(val.clone())).await;
        }
        Ok(result)
    }

    async fn find_endpoints_bulk(
        &self,
        service_id: i64,
        branch_name: &str,
        api_type: ApiType,
        endpoints: &[(String, String)],
    ) -> Result<EndpointMap, RepositoryError> {
        if self.is_disabled() {
            return self
                .inner
                .find_endpoints_bulk(service_id, branch_name, api_type, endpoints)
                .await;
        }
        // Try cache first for each endpoint
        let api_str = format!("{:?}", api_type);
        let mut result = EndpointMap::new();
        let mut misses = Vec::new();
        for (path, method) in endpoints {
            let key = (
                service_id,
                branch_name.to_string(),
                api_str.clone(),
                path.clone(),
                method.clone(),
            );
            if let Some(cached) = self.endpoint_cache.get(&key).await {
                self.record_hit();
                result.insert(
                    (path.clone(), method.clone()),
                    (cached.0, cached.1.clone(), cached.2, cached.3),
                );
            } else {
                misses.push((path.clone(), method.clone()));
            }
        }
        if !misses.is_empty() {
            self.record_miss();
            let db_results = self
                .inner
                .find_endpoints_bulk(service_id, branch_name, api_type, &misses)
                .await?;
            for ((path, method), val) in &db_results {
                let key = (
                    service_id,
                    branch_name.to_string(),
                    api_str.clone(),
                    path.clone(),
                    method.clone(),
                );
                self.endpoint_cache.insert(key, Arc::new(val.clone())).await;
            }
            result.extend(db_results);
        }
        Ok(result)
    }

    async fn record_dependency(
        &self,
        params: RecordDependencyParams<'_>,
    ) -> Result<(), RepositoryError> {
        self.inner.record_dependency(params).await?;
        if !self.is_disabled() {
            self.report_cache.invalidate_all();
        }
        Ok(())
    }

    async fn record_dependencies_bulk(
        &self,
        params: Vec<RecordDependencyParams<'_>>,
    ) -> Result<(), RepositoryError> {
        self.inner.record_dependencies_bulk(params).await?;
        if !self.is_disabled() {
            self.report_cache.invalidate_all();
        }
        Ok(())
    }

    async fn get_report(&self, branch: &str) -> Result<DependencyReport, RepositoryError> {
        if !self.is_disabled()
            && let Some(cached) = self.report_cache.get(&branch.to_string()).await
        {
            self.record_hit();
            return Ok((*cached).clone());
        }
        self.record_miss();
        let result = self.inner.get_report(branch).await?;
        if !self.is_disabled() {
            self.report_cache
                .insert(branch.to_string(), Arc::new(result.clone()))
                .await;
        }
        Ok(result)
    }

    async fn is_branch_protected(&self, branch_name: &str) -> Result<bool, RepositoryError> {
        if !self.is_disabled()
            && let Some(cached) = self
                .branch_protected_cache
                .get(&branch_name.to_string())
                .await
        {
            self.record_hit();
            return Ok(cached);
        }
        self.record_miss();
        let result = self.inner.is_branch_protected(branch_name).await?;
        if !self.is_disabled() {
            self.branch_protected_cache
                .insert(branch_name.to_string(), result)
                .await;
        }
        Ok(result)
    }

    async fn add_protected_branch(&self, pattern: &str) -> Result<(), RepositoryError> {
        self.inner.add_protected_branch(pattern).await?;
        if !self.is_disabled() {
            self.branch_protected_cache.invalidate_all();
            self.protected_branches_cache.invalidate_all();
        }
        Ok(())
    }

    async fn remove_protected_branch(&self, pattern: &str) -> Result<bool, RepositoryError> {
        let result = self.inner.remove_protected_branch(pattern).await?;
        if !self.is_disabled() {
            self.branch_protected_cache.invalidate_all();
            self.protected_branches_cache.invalidate_all();
        }
        Ok(result)
    }

    async fn list_protected_branches(&self) -> Result<Vec<String>, RepositoryError> {
        let sentinel = "_all_".to_string();
        if !self.is_disabled()
            && let Some(cached) = self.protected_branches_cache.get(&sentinel).await
        {
            self.record_hit();
            return Ok((*cached).clone());
        }
        self.record_miss();
        let result = self.inner.list_protected_branches().await?;
        if !self.is_disabled() {
            self.protected_branches_cache
                .insert(sentinel, Arc::new(result.clone()))
                .await;
        }
        Ok(result)
    }

    async fn update_endpoint(
        &self,
        params: UpdateEndpointParams<'_>,
    ) -> Result<(), RepositoryError> {
        let branch_id = params.branch_id;
        self.inner.update_endpoint(params).await?;
        if !self.is_disabled() {
            self.branch_endpoints_cache.invalidate(&branch_id).await;
            self.endpoint_cache.invalidate_all();
            self.report_cache.invalidate_all();
        }
        Ok(())
    }

    async fn soft_delete_endpoint(
        &self,
        branch_id: i64,
        api_type: ApiType,
        path: &str,
        method: &str,
    ) -> Result<(), RepositoryError> {
        self.inner
            .soft_delete_endpoint(branch_id, api_type, path, method)
            .await?;
        if !self.is_disabled() {
            self.branch_endpoints_cache.invalidate(&branch_id).await;
            self.endpoint_cache.invalidate_all();
            self.report_cache.invalidate_all();
        }
        Ok(())
    }

    async fn hard_delete_endpoint(
        &self,
        branch_id: i64,
        api_type: ApiType,
        path: &str,
        method: &str,
    ) -> Result<(), RepositoryError> {
        self.inner
            .hard_delete_endpoint(branch_id, api_type, path, method)
            .await?;
        if !self.is_disabled() {
            self.branch_endpoints_cache.invalidate(&branch_id).await;
            self.endpoint_cache.invalidate_all();
            self.report_cache.invalidate_all();
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
        self.inner
            .is_endpoint_deleted(branch_id, api_type, path, method)
            .await
    }

    async fn delete_all_services(&self) -> Result<u64, RepositoryError> {
        let result = self.inner.delete_all_services().await?;
        if !self.is_disabled() {
            self.invalidate_all_caches().await;
        }
        Ok(result)
    }

    async fn delete_all_clients(&self) -> Result<u64, RepositoryError> {
        let result = self.inner.delete_all_clients().await?;
        if !self.is_disabled() {
            self.clients_list_cache.invalidate_all();
            self.report_cache.invalidate_all();
        }
        Ok(result)
    }

    async fn delete_all_non_admin_users(&self) -> Result<u64, RepositoryError> {
        let result = self.inner.delete_all_non_admin_users().await?;
        Ok(result)
    }

    async fn nuke_database(&self, keep_user_id: Option<i64>) -> Result<(), RepositoryError> {
        self.inner.nuke_database(keep_user_id).await?;
        if !self.is_disabled() {
            self.invalidate_all_caches().await;
        }
        Ok(())
    }

    async fn delete_service(&self, name: &str) -> Result<bool, RepositoryError> {
        let result = self.inner.delete_service(name).await?;
        if !self.is_disabled() {
            self.service_id_cache.invalidate(&name.to_string()).await;
            self.branch_id_cache.invalidate_all();
            self.branch_endpoints_cache.invalidate_all();
            self.endpoint_cache.invalidate_all();
            self.report_cache.invalidate_all();
            self.services_list_cache.invalidate_all();
            self.services_cache.invalidate_all();
            self.branches_list_cache.invalidate_all();
            self.clients_list_cache.invalidate_all();
        }
        Ok(result)
    }

    async fn delete_branch(
        &self,
        service_name: &str,
        branch_name: &str,
    ) -> Result<bool, RepositoryError> {
        let result = self.inner.delete_branch(service_name, branch_name).await?;
        if !self.is_disabled() {
            self.branch_id_cache.invalidate_all();
            self.branch_endpoints_cache.invalidate_all();
            self.endpoint_cache.invalidate_all();
            self.report_cache.invalidate_all();
            self.branches_list_cache.invalidate_all();
        }
        Ok(result)
    }

    async fn delete_client(&self, name: &str) -> Result<bool, RepositoryError> {
        let result = self.inner.delete_client(name).await?;
        if !self.is_disabled() {
            self.clients_list_cache.invalidate_all();
            self.report_cache.invalidate_all();
        }
        Ok(result)
    }

    async fn list_services(&self) -> Result<Vec<String>, RepositoryError> {
        let sentinel = "_all_".to_string();
        if !self.is_disabled()
            && let Some(cached) = self.services_list_cache.get(&sentinel).await
        {
            self.record_hit();
            return Ok((*cached).clone());
        }
        self.record_miss();
        let result = self.inner.list_services().await?;
        if !self.is_disabled() {
            self.services_list_cache
                .insert(sentinel, Arc::new(result.clone()))
                .await;
        }
        Ok(result)
    }

    async fn list_services_detailed(&self) -> Result<Vec<ServiceSummary>, RepositoryError> {
        let sentinel = "_all_".to_string();
        if !self.is_disabled()
            && let Some(cached) = self.services_cache.get(&sentinel).await
        {
            self.record_hit();
            return Ok((*cached).clone());
        }
        self.record_miss();
        let result = self.inner.list_services_detailed().await?;
        if !self.is_disabled() {
            self.services_cache
                .insert(sentinel, Arc::new(result.clone()))
                .await;
        }
        Ok(result)
    }

    async fn set_fallback_branch(
        &self,
        service_name: &str,
        branch: Option<&str>,
    ) -> Result<(), RepositoryError> {
        self.inner.set_fallback_branch(service_name, branch).await?;
        if !self.is_disabled() {
            self.fallback_branch_cache
                .invalidate(&service_name.to_string())
                .await;
            self.services_cache.invalidate_all();
        }
        Ok(())
    }

    async fn update_service_metadata(
        &self,
        service_name: &str,
        icon: Option<&str>,
        domain: Option<&str>,
    ) -> Result<(), RepositoryError> {
        self.inner
            .update_service_metadata(service_name, icon, domain)
            .await?;
        if !self.is_disabled() {
            self.services_cache.invalidate_all();
        }
        Ok(())
    }

    async fn get_fallback_branch(
        &self,
        service_name: &str,
    ) -> Result<Option<String>, RepositoryError> {
        if !self.is_disabled()
            && let Some(cached) = self
                .fallback_branch_cache
                .get(&service_name.to_string())
                .await
        {
            self.record_hit();
            return Ok(cached);
        }
        self.record_miss();
        let result = self.inner.get_fallback_branch(service_name).await?;
        if !self.is_disabled() {
            self.fallback_branch_cache
                .insert(service_name.to_string(), result.clone())
                .await;
        }
        Ok(result)
    }

    async fn list_branches(&self, service_name: &str) -> Result<Vec<String>, RepositoryError> {
        if !self.is_disabled()
            && let Some(cached) = self
                .branches_list_cache
                .get(&service_name.to_string())
                .await
        {
            self.record_hit();
            return Ok((*cached).clone());
        }
        self.record_miss();
        let result = self.inner.list_branches(service_name).await?;
        if !self.is_disabled() {
            self.branches_list_cache
                .insert(service_name.to_string(), Arc::new(result.clone()))
                .await;
        }
        Ok(result)
    }

    async fn list_all_branches(&self) -> Result<Vec<String>, RepositoryError> {
        self.inner.list_all_branches().await
    }

    async fn list_clients(&self) -> Result<Vec<String>, RepositoryError> {
        let sentinel = "_all_".to_string();
        if !self.is_disabled()
            && let Some(cached) = self.clients_list_cache.get(&sentinel).await
        {
            self.record_hit();
            return Ok((*cached).clone());
        }
        self.record_miss();
        let result = self.inner.list_clients().await?;
        if !self.is_disabled() {
            self.clients_list_cache
                .insert(sentinel, Arc::new(result.clone()))
                .await;
        }
        Ok(result)
    }

    async fn list_client_branches(
        &self,
        client_name: &str,
    ) -> Result<Vec<String>, RepositoryError> {
        self.inner.list_client_branches(client_name).await
    }

    async fn list_client_endpoints(
        &self,
        client_name: &str,
        branch: &str,
    ) -> Result<Vec<ClientEndpointInfo>, RepositoryError> {
        self.inner.list_client_endpoints(client_name, branch).await
    }

    // --- Non-cached pass-through (auth/session/settings/user) ---

    async fn user_count(&self) -> Result<i64, RepositoryError> {
        self.inner.user_count().await
    }

    async fn find_user(&self, username: &str) -> Result<Option<User>, RepositoryError> {
        self.inner.find_user(username).await
    }

    async fn create_user(
        &self,
        username: &str,
        password_hash: &str,
        is_admin: bool,
        approved: bool,
    ) -> Result<User, RepositoryError> {
        self.inner
            .create_user(username, password_hash, is_admin, approved)
            .await
    }

    async fn update_password(&self, user_id: i64, new_hash: &str) -> Result<(), RepositoryError> {
        self.inner.update_password(user_id, new_hash).await
    }

    async fn list_users(&self) -> Result<Vec<User>, RepositoryError> {
        self.inner.list_users().await
    }

    async fn approve_user(&self, user_id: i64) -> Result<bool, RepositoryError> {
        self.inner.approve_user(user_id).await
    }

    async fn delete_user(&self, user_id: i64) -> Result<bool, RepositoryError> {
        self.inner.delete_user(user_id).await
    }

    async fn create_session(
        &self,
        user_id: i64,
        expires_at: &str,
    ) -> Result<Session, RepositoryError> {
        self.inner.create_session(user_id, expires_at).await
    }

    async fn create_session_with_token(
        &self,
        user_id: i64,
        token: &str,
        expires_at: &str,
    ) -> Result<Session, RepositoryError> {
        self.inner
            .create_session_with_token(user_id, token, expires_at)
            .await
    }

    async fn validate_session(
        &self,
        token: &str,
    ) -> Result<Option<(User, Session)>, RepositoryError> {
        self.inner.validate_session(token).await
    }

    async fn delete_session(&self, token: &str) -> Result<(), RepositoryError> {
        self.inner.delete_session(token).await
    }

    async fn get_setting(&self, key: &str) -> Result<Option<String>, RepositoryError> {
        self.inner.get_setting(key).await
    }

    async fn set_setting(&self, key: &str, value: &str) -> Result<(), RepositoryError> {
        self.inner.set_setting(key, value).await
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
        self.inner
            .create_api_token(id, user_id, name, token_hash, created_at, expires_at)
            .await
    }

    async fn list_api_tokens(&self, user_id: i64) -> Result<Vec<ApiToken>, RepositoryError> {
        self.inner.list_api_tokens(user_id).await
    }

    async fn delete_api_token(
        &self,
        token_id: &str,
        user_id: i64,
    ) -> Result<Option<String>, RepositoryError> {
        self.inner.delete_api_token(token_id, user_id).await
    }

    async fn validate_api_token(&self, token_hash: &str) -> Result<Option<User>, RepositoryError> {
        self.inner.validate_api_token(token_hash).await
    }

    async fn delete_stale_branches(&self, cutoff_iso: &str) -> Result<u64, RepositoryError> {
        let result = self.inner.delete_stale_branches(cutoff_iso).await?;
        if !self.is_disabled() && result > 0 {
            self.branch_id_cache.invalidate_all();
            self.branch_endpoints_cache.invalidate_all();
            self.endpoint_cache.invalidate_all();
            self.branches_list_cache.invalidate_all();
            self.report_cache.invalidate_all();
        }
        Ok(result)
    }

    async fn delete_stale_dependencies(&self, cutoff_iso: &str) -> Result<u64, RepositoryError> {
        let result = self.inner.delete_stale_dependencies(cutoff_iso).await?;
        if !self.is_disabled() && result > 0 {
            self.report_cache.invalidate_all();
        }
        Ok(result)
    }

    async fn get_endpoint_id(
        &self,
        branch_id: i64,
        api_type: ApiType,
        path: &str,
        method: &str,
    ) -> Result<Option<i64>, RepositoryError> {
        self.inner
            .get_endpoint_id(branch_id, api_type, path, method)
            .await
    }

    async fn insert_endpoint_version(
        &self,
        endpoint_id: i64,
        version: i32,
        yaml_content: &str,
        diff: Option<&str>,
        created_at: &str,
    ) -> Result<(), RepositoryError> {
        self.inner
            .insert_endpoint_version(endpoint_id, version, yaml_content, diff, created_at)
            .await
    }

    async fn get_latest_endpoint_version(&self, endpoint_id: i64) -> Result<i32, RepositoryError> {
        self.inner.get_latest_endpoint_version(endpoint_id).await
    }

    async fn get_endpoint_versions(
        &self,
        endpoint_id: i64,
    ) -> Result<Vec<EndpointVersion>, RepositoryError> {
        self.inner.get_endpoint_versions(endpoint_id).await
    }

    async fn get_global_endpoint_versions(
        &self,
        limit: u32,
    ) -> Result<Vec<EndpointVersion>, RepositoryError> {
        self.inner.get_global_endpoint_versions(limit).await
    }

    async fn apply_spec_changes(
        &self,
        branch_id: i64,
        changes: Vec<SpecChange>,
        is_protected: bool,
        username: Option<&str>,
        source_branch: Option<&str>,
    ) -> Result<(), RepositoryError> {
        self.inner
            .apply_spec_changes(branch_id, changes, is_protected, username, source_branch)
            .await?;
        if !self.is_disabled() {
            self.branch_endpoints_cache.invalidate(&branch_id).await;
            self.endpoint_cache.invalidate_all();
            self.report_cache.invalidate_all();
        }
        Ok(())
    }

    async fn add_service_tags(
        &self,
        service_id: i64,
        tags: &[String],
    ) -> Result<(), RepositoryError> {
        self.inner.add_service_tags(service_id, tags).await
    }

    async fn get_all_service_tags(
        &self,
    ) -> Result<std::collections::HashMap<String, Vec<String>>, RepositoryError> {
        self.inner.get_all_service_tags().await
    }

    async fn get_channel_message_contract(
        &self,
        branch_name: &str,
        channel: &str,
        message_name: &str,
    ) -> Result<Option<ChannelMessageContract>, RepositoryError> {
        self.inner
            .get_channel_message_contract(branch_name, channel, message_name)
            .await
    }

    async fn upsert_channel_message_contract(
        &self,
        contract: &ChannelMessageContract,
    ) -> Result<(), RepositoryError> {
        self.inner.upsert_channel_message_contract(contract).await
    }

    async fn delete_channel_message_contract(
        &self,
        branch_name: &str,
        channel: &str,
        message_name: &str,
    ) -> Result<(), RepositoryError> {
        self.inner
            .delete_channel_message_contract(branch_name, channel, message_name)
            .await
    }

    async fn list_channel_message_contracts(
        &self,
        branch_name: &str,
    ) -> Result<Vec<ChannelMessageContract>, RepositoryError> {
        self.inner.list_channel_message_contracts(branch_name).await
    }

    async fn delete_orphaned_channel_message_contracts(
        &self,
        live_branches: &[String],
    ) -> Result<u64, RepositoryError> {
        self.inner
            .delete_orphaned_channel_message_contracts(live_branches)
            .await
    }

    async fn insert_audit_log(
        &self,
        username: &str,
        log: NewAuditLog<'_>,
    ) -> Result<(), RepositoryError> {
        self.inner.insert_audit_log(username, log).await
    }

    async fn get_audit_logs(
        &self,
        filter: AuditLogFilter,
    ) -> Result<Vec<AuditLogEntry>, RepositoryError> {
        self.inner.get_audit_logs(filter).await
    }

    async fn get_recent_audit_logs(
        &self,
        limit: u32,
    ) -> Result<Vec<AuditLogEntry>, RepositoryError> {
        self.inner.get_recent_audit_logs(limit).await
    }

    async fn get_user_favorites(
        &self,
        user_id: i64,
        item_type: &str,
    ) -> Result<Vec<String>, RepositoryError> {
        self.inner.get_user_favorites(user_id, item_type).await
    }

    async fn add_user_favorite(
        &self,
        user_id: i64,
        item_type: &str,
        item_name: &str,
    ) -> Result<(), RepositoryError> {
        self.inner
            .add_user_favorite(user_id, item_type, item_name)
            .await
    }

    async fn remove_user_favorite(
        &self,
        user_id: i64,
        item_type: &str,
        item_name: &str,
    ) -> Result<(), RepositoryError> {
        self.inner
            .remove_user_favorite(user_id, item_type, item_name)
            .await
    }

    async fn list_branches_with_metadata(&self) -> Result<Vec<BranchMetadata>, RepositoryError> {
        self.inner.list_branches_with_metadata().await
    }

    async fn list_branch_last_published(
        &self,
    ) -> Result<Vec<(String, String, String)>, RepositoryError> {
        self.inner.list_branch_last_published().await
    }

    async fn list_branch_endpoint_counts(
        &self,
    ) -> Result<Vec<(String, String, i64)>, RepositoryError> {
        self.inner.list_branch_endpoint_counts().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infrastructure::database::DatabaseRepo;
    use crate::infrastructure::sqlite_repository::SqliteSpecRepository;
    use sqlx::sqlite::SqlitePoolOptions;

    async fn setup_cached_repo(memory_mb: u64) -> CachedSpecRepository {
        let pool = SqlitePoolOptions::new()
            .connect("sqlite::memory:")
            .await
            .unwrap();
        let repo = SqliteSpecRepository::new(pool);
        repo.run_migrations().await.unwrap();
        CachedSpecRepository::new(DatabaseRepo::Sqlite(repo), memory_mb)
    }

    #[tokio::test]
    async fn test_cache_miss_then_hit_find_service() {
        let repo = setup_cached_repo(256).await;
        let id = repo.ensure_service("test-svc").await.unwrap();
        let found = repo.find_service("test-svc").await.unwrap();
        assert_eq!(found, Some(id));
        let stats = repo.cache_stats().await;
        assert!(stats.hit_count > 0, "Expected cache hits");
    }

    #[tokio::test]
    async fn test_cache_miss_then_hit_find_branch() {
        let repo = setup_cached_repo(256).await;
        let sid = repo.ensure_service("svc").await.unwrap();
        let bid = repo.ensure_branch(sid, "main").await.unwrap();
        let found = repo.find_branch(sid, "main").await.unwrap();
        assert_eq!(found, Some(bid));
        assert!(repo.cache_stats().await.hit_count > 0);
    }

    #[tokio::test]
    async fn test_cache_invalidation_on_delete_service() {
        let repo = setup_cached_repo(256).await;
        let _id = repo.ensure_service("svc").await.unwrap();
        let found = repo.find_service("svc").await.unwrap();
        assert!(found.is_some());
        repo.delete_service("svc").await.unwrap();
        let found = repo.find_service("svc").await.unwrap();
        assert_eq!(found, None);
    }

    #[tokio::test]
    async fn test_cache_disabled_zero_mb() {
        let repo = setup_cached_repo(0).await;
        assert!(repo.is_disabled());
        let _id = repo.ensure_service("svc").await.unwrap();
        let found = repo.find_service("svc").await.unwrap();
        assert!(found.is_some());
        let stats = repo.cache_stats().await;
        assert!(!stats.enabled);
        assert_eq!(stats.entry_count, 0);
    }

    #[tokio::test]
    async fn test_cache_stats_hit_miss_counters() {
        let repo = setup_cached_repo(256).await;
        let stats = repo.cache_stats().await;
        assert_eq!(stats.hit_count, 0);
        assert_eq!(stats.miss_count, 0);

        // ensure_service populates service_id_cache
        repo.ensure_service("svc").await.unwrap();
        // find_service should be a cache hit
        repo.find_service("svc").await.unwrap();
        let stats = repo.cache_stats().await;
        assert!(stats.hit_count >= 1);
    }

    #[tokio::test]
    async fn test_rebuild_caches_resets_counters() {
        let repo = setup_cached_repo(256).await;
        repo.ensure_service("svc").await.unwrap();
        repo.find_service("svc").await.unwrap();
        assert!(repo.cache_stats().await.hit_count > 0);

        repo.rebuild_caches(256);
        let stats = repo.cache_stats().await;
        assert_eq!(stats.hit_count, 0);
        assert_eq!(stats.miss_count, 0);
    }

    #[tokio::test]
    async fn test_get_endpoints_for_branch_cached() {
        let repo = setup_cached_repo(256).await;
        let sid = repo.ensure_service("svc").await.unwrap();
        let bid = repo.ensure_branch(sid, "main").await.unwrap();

        let eps = repo.get_endpoints_for_branch(bid).await.unwrap();
        assert!(eps.is_empty());
        let miss_before = repo.cache_stats().await.miss_count;

        let eps = repo.get_endpoints_for_branch(bid).await.unwrap();
        assert!(eps.is_empty());
        assert!(repo.cache_stats().await.hit_count > 0);
        assert_eq!(repo.cache_stats().await.miss_count, miss_before);
    }

    #[tokio::test]
    async fn test_protected_branch_cache_invalidation() {
        let repo = setup_cached_repo(256).await;

        // is_branch_protected uses glob matching in sqlite, not exact match
        // Add protection first, then check
        repo.add_protected_branch("main").await.unwrap();
        let protected = repo.is_branch_protected("main").await.unwrap();
        assert!(protected);

        repo.remove_protected_branch("main").await.unwrap();
        let protected = repo.is_branch_protected("main").await.unwrap();
        assert!(!protected);
    }

    #[tokio::test]
    async fn test_list_services_cached() {
        let repo = setup_cached_repo(256).await;
        let sid_a = repo.ensure_service("svc-a").await.unwrap();
        repo.ensure_branch(sid_a, "main").await.unwrap();
        let sid_b = repo.ensure_service("svc-b").await.unwrap();
        repo.ensure_branch(sid_b, "main").await.unwrap();

        // First call populates cache
        let services = repo.list_services().await.unwrap();
        assert_eq!(services.len(), 2);
        let miss_count = repo.cache_stats().await.miss_count;

        // Second call should hit cache
        let services = repo.list_services().await.unwrap();
        assert_eq!(services.len(), 2);
        assert_eq!(repo.cache_stats().await.miss_count, miss_count);
    }

    #[tokio::test]
    async fn test_fallback_branch_cache() {
        let repo = setup_cached_repo(256).await;
        repo.ensure_service("svc").await.unwrap();

        let fb = repo.get_fallback_branch("svc").await.unwrap();
        assert_eq!(fb, None);

        repo.set_fallback_branch("svc", Some("main")).await.unwrap();
        let fb = repo.get_fallback_branch("svc").await.unwrap();
        assert_eq!(fb, Some("main".to_string()));
    }
}
