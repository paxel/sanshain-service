use crate::domain::models::*;
use crate::domain::ports::SpecRepository;
use chrono::Utc;
use tracing::instrument;

pub async fn delete_all_services(repo: &impl SpecRepository) -> Result<u64, AppError> {
    Ok(repo.delete_all_services().await?)
}

pub async fn delete_all_clients(repo: &impl SpecRepository) -> Result<u64, AppError> {
    Ok(repo.delete_all_clients().await?)
}

pub async fn delete_all_non_admin_users(repo: &impl SpecRepository) -> Result<u64, AppError> {
    Ok(repo.delete_all_non_admin_users().await?)
}

pub async fn nuke_database(
    repo: &impl SpecRepository,
    keep_user_id: Option<i64>,
) -> Result<(), AppError> {
    repo.nuke_database(keep_user_id).await?;
    Ok(())
}

pub async fn delete_producer(repo: &impl SpecRepository, name: &str) -> Result<bool, AppError> {
    Ok(repo.delete_producer(name).await?)
}

pub async fn delete_consumer(repo: &impl SpecRepository, name: &str) -> Result<bool, AppError> {
    Ok(repo.delete_consumer(name).await?)
}

pub async fn list_producers(repo: &impl SpecRepository) -> Result<Vec<String>, AppError> {
    Ok(repo.list_producers().await?)
}

/// Resolve a version-line entry by name. Shared by everything that addresses
/// a version on the admin surface.
pub async fn find_version_entry(
    repo: &impl SpecRepository,
    producer: &str,
    api_type: ApiType,
    version: SemVer,
) -> Result<SpecVersionMeta, AppError> {
    let service_id = repo
        .find_service(producer)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("Producer '{}' not found", producer)))?;
    repo.find_spec_version(service_id, api_type, version)
        .await?
        .ok_or_else(|| {
            AppError::NotFound(format!(
                "Producer '{}' has no {} version {}",
                producer,
                api_type.as_str(),
                version
            ))
        })
}

/// The Consumers currently pinned to a version — surfaced *before* a
/// delete-version is confirmed, because deleting a depended-on version means
/// those builds hard-fail. Visible, not hidden.
pub async fn list_version_dependents(
    repo: &impl SpecRepository,
    producer: &str,
    api_type: ApiType,
    version: SemVer,
) -> Result<Vec<String>, AppError> {
    let entry = find_version_entry(repo, producer, api_type, version).await?;
    Ok(repo.list_version_dependents(entry.id).await?)
}

/// The sole escape hatch from GA immutability (ADR-0003): delete the version
/// outright, freeing its number. Deliberately *delete*, not edit — content
/// immutability stays absolute. Returns the Consumers that were pinned to it.
pub async fn delete_version(
    repo: &impl SpecRepository,
    producer: &str,
    api_type: ApiType,
    version: SemVer,
) -> Result<Vec<String>, AppError> {
    let entry = find_version_entry(repo, producer, api_type, version).await?;
    let dependents = repo.list_version_dependents(entry.id).await?;
    repo.delete_spec_version(entry.id).await?;
    Ok(dependents)
}

/// Every Producer's version-line entries, decorated with endpoint counts and
/// snapshot expiry — shared by the producers listing and the per-Producer
/// versions endpoint.
async fn producer_versions_map(
    repo: &impl SpecRepository,
) -> Result<std::collections::HashMap<String, Vec<ProducerVersionInfo>>, AppError> {
    let max_age_days = get_snapshot_max_age_days(repo).await?;

    let mut versions_by_service: std::collections::HashMap<String, Vec<ProducerVersionInfo>> =
        std::collections::HashMap::new();
    for (service_name, meta, endpoint_count) in repo.list_all_spec_versions().await? {
        // Use-based expiry: a snapshot dies only when neither provided nor
        // required for the window, so the surfaced TTL counts from whichever
        // of the two happened last. GA never expires.
        let expires_at = if meta.stability == Stability::Snapshot && max_age_days > 0 {
            let last_use = meta
                .last_required_at
                .as_deref()
                .filter(|r| *r > meta.updated_at.as_str())
                .unwrap_or(meta.updated_at.as_str());
            chrono::DateTime::parse_from_rfc3339(last_use)
                .ok()
                .map(|t| {
                    (t + chrono::Duration::days(max_age_days as i64))
                        .to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
                })
        } else {
            None
        };
        versions_by_service
            .entry(service_name)
            .or_default()
            .push(ProducerVersionInfo {
                api_type: meta.api_type,
                version: meta.version,
                stability: meta.stability,
                content_hash: meta.content_hash,
                provided_by: meta.provided_by,
                created_at: meta.created_at,
                updated_at: meta.updated_at,
                last_required_at: meta.last_required_at,
                endpoint_count,
                expires_at,
            });
    }

    // One timeline per line: API types grouped, newest version first.
    for versions in versions_by_service.values_mut() {
        versions.sort_by(|a, b| {
            a.api_type
                .as_str()
                .cmp(b.api_type.as_str())
                .then_with(|| b.version.cmp(&a.version))
        });
    }
    Ok(versions_by_service)
}

