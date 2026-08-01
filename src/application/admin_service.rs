use crate::domain::branch_pattern::branch_matches_any;
use crate::domain::models::*;
use crate::domain::ports::SpecRepository;
use chrono::Utc;
use tracing::instrument;

pub async fn list_protected_branches(repo: &impl SpecRepository) -> Result<Vec<String>, AppError> {
    Ok(repo.list_protected_branches().await?)
}

pub async fn add_protected_branch(
    repo: &impl SpecRepository,
    pattern: &str,
) -> Result<(), AppError> {
    repo.add_protected_branch(pattern).await?;
    Ok(())
}

pub async fn remove_protected_branch(
    repo: &impl SpecRepository,
    pattern: &str,
) -> Result<bool, AppError> {
    Ok(repo.remove_protected_branch(pattern).await?)
}

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

pub async fn delete_branch(
    repo: &impl SpecRepository,
    service_name: &str,
    branch_name: &str,
) -> Result<bool, AppError> {
    Ok(repo.delete_branch(service_name, branch_name).await?)
}

pub async fn reset_branch_history(
    repo: &impl SpecRepository,
    service_name: &str,
    branch_name: &str,
) -> Result<bool, AppError> {
    Ok(repo.reset_branch_history(service_name, branch_name).await?)
}

pub async fn delete_branch_all_services(
    repo: &impl SpecRepository,
    branch_name: &str,
) -> Result<u64, AppError> {
    let services = repo.list_producers().await?;
    let mut count = 0;
    for svc in services {
        if repo.delete_branch(&svc, branch_name).await? {
            count += 1;
        }
    }
    Ok(count)
}

pub async fn delete_consumer(repo: &impl SpecRepository, name: &str) -> Result<bool, AppError> {
    Ok(repo.delete_consumer(name).await?)
}

pub async fn list_producers(repo: &impl SpecRepository) -> Result<Vec<String>, AppError> {
    Ok(repo.list_producers().await?)
}

