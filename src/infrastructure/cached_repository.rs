use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use moka::future::Cache;

use crate::domain::models::*;
use crate::domain::ports::{
    EndpointMap, NewAuditLog, RecordDependencyParams, RepositoryError, SpecRepository,
    UpsertSpecVersion,
};
use crate::infrastructure::database::DatabaseRepo;

/// (spec_version_id, api_type, normalized_path, method)
type EndpointKey = (i64, ApiType, String, String);
/// (service_id, api_type, version-string)
type SpecVersionKey = (i64, ApiType, String);

#[derive(Clone)]
pub struct CachedSpecRepository {
    inner: Box<DatabaseRepo>,
    // Spec data (60% of budget)
    endpoint_cache: Cache<EndpointKey, Arc<(i64, String, bool)>>,
    version_endpoints_cache: Cache<i64, Arc<Vec<EndpointRecord>>>,
    spec_content_cache: Cache<i64, Arc<String>>,
    // Reports (20%)
    report_cache: Cache<String, Arc<DependencyReport>>,
    services_cache: Cache<String, Arc<Vec<ProducerSummary>>>,
    // ID / metadata lookups (10%)
    service_id_cache: Cache<String, i64>,
    spec_version_cache: Cache<SpecVersionKey, Arc<SpecVersionMeta>>,
    // Listings (10%)
    services_list_cache: Cache<String, Arc<Vec<String>>>,
    clients_list_cache: Cache<String, Arc<Vec<String>>>,
    // Authorisation (entry-count bounded, short TTL)
    effective_roles_cache: Cache<i64, Arc<Vec<String>>>,
    // Stats
    hits: Arc<AtomicU64>,
    misses: Arc<AtomicU64>,
    memory_limit_mb: Arc<AtomicU64>,
}

fn mb_to_bytes(mb: u64) -> u64 {
    mb * 1024 * 1024
}

/// Safety net for the effective-roles cache: writes that bypass this
/// repository (direct SQL) take effect within this window. Writes *through*
/// the repository invalidate immediately and never wait on it.
const EFFECTIVE_ROLES_TTL_SECS: u64 = 10;

struct RepoCaches {
    endpoint_cache: Cache<EndpointKey, Arc<(i64, String, bool)>>,
    version_endpoints_cache: Cache<i64, Arc<Vec<EndpointRecord>>>,
    spec_content_cache: Cache<i64, Arc<String>>,
    report_cache: Cache<String, Arc<DependencyReport>>,
    services_cache: Cache<String, Arc<Vec<ProducerSummary>>>,
    service_id_cache: Cache<String, i64>,
    spec_version_cache: Cache<SpecVersionKey, Arc<SpecVersionMeta>>,
    services_list_cache: Cache<String, Arc<Vec<String>>>,
    clients_list_cache: Cache<String, Arc<Vec<String>>>,
    effective_roles_cache: Cache<i64, Arc<Vec<String>>>,
}

impl CachedSpecRepository {
    pub fn new(inner: DatabaseRepo, memory_limit_mb: u64) -> Self {
        let caches = Self::build_caches(memory_limit_mb);
        Self {
            inner: Box::new(inner),
            endpoint_cache: caches.endpoint_cache,
            version_endpoints_cache: caches.version_endpoints_cache,
            spec_content_cache: caches.spec_content_cache,
            report_cache: caches.report_cache,
            services_cache: caches.services_cache,
            service_id_cache: caches.service_id_cache,
            spec_version_cache: caches.spec_version_cache,
            services_list_cache: caches.services_list_cache,
            clients_list_cache: caches.clients_list_cache,
            effective_roles_cache: caches.effective_roles_cache,
            hits: Arc::new(AtomicU64::new(0)),
            misses: Arc::new(AtomicU64::new(0)),
            memory_limit_mb: Arc::new(AtomicU64::new(memory_limit_mb)),
        }
    }