/// The version lines of one Producer, optionally narrowed to one API type.
pub async fn list_producer_versions(
    repo: &impl SpecRepository,
    producer: &str,
    api_type: Option<ApiType>,
) -> Result<Vec<ProducerVersionInfo>, AppError> {
    repo.find_service(producer)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("Producer '{}' not found", producer)))?;
    let mut versions = producer_versions_map(repo)
        .await?
        .remove(producer)
        .unwrap_or_default();
    if let Some(api_type) = api_type {
        versions.retain(|v| v.api_type == api_type);
    }
    Ok(versions)
}

#[instrument(skip_all)]
pub async fn list_producers_detailed(
    repo: &impl SpecRepository,
    user_id: Option<i64>,
) -> Result<Vec<ProducerSummary>, AppError> {
    let mut services = repo.list_producers_detailed().await?;
    let mut versions_by_service = producer_versions_map(repo).await?;

    for svc in &mut services {
        svc.versions = versions_by_service.remove(&svc.name).unwrap_or_default();
    }

    if let Some(uid) = user_id {
        let favorites = repo.get_user_favorites(uid, "service").await?;
        for svc in &mut services {
            if favorites.contains(&svc.name) {
                svc.is_favorite = true;
            }
        }
        services.sort_by_key(|b| std::cmp::Reverse(b.is_favorite));
    }
    Ok(services)
}

pub async fn update_producer_metadata(
    repo: &impl SpecRepository,
    service_name: &str,
    icon: Option<&str>,
    domain: Option<&str>,
) -> Result<(), AppError> {
    repo.update_producer_metadata(service_name, icon, domain)
        .await?;
    Ok(())
}

pub async fn list_consumers(
    repo: &impl SpecRepository,
    user_id: Option<i64>,
) -> Result<Vec<String>, AppError> {
    let mut clients = repo.list_consumers().await?;
    if let Some(uid) = user_id {
        let favorites = repo.get_user_favorites(uid, "client").await?;
        clients.sort_by(|a, b| {
            let a_fav = favorites.contains(a);
            let b_fav = favorites.contains(b);
            b_fav.cmp(&a_fav)
        });
    }
    Ok(clients)
}

pub async fn list_consumer_endpoints(
    repo: &impl SpecRepository,
    client_name: &str,
) -> Result<Vec<ConsumerEndpointInfo>, AppError> {
    Ok(repo.list_consumer_endpoints(client_name).await?)
}

pub async fn get_snapshot_max_age_days(repo: &impl SpecRepository) -> Result<u64, AppError> {
    let val = repo
        .get_setting("snapshot_max_age_days")
        .await?
        .unwrap_or("30".to_string());
    Ok(val.parse().unwrap_or(30))
}

pub async fn set_snapshot_max_age_days(
    repo: &impl SpecRepository,
    days: u64,
) -> Result<(), AppError> {
    repo.set_setting("snapshot_max_age_days", &days.to_string())
        .await?;
    Ok(())
}

/// Use-based snapshot expiry: a snapshot neither provided nor required for
/// `snapshot_max_age_days` is abandoned work and dies. GA is never age-culled.
#[instrument(skip_all)]
pub async fn cleanup_expired_snapshots(repo: &impl SpecRepository) -> Result<u64, AppError> {
    let days = get_snapshot_max_age_days(repo).await?;
    if days == 0 {
        return Ok(0);
    }
    let cutoff = Utc::now() - chrono::Duration::days(days as i64);
    Ok(repo.delete_expired_snapshots(&cutoff.to_rfc3339()).await?)
}

pub async fn get_dependency_max_age_days(repo: &impl SpecRepository) -> Result<u64, AppError> {
    let val = repo
        .get_setting("dependency_max_age_days")
        .await?
        .unwrap_or("30".to_string());
    Ok(val.parse().unwrap_or(30))
}

pub async fn set_dependency_max_age_days(
    repo: &impl SpecRepository,
    days: u64,
) -> Result<(), AppError> {
    repo.set_setting("dependency_max_age_days", &days.to_string())
        .await?;
    Ok(())
}