#[instrument(skip_all)]
pub async fn list_producers_detailed(
    repo: &impl SpecRepository,
    user_id: Option<i64>,
) -> Result<Vec<ProducerSummary>, AppError> {
    let mut services = repo.list_producers_detailed().await?;

    // Last-published time and endpoint count, keyed service -> branch -> value.
    // Nested rather than keyed by a `(String, String)` tuple so lookups below
    // borrow instead of allocating a fresh key pair each time.
    let mut last_published: std::collections::HashMap<
        String,
        std::collections::HashMap<String, String>,
    > = std::collections::HashMap::new();
    for (svc, branch, ts) in repo.list_branch_last_published().await? {
        last_published.entry(svc).or_default().insert(branch, ts);
    }

    // The endpoint count lets the discovery UI tell a branch that serves nothing
    // (e.g. an OpenAPI spec provided with no paths) apart from one that does.
    // `branches` itself is left unfiltered — admin management needs to see and
    // clean up empty branches too.
    let mut endpoint_count: std::collections::HashMap<
        String,
        std::collections::HashMap<String, i64>,
    > = std::collections::HashMap::new();
    for (svc, branch, count) in repo.list_branch_endpoint_counts().await? {
        endpoint_count.entry(svc).or_default().insert(branch, count);
    }

    // Stale-cleanup horizon: non-protected branches are culled once their last
    // publish is older than this many days (0 = disabled). Used to surface a TTL.
    let max_age_days = get_branch_max_age_days(repo).await?;

    let patterns = repo.list_protected_branches().await?;
    let empty_times = std::collections::HashMap::new();
    let empty_counts = std::collections::HashMap::new();
    for svc in &mut services {
        let times = last_published.get(&svc.name).unwrap_or(&empty_times);
        let counts = endpoint_count.get(&svc.name).unwrap_or(&empty_counts);

        // Attach the last-published time per branch, for display in the overview.
        svc.branches_last_published = svc
            .branches
            .iter()
            .filter_map(|b| times.get(b).map(|ts| (b.clone(), ts.clone())))
            .collect();
        // Attach the endpoint count per branch, for filtering in the discovery UI.
        svc.branches_endpoint_count = svc
            .branches
            .iter()
            .map(|b| (b.clone(), counts.get(b).copied().unwrap_or(0)))
            .collect();
        // Attach the stale-cleanup expiry per branch (non-protected only), so the UI
        // can warn when a branch is about to be culled. Protection is glob-matched,
        // so a wildcard-protected branch correctly shows no expiry badge.
        if max_age_days > 0 {
            svc.branches_expire_at = svc
                .branches
                .iter()
                .filter(|b| !branch_matches_any(b, &patterns))
                .filter_map(|b| {
                    let ts = times.get(b)?;
                    let published =
                        chrono::NaiveDateTime::parse_from_str(ts, "%Y-%m-%dT%H:%M:%SZ").ok()?;
                    let expire = published + chrono::Duration::days(max_age_days as i64);
                    Some((b.clone(), expire.format("%Y-%m-%dT%H:%M:%SZ").to_string()))
                })
                .collect();
        }

        // Order branches deterministically: protected first (e.g. master/main),
        // then most recently published, then alphabetically — the underlying
        // GROUP_CONCAT returns them in an unspecified order otherwise. Sort keys
        // are computed once per branch rather than inside the comparator, which
        // would otherwise re-derive them on every comparison.
        let mut keyed: Vec<(bool, &str, &String)> = svc
            .branches
            .iter()
            .map(|b| {
                (
                    branch_matches_any(b, &patterns),
                    times.get(b).map(String::as_str).unwrap_or(""),
                    b,
                )
            })
            .collect();
        keyed.sort_by(|a, b| {
            b.0.cmp(&a.0)
                .then_with(|| b.1.cmp(a.1)) // newest publish first
                .then_with(|| a.2.cmp(b.2))
        });
        let ordered: Vec<String> = keyed.into_iter().map(|(_, _, name)| name.clone()).collect();
        svc.branches = ordered;
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

pub async fn set_fallback_branch(
    repo: &impl SpecRepository,
    service_name: &str,
    branch: Option<&str>,
) -> Result<(), AppError> {
    repo.set_fallback_branch(service_name, branch).await?;
    Ok(())
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

pub async fn get_fallback_branch(
    repo: &impl SpecRepository,
    service_name: &str,
) -> Result<Option<String>, AppError> {
    Ok(repo.get_fallback_branch(service_name).await?)
}

async fn resolve_branch_id(
    repo: &impl SpecRepository,
    service_name: &str,
    branch_name: &str,
) -> Result<i64, AppError> {
    let service_id = repo
        .find_service(service_name)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("Service '{}' not found", service_name)))?;
    repo.find_branch(service_id, branch_name)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("Branch '{}' not found", branch_name)))
}

/// Item #17: get a branch's `source_protected_branch`, if any.
pub async fn get_source_protected_branch(
    repo: &impl SpecRepository,
    service_name: &str,
    branch_name: &str,
) -> Result<Option<String>, AppError> {
    let branch_id = resolve_branch_id(repo, service_name, branch_name).await?;
    Ok(repo.get_source_protected_branch(branch_id).await?)
}

/// Item #17, admin-only: unconditionally set (or clear, with `None`) a
/// branch's `source_protected_branch`, overwriting whatever caller-supplied
/// or previously-admin-set value is currently stored.
pub async fn admin_set_source_protected_branch(
    repo: &impl SpecRepository,
    service_name: &str,
    branch_name: &str,
    value: Option<&str>,
) -> Result<(), AppError> {
    let branch_id = resolve_branch_id(repo, service_name, branch_name).await?;
    repo.admin_set_source_protected_branch(branch_id, value)
        .await?;
    Ok(())
}

pub async fn list_branches(
    repo: &impl SpecRepository,
    service_name: &str,
) -> Result<Vec<String>, AppError> {
    Ok(repo.list_branches(service_name).await?)
}

pub async fn list_all_branches(repo: &impl SpecRepository) -> Result<Vec<String>, AppError> {
    Ok(repo.list_all_branches().await?)
}