    fn build_caches(memory_limit_mb: u64) -> RepoCaches {
        let total = mb_to_bytes(memory_limit_mb);
        let spec_budget = total * 60 / 100;
        let report_budget = total * 20 / 100;
        let id_budget = total * 10 / 100;
        let list_budget = total * 10 / 100;

        // Spec data caches (60%)
        let endpoint_cache = Cache::builder()
            .max_capacity(spec_budget / 3)
            .weigher(|_k: &EndpointKey, v: &Arc<(i64, String, bool)>| (v.1.len() + 80) as u32)
            .build();

        let version_endpoints_cache = Cache::builder()
            .max_capacity(spec_budget / 3)
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

        let spec_content_cache = Cache::builder()
            .max_capacity(spec_budget / 3)
            .weigher(|_k: &i64, v: &Arc<String>| (v.len() + 40) as u32)
            .build();

        // Report caches (20%)
        let report_cache = Cache::builder()
            .max_capacity(report_budget / 2)
            .weigher(|_k: &String, v: &Arc<DependencyReport>| {
                let size = v.unused_endpoints.len() * 120
                    + v.missing_endpoints.len() * 150
                    + v.dependency_graph.len() * 150
                    + v.service_tags.len() * 60
                    + 80;
                size as u32
            })
            .build();

        let services_cache = Cache::builder()
            .max_capacity(report_budget / 2)
            .weigher(|_k: &String, v: &Arc<Vec<ProducerSummary>>| {
                let size: usize = v
                    .iter()
                    .map(|s| {
                        s.name.len()
                            + s.versions.len() * 160
                            + s.icon.as_ref().map_or(0, |i| i.len())
                            + s.domain.as_ref().map_or(0, |d| d.len())
                            + 100
                    })
                    .sum();
                (size + 24) as u32
            })
            .build();

        // ID / metadata lookup caches (10%)
        let service_id_cache = Cache::builder()
            .max_capacity(id_budget / 2)
            .weigher(|k: &String, _v: &i64| (k.len() + 32) as u32)
            .build();

        let spec_version_cache = Cache::builder()
            .max_capacity(id_budget / 2)
            .weigher(|k: &SpecVersionKey, v: &Arc<SpecVersionMeta>| {
                (k.2.len() + v.content_hash.len() + v.provided_by.len() + 160) as u32
            })
            .build();

        // Listing caches (10%)
        let services_list_cache = Cache::builder()
            .max_capacity(list_budget / 2)
            .weigher(|_k: &String, v: &Arc<Vec<String>>| {
                let size: usize = v.iter().map(|s| s.len() + 24).sum();
                (size + 24) as u32
            })
            .build();

        let clients_list_cache = Cache::builder()
            .max_capacity(list_budget / 2)
            .weigher(|_k: &String, v: &Arc<Vec<String>>| {
                let size: usize = v.iter().map(|s| s.len() + 24).sum();
                (size + 24) as u32
            })
            .build();

        // Stored roles are consulted on every authenticated request, which made
        // them the one uncached query on the hot path. Every grant, revocation
        // and membership change through this repository invalidates explicitly,
        // so a revocation still bites immediately; the TTL only bounds how long
        // an edit made directly against the database can go unnoticed.
        let effective_roles_cache = Cache::builder()
            .max_capacity(10_000)
            .time_to_live(std::time::Duration::from_secs(EFFECTIVE_ROLES_TTL_SECS))
            .build();

        RepoCaches {
            endpoint_cache,
            version_endpoints_cache,
            spec_content_cache,
            report_cache,
            services_cache,
            service_id_cache,
            spec_version_cache,
            services_list_cache,
            clients_list_cache,
            effective_roles_cache,
        }
    }

    pub async fn cache_stats(&self) -> CacheStats {
        // Run maintenance tasks to get accurate stats
        self.endpoint_cache.run_pending_tasks().await;
        self.version_endpoints_cache.run_pending_tasks().await;
        self.spec_content_cache.run_pending_tasks().await;
        self.report_cache.run_pending_tasks().await;
        self.services_cache.run_pending_tasks().await;
        self.service_id_cache.run_pending_tasks().await;
        self.spec_version_cache.run_pending_tasks().await;
        self.services_list_cache.run_pending_tasks().await;
        self.clients_list_cache.run_pending_tasks().await;
        self.effective_roles_cache.run_pending_tasks().await;

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
            + self.version_endpoints_cache.weighted_size()
            + self.spec_content_cache.weighted_size()
            + self.report_cache.weighted_size()
            + self.services_cache.weighted_size()
            + self.service_id_cache.weighted_size()
            + self.spec_version_cache.weighted_size()
            + self.services_list_cache.weighted_size()
            + self.clients_list_cache.weighted_size();

        let entry_count = self.endpoint_cache.entry_count()
            + self.version_endpoints_cache.entry_count()
            + self.spec_content_cache.entry_count()
            + self.report_cache.entry_count()
            + self.services_cache.entry_count()
            + self.service_id_cache.entry_count()
            + self.spec_version_cache.entry_count()
            + self.services_list_cache.entry_count()
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
        self.version_endpoints_cache.invalidate_all();
        self.spec_content_cache.invalidate_all();
        self.report_cache.invalidate_all();
        self.services_cache.invalidate_all();
        self.service_id_cache.invalidate_all();
        self.spec_version_cache.invalidate_all();
        self.services_list_cache.invalidate_all();
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

    /// Clear everything derived from stored spec versions. Moka offers no
    /// partial-key invalidation, so a write to any version-line entry clears
    /// these caches wholesale — correctness over cleverness.
    fn invalidate_spec_caches(&self) {
        self.spec_version_cache.invalidate_all();
        self.version_endpoints_cache.invalidate_all();
        self.endpoint_cache.invalidate_all();
        self.spec_content_cache.invalidate_all();
        self.report_cache.invalidate_all();
    }

    fn invalidate_all_caches(&self) {
        self.invalidate_spec_caches();
        self.service_id_cache.invalidate_all();
        self.services_list_cache.invalidate_all();
        self.services_cache.invalidate_all();
        self.clients_list_cache.invalidate_all();
        self.effective_roles_cache.invalidate_all();
    }
}

impl SpecRepository for CachedSpecRepository {
    // --- Spec versions: cached reads with write-through invalidation ---