#[instrument(skip_all)]
pub async fn cleanup_stale_dependencies(repo: &impl SpecRepository) -> Result<u64, AppError> {
    let days = get_dependency_max_age_days(repo).await?;
    if days == 0 {
        return Ok(0);
    }
    let cutoff = Utc::now() - chrono::Duration::days(days as i64);
    Ok(repo.delete_stale_dependencies(&cutoff.to_rfc3339()).await?)
}

pub async fn get_user_favorites(
    repo: &impl SpecRepository,
    user_id: i64,
) -> Result<UserFavoritesResponse, AppError> {
    let services = repo.get_user_favorites(user_id, "service").await?;
    let clients = repo.get_user_favorites(user_id, "client").await?;
    Ok(UserFavoritesResponse { services, clients })
}

pub async fn add_user_favorite(
    repo: &impl SpecRepository,
    user_id: i64,
    item_type: &str,
    item_name: &str,
) -> Result<(), AppError> {
    if item_type != "service" && item_type != "client" {
        return Err(AppError::BadRequest(
            "Invalid item type. Must be 'service' or 'client'".to_string(),
        ));
    }
    repo.add_user_favorite(user_id, item_type, item_name)
        .await?;
    Ok(())
}

pub async fn remove_user_favorite(
    repo: &impl SpecRepository,
    user_id: i64,
    item_type: &str,
    item_name: &str,
) -> Result<(), AppError> {
    if item_type != "service" && item_type != "client" {
        return Err(AppError::BadRequest(
            "Invalid item type. Must be 'service' or 'client'".to_string(),
        ));
    }
    repo.remove_user_favorite(user_id, item_type, item_name)
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::mock_repo::MockRepo;

    #[tokio::test]
    async fn test_list_services() {
        let repo = MockRepo::new();
        repo.ensure_service("svc-a").await.unwrap();
        repo.ensure_service("svc-b").await.unwrap();
        let svcs = list_producers(&repo).await.unwrap();
        assert_eq!(svcs.len(), 2);
    }

    #[tokio::test]
    async fn test_delete_service() {
        let repo = MockRepo::new();
        repo.ensure_service("svc").await.unwrap();
        assert!(delete_producer(&repo, "svc").await.unwrap());
        assert!(!delete_producer(&repo, "svc").await.unwrap());
    }

    #[tokio::test]
    async fn test_cleanup_expired_snapshots_disabled() {
        let repo = MockRepo::new();
        set_snapshot_max_age_days(&repo, 0).await.unwrap();
        let count = cleanup_expired_snapshots(&repo).await.unwrap();
        assert_eq!(count, 0);
    }

    #[tokio::test]
    async fn test_cleanup_stale_dependencies_disabled() {
        let repo = MockRepo::new();
        let count = cleanup_stale_dependencies(&repo).await.unwrap();
        assert_eq!(count, 0);
    }

    #[tokio::test]
    async fn test_delete_version_unknown_is_not_found() {
        let repo = MockRepo::new();
        repo.ensure_service("svc").await.unwrap();
        let err = delete_version(&repo, "svc", ApiType::OpenApi, "1.0.0".parse().unwrap())
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::NotFound(_)));
    }

    #[tokio::test]
    async fn test_favorites_sorting() {
        let repo = MockRepo::new();
        repo.ensure_service("svc-a").await.unwrap();
        repo.ensure_service("svc-b").await.unwrap();
        repo.ensure_service("svc-c").await.unwrap();

        repo.ensure_client("client-a").await.unwrap();
        repo.ensure_client("client-b").await.unwrap();
        repo.ensure_client("client-c").await.unwrap();

        let svcs = list_producers_detailed(&repo, None).await.unwrap();
        assert_eq!(svcs[0].name, "svc-a");
        assert_eq!(svcs[1].name, "svc-b");
        assert_eq!(svcs[2].name, "svc-c");

        let clients = list_consumers(&repo, None).await.unwrap();
        assert_eq!(clients[0], "client-a");

        add_user_favorite(&repo, 42, "service", "svc-b")
            .await
            .unwrap();
        add_user_favorite(&repo, 42, "client", "client-c")
            .await
            .unwrap();

        let svcs_fav = list_producers_detailed(&repo, Some(42)).await.unwrap();
        assert_eq!(svcs_fav[0].name, "svc-b");
        assert!(svcs_fav[0].is_favorite);

        let clients_fav = list_consumers(&repo, Some(42)).await.unwrap();
        assert_eq!(clients_fav[0], "client-c");

        remove_user_favorite(&repo, 42, "service", "svc-b")
            .await
            .unwrap();
        let svcs_removed = list_producers_detailed(&repo, Some(42)).await.unwrap();
        assert_eq!(svcs_removed[0].name, "svc-a");
    }
}