pub async fn list_branches_with_metadata(
    repo: &impl SpecRepository,
) -> Result<Vec<BranchMetadata>, AppError> {
    Ok(repo.list_branches_with_metadata().await?)
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

pub async fn list_consumer_branches(
    repo: &impl SpecRepository,
    client_name: &str,
) -> Result<Vec<String>, AppError> {
    Ok(repo.list_consumer_branches(client_name).await?)
}

pub async fn list_consumer_endpoints(
    repo: &impl SpecRepository,
    client_name: &str,
    branch: &str,
) -> Result<Vec<ConsumerEndpointInfo>, AppError> {
    Ok(repo.list_consumer_endpoints(client_name, branch).await?)
}

pub async fn get_branch_max_age_days(repo: &impl SpecRepository) -> Result<u64, AppError> {
    let val = repo
        .get_setting("branch_max_age_days")
        .await?
        .unwrap_or("30".to_string());
    Ok(val.parse().unwrap_or(30))
}

pub async fn set_branch_max_age_days(
    repo: &impl SpecRepository,
    days: u64,
) -> Result<(), AppError> {
    repo.set_setting("branch_max_age_days", &days.to_string())
        .await?;
    Ok(())
}

#[instrument(skip_all)]
pub async fn cleanup_stale_branches(repo: &impl SpecRepository) -> Result<u64, AppError> {
    let days = get_branch_max_age_days(repo).await?;
    if days == 0 {
        return Ok(0);
    }
    let cutoff = Utc::now() - chrono::Duration::days(days as i64);
    Ok(repo.delete_stale_branches(&cutoff.to_rfc3339()).await?)
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

/// Drop message-level channel contracts (item #20) whose branch no longer
/// exists on any service — the counterpart of stale-branch/dependency cleanup.
#[instrument(skip_all)]
pub async fn cleanup_orphaned_channel_message_contracts(
    repo: &impl SpecRepository,
) -> Result<u64, AppError> {
    let live_branches = repo.list_all_branches().await?;
    Ok(repo
        .delete_orphaned_channel_message_contracts(&live_branches)
        .await?)
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
// --- Producer Onboarding ---

/// Put a Producer into onboarding, or take it out.
///
/// While it is on, a Provide to a protected branch is not gatekept: none of the
/// five refusals apply. Deletes stay soft and version history is still written,
/// so this is strictly less destructive than the workaround it replaces —
/// deleting the branch by hand, which throws that history away.
pub async fn set_producer_onboarding(
    repo: &impl SpecRepository,
    producer: &str,
    onboarding: bool,
) -> Result<(), AppError> {
    let service_id = repo
        .find_service(producer)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("Producer '{}' not found", producer)))?;
    repo.set_producer_onboarding(service_id, onboarding).await?;
    Ok(())
}

pub async fn is_producer_onboarding(
    repo: &impl SpecRepository,
    producer: &str,
) -> Result<bool, AppError> {
    let service_id = repo
        .find_service(producer)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("Producer '{}' not found", producer)))?;
    Ok(repo.is_producer_onboarding(service_id).await?)
}