    async fn ping(&self) -> Result<(), RepositoryError> {
        self.inner.ping().await
    }

    async fn upsert_spec_version(
        &self,
        params: UpsertSpecVersion<'_>,
    ) -> Result<i64, RepositoryError> {
        let result = self.inner.upsert_spec_version(params).await;
        if !self.is_disabled() {
            // Invalidate on failure too: a Conflict means another replica moved
            // the row past what this process has cached, and the application
            // layer re-reads to diagnose the refusal — that re-read must see
            // the row as the database refused it, not the stale cached state
            // that made the write look viable.
            self.invalidate_spec_caches();
        }
        result
    }

    async fn find_spec_version(
        &self,
        service_id: i64,
        api_type: ApiType,
        version: SemVer,
    ) -> Result<Option<SpecVersionMeta>, RepositoryError> {
        let key = (service_id, api_type, version.to_string());
        if !self.is_disabled()
            && let Some(cached) = self.spec_version_cache.get(&key).await
        {
            self.record_hit();
            return Ok(Some((*cached).clone()));
        }
        self.record_miss();
        let result = self
            .inner
            .find_spec_version(service_id, api_type, version)
            .await?;
        if !self.is_disabled()
            && let Some(ref meta) = result
        {
            self.spec_version_cache
                .insert(key, Arc::new(meta.clone()))
                .await;
        }
        Ok(result)
    }

    async fn list_spec_versions(
        &self,
        service_id: i64,
    ) -> Result<Vec<SpecVersionMeta>, RepositoryError> {
        self.inner.list_spec_versions(service_id).await
    }

    async fn list_all_spec_versions(
        &self,
    ) -> Result<Vec<(String, SpecVersionMeta, i64)>, RepositoryError> {
        self.inner.list_all_spec_versions().await
    }

    async fn get_spec_content(
        &self,
        spec_version_id: i64,
    ) -> Result<Option<String>, RepositoryError> {
        if !self.is_disabled()
            && let Some(cached) = self.spec_content_cache.get(&spec_version_id).await
        {
            self.record_hit();
            return Ok(Some((*cached).clone()));
        }
        self.record_miss();
        let result = self.inner.get_spec_content(spec_version_id).await?;
        if !self.is_disabled()
            && let Some(ref content) = result
        {
            self.spec_content_cache
                .insert(spec_version_id, Arc::new(content.clone()))
                .await;
        }
        Ok(result)
    }

    async fn delete_spec_version(&self, spec_version_id: i64) -> Result<bool, RepositoryError> {
        let existed = self.inner.delete_spec_version(spec_version_id).await?;
        if existed && !self.is_disabled() {
            self.invalidate_spec_caches();
        }
        Ok(existed)
    }

    async fn touch_spec_version_required(
        &self,
        spec_version_id: i64,
        now_iso: &str,
    ) -> Result<(), RepositoryError> {
        self.inner
            .touch_spec_version_required(spec_version_id, now_iso)
            .await?;
        // `find_spec_version` results carry `last_required_at`, and only the
        // row id is known here — not the (service, api_type, version) key — so
        // the whole meta cache goes rather than guessing the entry.
        if !self.is_disabled() {
            self.spec_version_cache.invalidate_all();
        }
        Ok(())
    }

    async fn touch_spec_version_provided(
        &self,
        spec_version_id: i64,
        now_iso: &str,
    ) -> Result<(), RepositoryError> {
        self.inner
            .touch_spec_version_provided(spec_version_id, now_iso)
            .await?;
        // Same shape as `touch_spec_version_required`: cached metas carry
        // `updated_at`, and only the row id is known here.
        if !self.is_disabled() {
            self.spec_version_cache.invalidate_all();
        }
        Ok(())
    }

    async fn delete_expired_snapshots(&self, cutoff_iso: &str) -> Result<u64, RepositoryError> {
        let result = self.inner.delete_expired_snapshots(cutoff_iso).await?;
        if result > 0 && !self.is_disabled() {
            self.invalidate_spec_caches();
        }
        Ok(result)
    }

    async fn list_version_dependents(
        &self,
        spec_version_id: i64,
    ) -> Result<Vec<String>, RepositoryError> {
        self.inner.list_version_dependents(spec_version_id).await
    }

    // --- Services and clients ---

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

