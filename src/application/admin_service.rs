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

/// Delete every user who is not an effective administrator. Spared alongside
/// direct/group admins are the configured root usernames, who hold their power
/// through configuration rather than a stored role.
pub async fn delete_all_non_admin_users(
    repo: &impl SpecRepository,
    root_users: &crate::domain::permissions::RootUsers,
) -> Result<u64, AppError> {
    let spare: Vec<String> = root_users.usernames().map(str::to_string).collect();
    Ok(repo.delete_all_non_admin_users(&spare).await?)
}

pub async fn nuke_database(
    repo: &impl SpecRepository,
    keep_user_id: Option<i64>,
) -> Result<(), AppError> {
    repo.nuke_database(keep_user_id).await?;
    Ok(())
}

/// The sanshain-branches whose recorded graph still references this
/// participant (as Consumer or Producer). Surfaced before deleting it, for
/// the same reason [`list_version_dependents`] names branches before a
/// version delete — except the stakes are higher here: the participant's rows
/// cascade out of every branch graph, so those release cuts lose the edges
/// retroactively rather than showing them as dangling. Removing that
/// asymmetry is `ai/improvements.md` #26; until then, say it out loud.
///
/// Composed from the branch listing rather than a dedicated query: release
/// cuts are few and this runs only on the interactive delete path.
pub async fn list_participant_branch_references(
    repo: &impl SpecRepository,
    name: &str,
    role: ParticipantRole,
) -> Result<Vec<String>, AppError> {
    // A Producer can be present in a release cut through a tagged Provide
    // alone — a hotfix recorded as a member version before anything pins it —
    // so the pin scan is not the whole picture for that role.
    let member_branches = match role {
        ParticipantRole::Producer => match repo.find_service(name).await? {
            Some(sid) => repo
                .list_branch_memberships_for_service(sid)
                .await?
                .into_iter()
                .map(|m| m.branch)
                .collect(),
            None => Vec::new(),
        },
        ParticipantRole::Consumer => Vec::new(),
    };

    let mut referencing = Vec::new();
    for branch in repo.list_branches().await? {
        let pins = repo.list_branch_pins(branch.id, None).await?;
        let touched = pins.iter().any(|p| match role {
            ParticipantRole::Consumer => p.client == name,
            ParticipantRole::Producer => p.service == name,
        }) || member_branches.contains(&branch.name);
        if touched {
            referencing.push(branch.name);
        }
    }
    Ok(referencing)
}

/// Which side of an edge a delete removes. Names are unique per table, not
/// across them — one service commonly appears as both — so matching either
/// column would report release graphs that lose nothing.
#[derive(Clone, Copy)]
pub enum ParticipantRole {
    Consumer,
    Producer,
}

/// A participant's rows cascade out of every graph that references them, and
/// a sanshain-branch is a frozen record — losing edges from a recorded release
/// cut would rewrite history retroactively. So the delete is refused while any
/// branch still references the participant, naming them so the admin knows
/// what to retire first.
///
/// Trunk presence deliberately does *not* block: trunk is the living stream,
/// it already churns and ages out on its TTL, and with trunk CI in use almost
/// every participant appears there — blocking on it would make retiring a
/// decommissioned service impossible. The trunk timeline does lose that
/// participant's closed rows; see `ai/improvements.md` #26.
async fn refuse_if_a_release_graph_needs_it(
    repo: &impl SpecRepository,
    name: &str,
    role: ParticipantRole,
) -> Result<(), AppError> {
    let branches = list_participant_branch_references(repo, name, role).await?;
    if branches.is_empty() {
        return Ok(());
    }
    Err(AppError::Conflict(format!(
        "'{name}' is part of the recorded graph of sanshain-branch(es) {} — \
         deleting it would remove those edges from release cuts that already \
         happened. Delete the branch(es) first if the record is no longer needed.",
        branches.join(", ")
    )))
}

pub async fn delete_producer(repo: &impl SpecRepository, name: &str) -> Result<bool, AppError> {
    refuse_if_a_release_graph_needs_it(repo, name, ParticipantRole::Producer).await?;
    Ok(repo.delete_producer(name).await?)
}