/// Producers currently in onboarding.
///
/// The flag has no expiry, so nothing else will ever mention that it is still
/// on. This listing is that reminder.
pub async fn list_onboarding_producers(
    repo: &impl SpecRepository,
) -> Result<Vec<String>, AppError> {
    Ok(repo.list_onboarding_producers().await?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::mock_repo::MockRepo;

    #[tokio::test]
    async fn test_protected_branch_crud() {
        let repo = MockRepo::new();
        // MockRepo starts with ["main", "master"]
        let initial = list_protected_branches(&repo).await.unwrap();
        let initial_len = initial.len();

        add_protected_branch(&repo, "release/*").await.unwrap();
        let branches = list_protected_branches(&repo).await.unwrap();
        assert_eq!(branches.len(), initial_len + 1);

        assert!(remove_protected_branch(&repo, "main").await.unwrap());
        assert!(!remove_protected_branch(&repo, "nonexistent").await.unwrap());
        assert_eq!(
            list_protected_branches(&repo).await.unwrap().len(),
            initial_len
        );
    }

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
    async fn test_fallback_branch() {
        let repo = MockRepo::new();
        repo.ensure_service("svc").await.unwrap();
        assert!(get_fallback_branch(&repo, "svc").await.unwrap().is_none());

        set_fallback_branch(&repo, "svc", Some("main"))
            .await
            .unwrap();
        assert_eq!(
            get_fallback_branch(&repo, "svc").await.unwrap().unwrap(),
            "main"
        );

        set_fallback_branch(&repo, "svc", None).await.unwrap();
        assert!(get_fallback_branch(&repo, "svc").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn test_cleanup_stale_branches_disabled() {
        let repo = MockRepo::new();
        let count = cleanup_stale_branches(&repo).await.unwrap();
        assert_eq!(count, 0);
    }

    #[tokio::test]
    async fn test_cleanup_stale_dependencies_disabled() {
        let repo = MockRepo::new();
        let count = cleanup_stale_dependencies(&repo).await.unwrap();
        assert_eq!(count, 0);
    }

    #[tokio::test]
    async fn test_favorites_sorting() {
        let repo = MockRepo::new();
        let s_a = repo.ensure_service("svc-a").await.unwrap();
        let s_b = repo.ensure_service("svc-b").await.unwrap();
        let s_c = repo.ensure_service("svc-c").await.unwrap();

        repo.ensure_branch(s_a, "main").await.unwrap();
        repo.ensure_branch(s_b, "main").await.unwrap();
        repo.ensure_branch(s_c, "main").await.unwrap();

        repo.ensure_client("client-a").await.unwrap();
        repo.ensure_client("client-b").await.unwrap();
        repo.ensure_client("client-c").await.unwrap();

        let svcs = list_producers_detailed(&repo, None).await.unwrap();
        assert_eq!(svcs[0].name, "svc-a");
        assert_eq!(svcs[1].name, "svc-b");
        assert_eq!(svcs[2].name, "svc-c");

        let clients = list_consumers(&repo, None).await.unwrap();
        assert_eq!(clients[0], "client-a");
        assert_eq!(clients[1], "client-b");
        assert_eq!(clients[2], "client-c");

        add_user_favorite(&repo, 42, "service", "svc-b")
            .await
            .unwrap();
        add_user_favorite(&repo, 42, "client", "client-c")
            .await
            .unwrap();

        let svcs_fav = list_producers_detailed(&repo, Some(42)).await.unwrap();
        assert_eq!(svcs_fav[0].name, "svc-b");
        assert!(svcs_fav[0].is_favorite);
        assert_eq!(svcs_fav[1].name, "svc-a");
        assert!(!svcs_fav[1].is_favorite);
        assert_eq!(svcs_fav[2].name, "svc-c");
        assert!(!svcs_fav[2].is_favorite);

        let clients_fav = list_consumers(&repo, Some(42)).await.unwrap();
        assert_eq!(clients_fav[0], "client-c");
        assert_eq!(clients_fav[1], "client-a");
        assert_eq!(clients_fav[2], "client-b");

        remove_user_favorite(&repo, 42, "service", "svc-b")
            .await
            .unwrap();
        let svcs_removed = list_producers_detailed(&repo, Some(42)).await.unwrap();
        assert_eq!(svcs_removed[0].name, "svc-a");
    }

    #[tokio::test]
    async fn test_branches_ordered_protected_first_then_alphabetical() {
        let repo = MockRepo::new();
        // MockRepo seeds "main" and "master" as protected branches.
        let s = repo.ensure_service("svc").await.unwrap();
        // Insert in a deliberately unsorted order.
        for b in ["zebra", "master", "alpha", "main"] {
            repo.ensure_branch(s, b).await.unwrap();
        }

        let svcs = list_producers_detailed(&repo, None).await.unwrap();
        let svc = svcs.iter().find(|s| s.name == "svc").unwrap();
        // Protected first (alphabetical among themselves), then the rest alphabetically.
        assert_eq!(svc.branches, vec!["main", "master", "alpha", "zebra"]);
    }
}