    async fn get_endpoints_for_version(
        &self,
        spec_version_id: i64,
    ) -> Result<Vec<EndpointRecord>, RepositoryError> {
        if !self.is_disabled()
            && let Some(cached) = self.version_endpoints_cache.get(&spec_version_id).await
        {
            self.record_hit();
            return Ok((*cached).clone());
        }
        self.record_miss();
        let result = self
            .inner
            .get_endpoints_for_version(spec_version_id)
            .await?;
        if !self.is_disabled() {
            self.version_endpoints_cache
                .insert(spec_version_id, Arc::new(result.clone()))
                .await;
        }
        Ok(result)
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
        spec_version_id: i64,
        api_type: ApiType,
        path: &str,
        method: &str,
    ) -> Result<Option<(i64, String, bool)>, RepositoryError> {
        // Keyed by the normalized path, so lenient path variants of the same
        // endpoint share one entry — mirroring how the repository matches.
        let key = (
            spec_version_id,
            api_type,
            crate::openapi::normalize_path(path),
            method.to_string(),
        );
        if !self.is_disabled()
            && let Some(cached) = self.endpoint_cache.get(&key).await
        {
            self.record_hit();
            return Ok(Some((cached.0, cached.1.clone(), cached.2)));
        }
        self.record_miss();
        let result = self
            .inner
            .find_endpoint(spec_version_id, api_type, path, method)
            .await?;
        if !self.is_disabled()
            && let Some(ref val) = result
        {
            self.endpoint_cache.insert(key, Arc::new(val.clone())).await;
        }
        Ok(result)
    }