pub async fn delete_consumer(repo: &impl SpecRepository, name: &str) -> Result<bool, AppError> {
    refuse_if_a_release_graph_needs_it(repo, name, ParticipantRole::Consumer).await?;
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

/// The Consumers pinned to a version plus the sanshain-branches referencing
/// it. ADR-0005: release graphs are part of the informed-delete picture — a
/// warning, never a block. Deleting anyway leaves them visibly dangling; a
/// later re-provide heals them.
async fn version_dependents(
    repo: &impl SpecRepository,
    entry: &SpecVersionMeta,
) -> Result<Vec<String>, AppError> {
    let mut dependents = repo.list_version_dependents(entry.id).await?;
    for branch in repo
        .list_branches_referencing(entry.service_id, entry.api_type, entry.version)
        .await?
    {
        dependents.push(format!("sanshain-branch '{branch}'"));
    }
    Ok(dependents)
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
    version_dependents(repo, &entry).await
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
    let dependents = version_dependents(repo, &entry).await?;
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
                trunk_provided_at: meta.trunk_provided_at,
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

/// Upper bound for every *_max_age_days setting (~100 years): far beyond any
/// real retention need, and small enough that the chrono duration math on the
/// cleanup and report paths can never overflow (chrono panics on overflow).
pub const MAX_AGE_DAYS_LIMIT: u64 = 36_500;

fn validate_max_age_days(days: u64) -> Result<(), AppError> {
    if days > MAX_AGE_DAYS_LIMIT {
        return Err(AppError::BadRequest(format!(
            "days must be at most {MAX_AGE_DAYS_LIMIT}"
        )));
    }
    Ok(())
}

pub async fn get_snapshot_max_age_days(repo: &impl SpecRepository) -> Result<u64, AppError> {
    let val = repo
        .get_setting("snapshot_max_age_days")
        .await?
        .unwrap_or("30".to_string());
    // The clamp covers values stored before the setter validated.
    Ok(val.parse().unwrap_or(30).min(MAX_AGE_DAYS_LIMIT))
}

pub async fn set_snapshot_max_age_days(
    repo: &impl SpecRepository,
    days: u64,
) -> Result<(), AppError> {
    validate_max_age_days(days)?;
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
    Ok(val.parse().unwrap_or(30).min(MAX_AGE_DAYS_LIMIT))
}

pub async fn set_dependency_max_age_days(
    repo: &impl SpecRepository,
    days: u64,
) -> Result<(), AppError> {
    validate_max_age_days(days)?;
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

/// Trunk TTL (ADR-0004): month-scale, independent of the much shorter dev
/// expiry. 0 disables the cleanup.
pub async fn get_trunk_max_age_days(repo: &impl SpecRepository) -> Result<u64, AppError> {
    let val = repo
        .get_setting("trunk_max_age_days")
        .await?
        .unwrap_or("90".to_string());
    Ok(val.parse().unwrap_or(90).min(MAX_AGE_DAYS_LIMIT))
}

pub async fn set_trunk_max_age_days(repo: &impl SpecRepository, days: u64) -> Result<(), AppError> {
    validate_max_age_days(days)?;
    repo.set_setting("trunk_max_age_days", &days.to_string())
        .await?;
    Ok(())
}

/// Expired sessions and API tokens are already refused by validation, so this
/// reclaims storage rather than changing who can log in. Without it the
/// credential tables grow forever — a record of every login the service ever
/// issued, kept indefinitely for no operational purpose.
#[instrument(skip_all)]
pub async fn cleanup_expired_credentials(repo: &impl SpecRepository) -> Result<u64, AppError> {
    Ok(repo.delete_expired_credentials(&super::now_iso()).await?)
}

/// Close trunk pins and clear trunk markers not refreshed within the TTL —
/// they leave the *current* view; the closed rows stay as history (ADR-0005).
#[instrument(skip_all)]
pub async fn cleanup_stale_trunk_data(repo: &impl SpecRepository) -> Result<u64, AppError> {
    let days = get_trunk_max_age_days(repo).await?;
    if days == 0 {
        return Ok(0);
    }
    let now = Utc::now();
    let cutoff = (now - chrono::Duration::days(days as i64))
        .to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    let now = now.to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    Ok(repo.close_expired_trunk_data(&cutoff, &now).await?)
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
    async fn max_age_settings_reject_and_clamp_absurd_values() {
        let repo = MockRepo::new();
        for result in [
            set_snapshot_max_age_days(&repo, MAX_AGE_DAYS_LIMIT + 1).await,
            set_dependency_max_age_days(&repo, MAX_AGE_DAYS_LIMIT + 1).await,
            set_trunk_max_age_days(&repo, MAX_AGE_DAYS_LIMIT + 1).await,
        ] {
            assert!(matches!(result, Err(AppError::BadRequest(_))));
        }
        set_trunk_max_age_days(&repo, MAX_AGE_DAYS_LIMIT)
            .await
            .unwrap();
        assert_eq!(
            get_trunk_max_age_days(&repo).await.unwrap(),
            MAX_AGE_DAYS_LIMIT
        );

        // A value stored before the setter validated is clamped on read, so
        // the chrono duration math on the report/cleanup paths cannot panic.
        repo.set_setting("snapshot_max_age_days", "999999999999999")
            .await
            .unwrap();
        assert_eq!(
            get_snapshot_max_age_days(&repo).await.unwrap(),
            MAX_AGE_DAYS_LIMIT
        );
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