    async fn find_endpoints_bulk(
        &self,
        spec_version_id: i64,
        api_type: ApiType,
        endpoints: &[(String, String)],
    ) -> Result<EndpointMap, RepositoryError> {
        if self.is_disabled() {
            return self
                .inner
                .find_endpoints_bulk(spec_version_id, api_type, endpoints)
                .await;
        }
        // Try cache first for each endpoint
        let mut result = EndpointMap::new();
        let mut misses = Vec::new();
        for (path, method) in endpoints {
            let key = (
                spec_version_id,
                api_type,
                crate::openapi::normalize_path(path),
                method.clone(),
            );
            if let Some(cached) = self.endpoint_cache.get(&key).await {
                self.record_hit();
                result.insert(
                    (path.clone(), method.clone()),
                    (cached.0, cached.1.clone(), cached.2),
                );
            } else {
                misses.push((path.clone(), method.clone()));
            }
        }
        if !misses.is_empty() {
            self.record_miss();
            let db_results = self
                .inner
                .find_endpoints_bulk(spec_version_id, api_type, &misses)
                .await?;
            for ((path, method), val) in &db_results {
                let key = (
                    spec_version_id,
                    api_type,
                    crate::openapi::normalize_path(path),
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

    async fn get_report(&self) -> Result<DependencyReport, RepositoryError> {
        let sentinel = "_all_".to_string();
        if !self.is_disabled()
            && let Some(cached) = self.report_cache.get(&sentinel).await
        {
            self.record_hit();
            return Ok((*cached).clone());
        }
        self.record_miss();
        let result = self.inner.get_report().await?;
        if !self.is_disabled() {
            self.report_cache
                .insert(sentinel, Arc::new(result.clone()))
                .await;
        }
        Ok(result)
    }

    async fn delete_all_services(&self) -> Result<u64, RepositoryError> {
        let result = self.inner.delete_all_services().await?;
        if !self.is_disabled() {
            self.invalidate_all_caches();
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

    async fn delete_all_non_admin_users(
        &self,
        spare_usernames: &[String],
    ) -> Result<u64, RepositoryError> {
        self.inner.delete_all_non_admin_users(spare_usernames).await
    }

    async fn nuke_database(&self, keep_user_id: Option<i64>) -> Result<(), RepositoryError> {
        self.inner.nuke_database(keep_user_id).await?;
        if !self.is_disabled() {
            self.invalidate_all_caches();
        }
        Ok(())
    }

    async fn delete_producer(&self, name: &str) -> Result<bool, RepositoryError> {
        let result = self.inner.delete_producer(name).await?;
        if !self.is_disabled() {
            self.service_id_cache.invalidate(&name.to_string()).await;
            self.invalidate_spec_caches();
            self.services_list_cache.invalidate_all();
            self.services_cache.invalidate_all();
        }
        Ok(result)
    }

    async fn delete_consumer(&self, name: &str) -> Result<bool, RepositoryError> {
        let result = self.inner.delete_consumer(name).await?;
        if !self.is_disabled() {
            self.clients_list_cache.invalidate_all();
            self.report_cache.invalidate_all();
        }
        Ok(result)
    }

    async fn list_producers(&self) -> Result<Vec<String>, RepositoryError> {
        let sentinel = "_all_".to_string();
        if !self.is_disabled()
            && let Some(cached) = self.services_list_cache.get(&sentinel).await
        {
            self.record_hit();
            return Ok((*cached).clone());
        }
        self.record_miss();
        let result = self.inner.list_producers().await?;
        if !self.is_disabled() {
            self.services_list_cache
                .insert(sentinel, Arc::new(result.clone()))
                .await;
        }
        Ok(result)
    }

    async fn list_producers_detailed(&self) -> Result<Vec<ProducerSummary>, RepositoryError> {
        let sentinel = "_all_".to_string();
        if !self.is_disabled()
            && let Some(cached) = self.services_cache.get(&sentinel).await
        {
            self.record_hit();
            return Ok((*cached).clone());
        }
        self.record_miss();
        let result = self.inner.list_producers_detailed().await?;
        if !self.is_disabled() {
            self.services_cache
                .insert(sentinel, Arc::new(result.clone()))
                .await;
        }
        Ok(result)
    }

    async fn update_producer_metadata(
        &self,
        service_name: &str,
        icon: Option<&str>,
        domain: Option<&str>,
    ) -> Result<(), RepositoryError> {
        self.inner
            .update_producer_metadata(service_name, icon, domain)
            .await?;
        if !self.is_disabled() {
            self.services_cache.invalidate_all();
        }
        Ok(())
    }

    async fn list_consumers(&self) -> Result<Vec<String>, RepositoryError> {
        let sentinel = "_all_".to_string();
        if !self.is_disabled()
            && let Some(cached) = self.clients_list_cache.get(&sentinel).await
        {
            self.record_hit();
            return Ok((*cached).clone());
        }
        self.record_miss();
        let result = self.inner.list_consumers().await?;
        if !self.is_disabled() {
            self.clients_list_cache
                .insert(sentinel, Arc::new(result.clone()))
                .await;
        }
        Ok(result)
    }

    async fn list_consumer_endpoints(
        &self,
        client_name: &str,
    ) -> Result<Vec<ConsumerEndpointInfo>, RepositoryError> {
        self.inner.list_consumer_endpoints(client_name).await
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
        approved: bool,
    ) -> Result<User, RepositoryError> {
        self.inner
            .create_user(username, password_hash, approved)
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
    ) -> Result<bool, RepositoryError> {
        self.inner.delete_api_token(token_id, user_id).await
    }

    async fn validate_api_token(&self, token_hash: &str) -> Result<Option<User>, RepositoryError> {
        self.inner.validate_api_token(token_hash).await
    }

    async fn delete_stale_dependencies(&self, cutoff_iso: &str) -> Result<u64, RepositoryError> {
        let result = self.inner.delete_stale_dependencies(cutoff_iso).await?;
        if result > 0 && !self.is_disabled() {
            self.report_cache.invalidate_all();
        }
        Ok(result)
    }

    // --- Roles and Groups ---
    //
    // `effective_stored_roles` runs on every authenticated request, so it is
    // cached — with every write below invalidating explicitly, which keeps
    // revocation immediate. The management reads (listing roles, groups,
    // members) stay uncached: they render admin pages, not the hot path.

    async fn grant_user_role(&self, user_id: i64, role: &str) -> Result<(), RepositoryError> {
        self.inner.grant_user_role(user_id, role).await?;
        self.effective_roles_cache.invalidate(&user_id).await;
        Ok(())
    }

    async fn revoke_user_role(&self, user_id: i64, role: &str) -> Result<bool, RepositoryError> {
        let removed = self.inner.revoke_user_role(user_id, role).await?;
        self.effective_roles_cache.invalidate(&user_id).await;
        Ok(removed)
    }

    async fn list_user_roles(&self, user_id: i64) -> Result<Vec<String>, RepositoryError> {
        self.inner.list_user_roles(user_id).await
    }

    async fn effective_stored_roles(&self, user_id: i64) -> Result<Vec<String>, RepositoryError> {
        if !self.is_disabled()
            && let Some(roles) = self.effective_roles_cache.get(&user_id).await
        {
            self.record_hit();
            return Ok(roles.as_ref().clone());
        }
        self.record_miss();
        let roles = self.inner.effective_stored_roles(user_id).await?;
        if !self.is_disabled() {
            self.effective_roles_cache
                .insert(user_id, Arc::new(roles.clone()))
                .await;
        }
        Ok(roles)
    }

    async fn create_group(
        &self,
        name: &str,
        source: GroupSource,
    ) -> Result<Group, RepositoryError> {
        self.inner.create_group(name, source).await
    }

    async fn rename_group(&self, group_id: i64, name: &str) -> Result<bool, RepositoryError> {
        self.inner.rename_group(group_id, name).await
    }

    async fn delete_group(&self, group_id: i64) -> Result<bool, RepositoryError> {
        let removed = self.inner.delete_group(group_id).await?;
        // Which users held roles through this group is not tracked here, so
        // everyone's entry goes rather than guessing.
        self.effective_roles_cache.invalidate_all();
        Ok(removed)
    }

    async fn list_groups(&self) -> Result<Vec<Group>, RepositoryError> {
        self.inner.list_groups().await
    }

    async fn set_group_roles(
        &self,
        group_id: i64,
        roles: &[String],
    ) -> Result<(), RepositoryError> {
        self.inner.set_group_roles(group_id, roles).await?;
        // Affects every member of the group; membership is not tracked here.
        self.effective_roles_cache.invalidate_all();
        Ok(())
    }

    async fn list_group_roles(&self, group_id: i64) -> Result<Vec<String>, RepositoryError> {
        self.inner.list_group_roles(group_id).await
    }

    async fn add_group_member(&self, group_id: i64, user_id: i64) -> Result<(), RepositoryError> {
        self.inner.add_group_member(group_id, user_id).await?;
        self.effective_roles_cache.invalidate(&user_id).await;
        Ok(())
    }

    async fn remove_group_member(
        &self,
        group_id: i64,
        user_id: i64,
    ) -> Result<bool, RepositoryError> {
        let removed = self.inner.remove_group_member(group_id, user_id).await?;
        self.effective_roles_cache.invalidate(&user_id).await;
        Ok(removed)
    }

    async fn list_group_member_ids(&self, group_id: i64) -> Result<Vec<i64>, RepositoryError> {
        self.inner.list_group_member_ids(group_id).await
    }

    // --- Maintainer Scope ---
    //
    // Deliberately uncached, for the same reason as roles: an unassignment must
    // take effect at once, not when a cache entry happens to expire.

    async fn add_user_maintainer(
        &self,
        service_id: i64,
        user_id: i64,
    ) -> Result<(), RepositoryError> {
        self.inner.add_user_maintainer(service_id, user_id).await
    }

    async fn remove_user_maintainer(
        &self,
        service_id: i64,
        user_id: i64,
    ) -> Result<bool, RepositoryError> {
        self.inner.remove_user_maintainer(service_id, user_id).await
    }

    async fn add_group_maintainer(
        &self,
        service_id: i64,
        group_id: i64,
    ) -> Result<(), RepositoryError> {
        self.inner.add_group_maintainer(service_id, group_id).await
    }

    async fn remove_group_maintainer(
        &self,
        service_id: i64,
        group_id: i64,
    ) -> Result<bool, RepositoryError> {
        self.inner
            .remove_group_maintainer(service_id, group_id)
            .await
    }

    async fn list_user_maintainer_ids(&self, service_id: i64) -> Result<Vec<i64>, RepositoryError> {
        self.inner.list_user_maintainer_ids(service_id).await
    }

    async fn list_group_maintainer_ids(
        &self,
        service_id: i64,
    ) -> Result<Vec<i64>, RepositoryError> {
        self.inner.list_group_maintainer_ids(service_id).await
    }

    async fn list_all_user_maintainers(&self) -> Result<Vec<(String, i64)>, RepositoryError> {
        self.inner.list_all_user_maintainers().await
    }

    async fn list_all_group_maintainers(&self) -> Result<Vec<(String, i64)>, RepositoryError> {
        self.inner.list_all_group_maintainers().await
    }

    async fn list_group_maintained_producers(
        &self,
        group_ids: &[i64],
    ) -> Result<Vec<String>, RepositoryError> {
        self.inner.list_group_maintained_producers(group_ids).await
    }

    async fn maintains_producer(
        &self,
        user_id: i64,
        service_id: i64,
    ) -> Result<bool, RepositoryError> {
        self.inner.maintains_producer(user_id, service_id).await
    }

    async fn list_maintained_producers(
        &self,
        user_id: i64,
    ) -> Result<Vec<String>, RepositoryError> {
        self.inner.list_maintained_producers(user_id).await
    }

    // --- Service Tags ---

    async fn add_service_tags(
        &self,
        service_id: i64,
        tags: &[String],
    ) -> Result<(), RepositoryError> {
        self.inner.add_service_tags(service_id, tags).await?;
        // Tags ride along inside the dependency report.
        if !self.is_disabled() {
            self.report_cache.invalidate_all();
        }
        Ok(())
    }

    async fn get_all_service_tags(
        &self,
    ) -> Result<std::collections::HashMap<String, Vec<String>>, RepositoryError> {
        self.inner.get_all_service_tags().await
    }

    // --- Audit Logs ---

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

    // --- User Favorites ---

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

    // --- AsyncAPI Channel Message Contracts ---

    async fn get_channel_message_contract(
        &self,
        channel: &str,
        message_name: &str,
    ) -> Result<Option<ChannelMessageContract>, RepositoryError> {
        self.inner
            .get_channel_message_contract(channel, message_name)
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
        channel: &str,
        message_name: &str,
    ) -> Result<(), RepositoryError> {
        self.inner
            .delete_channel_message_contract(channel, message_name)
            .await
    }

    async fn list_channel_message_contracts(
        &self,
    ) -> Result<Vec<ChannelMessageContract>, RepositoryError> {
        self.inner.list_channel_message_contracts().await
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

    fn endpoint(path: &str, method: &str, yaml: &str) -> EndpointRecord {
        EndpointRecord {
            id: None,
            api_type: ApiType::OpenApi,
            path: path.to_string(),
            normalized_path: crate::openapi::normalize_path(path),
            method: method.to_string(),
            yaml_content: yaml.to_string(),
            deprecated: false,
        }
    }

    async fn provide(
        repo: &CachedSpecRepository,
        service_id: i64,
        version: &str,
        content_hash: &str,
        endpoints: Vec<EndpointRecord>,
    ) -> i64 {
        repo.upsert_spec_version(UpsertSpecVersion {
            service_id,
            api_type: ApiType::OpenApi,
            version: version.parse().unwrap(),
            stability: Stability::Snapshot,
            content: "spec-content",
            content_hash,
            provided_by: "tester",
            expected_prior_hash: None,
            now_iso: "2026-01-01T00:00:00Z",
            endpoints,
        })
        .await
        .unwrap()
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
    async fn test_cache_invalidation_on_delete_producer() {
        let repo = setup_cached_repo(256).await;
        let _id = repo.ensure_service("svc").await.unwrap();
        let found = repo.find_service("svc").await.unwrap();
        assert!(found.is_some());
        repo.delete_producer("svc").await.unwrap();
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
    async fn find_spec_version_cached_and_invalidated_on_upsert() {
        let repo = setup_cached_repo(256).await;
        let sid = repo.ensure_service("svc").await.unwrap();
        let version: SemVer = "1.0.0".parse().unwrap();
        provide(&repo, sid, "1.0.0", "hash-1", vec![]).await;

        // First read populates the cache, the second is served from it.
        let meta = repo
            .find_spec_version(sid, ApiType::OpenApi, version)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(meta.content_hash, "hash-1");
        let misses_before = repo.cache_stats().await.miss_count;
        let meta = repo
            .find_spec_version(sid, ApiType::OpenApi, version)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(meta.content_hash, "hash-1");
        assert_eq!(
            repo.cache_stats().await.miss_count,
            misses_before,
            "the second read must come from the cache"
        );

        // A snapshot overwrite must be visible immediately.
        provide(&repo, sid, "1.0.0", "hash-2", vec![]).await;
        let meta = repo
            .find_spec_version(sid, ApiType::OpenApi, version)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(meta.content_hash, "hash-2");
    }

    #[tokio::test]
    async fn get_endpoints_for_version_cached_and_replaced_on_upsert() {
        let repo = setup_cached_repo(256).await;
        let sid = repo.ensure_service("svc").await.unwrap();
        let vid = provide(
            &repo,
            sid,
            "1.0.0",
            "hash-1",
            vec![endpoint("/users/{id}", "GET", "yaml-1")],
        )
        .await;

        let eps = repo.get_endpoints_for_version(vid).await.unwrap();
        assert_eq!(eps.len(), 1);
        assert_eq!(eps[0].yaml_content, "yaml-1");
        let misses_before = repo.cache_stats().await.miss_count;
        let eps = repo.get_endpoints_for_version(vid).await.unwrap();
        assert_eq!(eps.len(), 1);
        assert_eq!(repo.cache_stats().await.miss_count, misses_before);

        // Overwriting the version replaces the endpoint set wholesale.
        let vid_again = provide(
            &repo,
            sid,
            "1.0.0",
            "hash-2",
            vec![
                endpoint("/users/{id}", "GET", "yaml-2"),
                endpoint("/orders", "POST", "yaml-3"),
            ],
        )
        .await;
        assert_eq!(vid_again, vid, "overwrite keeps the row's identity");
        let eps = repo.get_endpoints_for_version(vid).await.unwrap();
        assert_eq!(eps.len(), 2);
    }

    #[tokio::test]
    async fn find_endpoint_cached_across_lenient_path_variants() {
        let repo = setup_cached_repo(256).await;
        let sid = repo.ensure_service("svc").await.unwrap();
        let vid = provide(
            &repo,
            sid,
            "1.0.0",
            "hash-1",
            vec![endpoint("/users/{id}", "GET", "yaml-1")],
        )
        .await;

        let found = repo
            .find_endpoint(vid, ApiType::OpenApi, "/users/{userId}", "GET")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(found.1, "yaml-1");

        // A differently named path parameter shares the normalized cache key.
        let misses_before = repo.cache_stats().await.miss_count;
        let found = repo
            .find_endpoint(vid, ApiType::OpenApi, "/users/{anything}", "GET")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(found.1, "yaml-1");
        assert_eq!(
            repo.cache_stats().await.miss_count,
            misses_before,
            "lenient path variants must share one cache entry"
        );

        // Removing the endpoint via an overwrite must be visible immediately.
        provide(&repo, sid, "1.0.0", "hash-2", vec![]).await;
        let found = repo
            .find_endpoint(vid, ApiType::OpenApi, "/users/{id}", "GET")
            .await
            .unwrap();
        assert_eq!(found, None);
    }

    #[tokio::test]
    async fn get_spec_content_cached_and_gone_after_delete() {
        let repo = setup_cached_repo(256).await;
        let sid = repo.ensure_service("svc").await.unwrap();
        let vid = provide(&repo, sid, "1.0.0", "hash-1", vec![]).await;

        let content = repo.get_spec_content(vid).await.unwrap();
        assert_eq!(content.as_deref(), Some("spec-content"));
        let misses_before = repo.cache_stats().await.miss_count;
        let content = repo.get_spec_content(vid).await.unwrap();
        assert_eq!(content.as_deref(), Some("spec-content"));
        assert_eq!(repo.cache_stats().await.miss_count, misses_before);

        assert!(repo.delete_spec_version(vid).await.unwrap());
        let content = repo.get_spec_content(vid).await.unwrap();
        assert_eq!(content, None);
    }

    #[tokio::test]
    async fn touch_spec_version_required_invalidates_cached_meta() {
        let repo = setup_cached_repo(256).await;
        let sid = repo.ensure_service("svc").await.unwrap();
        let version: SemVer = "1.0.0".parse().unwrap();
        let vid = provide(&repo, sid, "1.0.0", "hash-1", vec![]).await;

        // Warm the cache with a never-required entry.
        let meta = repo
            .find_spec_version(sid, ApiType::OpenApi, version)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(meta.last_required_at, None);

        repo.touch_spec_version_required(vid, "2026-02-02T00:00:00Z")
            .await
            .unwrap();
        let meta = repo
            .find_spec_version(sid, ApiType::OpenApi, version)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            meta.last_required_at.as_deref(),
            Some("2026-02-02T00:00:00Z"),
            "a require must be visible despite the warm cache"
        );
    }

    #[tokio::test]
    async fn test_list_producers_cached() {
        let repo = setup_cached_repo(256).await;
        repo.ensure_service("svc-a").await.unwrap();
        repo.ensure_service("svc-b").await.unwrap();

        // First call populates cache
        let services = repo.list_producers().await.unwrap();
        assert_eq!(services.len(), 2);
        let miss_count = repo.cache_stats().await.miss_count;

        // Second call should hit cache
        let services = repo.list_producers().await.unwrap();
        assert_eq!(services.len(), 2);
        assert_eq!(repo.cache_stats().await.miss_count, miss_count);
    }

    /// The cache must never delay a revocation: every write path invalidates,
    /// so the answer after a change is correct immediately, not after the TTL.
    #[tokio::test]
    async fn effective_roles_cache_reflects_writes_immediately() {
        let repo = setup_cached_repo(256).await;
        let user = repo.create_user("alice", "hash", true).await.unwrap();

        // Warm the cache, then prove a second read is served from it.
        repo.grant_user_role(user.id, "viewer").await.unwrap();
        assert_eq!(
            repo.effective_stored_roles(user.id).await.unwrap(),
            vec!["viewer".to_string()]
        );
        let misses_before = repo.cache_stats().await.miss_count;
        assert_eq!(
            repo.effective_stored_roles(user.id).await.unwrap(),
            vec!["viewer".to_string()]
        );
        assert_eq!(
            repo.cache_stats().await.miss_count,
            misses_before,
            "the second read must come from the cache"
        );

        // Revocation bites immediately despite the warm cache.
        repo.revoke_user_role(user.id, "viewer").await.unwrap();
        assert!(
            repo.effective_stored_roles(user.id)
                .await
                .unwrap()
                .is_empty(),
            "a revoked role must disappear at once, not after the TTL"
        );
    }

    #[tokio::test]
    async fn effective_roles_cache_tracks_group_membership() {
        let repo = setup_cached_repo(256).await;
        let user = repo.create_user("bob", "hash", true).await.unwrap();
        let group = repo
            .create_group("platform", crate::domain::models::GroupSource::Native)
            .await
            .unwrap();
        repo.set_group_roles(group.id, &["admin".to_string()])
            .await
            .unwrap();

        // Warm with the empty answer, then join the group.
        assert!(
            repo.effective_stored_roles(user.id)
                .await
                .unwrap()
                .is_empty()
        );
        repo.add_group_member(group.id, user.id).await.unwrap();
        assert_eq!(
            repo.effective_stored_roles(user.id).await.unwrap(),
            vec!["admin".to_string()],
            "joining a group must grant its roles immediately"
        );

        // Changing the group's roles reaches every member immediately.
        repo.set_group_roles(group.id, &["viewer".to_string()])
            .await
            .unwrap();
        assert_eq!(
            repo.effective_stored_roles(user.id).await.unwrap(),
            vec!["viewer".to_string()]
        );

        // Leaving the group, likewise.
        repo.remove_group_member(group.id, user.id).await.unwrap();
        assert!(
            repo.effective_stored_roles(user.id)
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[tokio::test]
    async fn effective_roles_cache_tracks_group_deletion() {
        let repo = setup_cached_repo(256).await;
        let user = repo.create_user("carol", "hash", true).await.unwrap();
        let group = repo
            .create_group("temp", crate::domain::models::GroupSource::Native)
            .await
            .unwrap();
        repo.set_group_roles(group.id, &["viewer".to_string()])
            .await
            .unwrap();
        repo.add_group_member(group.id, user.id).await.unwrap();
        assert_eq!(
            repo.effective_stored_roles(user.id).await.unwrap(),
            vec!["viewer".to_string()]
        );

        repo.delete_group(group.id).await.unwrap();
        assert!(
            repo.effective_stored_roles(user.id)
                .await
                .unwrap()
                .is_empty(),
            "deleting the group must strip its roles from members immediately"
        );
    }

    /// With caching disabled entirely, every read goes to the database.
    #[tokio::test]
    async fn effective_roles_bypass_when_cache_disabled() {
        let repo = setup_cached_repo(0).await;
        let user = repo.create_user("dave", "hash", true).await.unwrap();
        repo.grant_user_role(user.id, "viewer").await.unwrap();
        assert_eq!(
            repo.effective_stored_roles(user.id).await.unwrap(),
            vec!["viewer".to_string()]
        );
    }
}
