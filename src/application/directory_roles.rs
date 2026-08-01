//! Roles a caller holds because of what the directory says about them.
//!
//! Sanshain stores which roles a directory group confers, never who is in it.
//! Membership belongs to the directory, so it is read from there and cached for
//! a short while rather than copied into the database — which is what the
//! previous arrangement did, once, on the user's first ever login, after which a
//! promotion or demotion in the directory never took effect again.

use crate::domain::models::{AppError, GroupSource};
use crate::domain::ports::{DirectoryGroups, SpecRepository};
use moka::future::Cache;
use std::sync::Arc;
use std::time::Duration;

/// Default lifetime of a cached membership answer.
///
/// The dial between load on the directory and how long a revocation takes to
/// bite. Five minutes keeps per-request LDAP traffic near zero for CI that
/// polls, while making a group change take effect within a coffee break rather
/// than within the seven-day session lifetime.
pub const DEFAULT_GROUP_CACHE_TTL_SECS: u64 = 300;

/// Caches directory membership per username.
#[derive(Clone)]
pub struct DirectoryRoleCache {
    entries: Cache<String, Arc<Vec<String>>>,
}

impl DirectoryRoleCache {
    pub fn new(ttl: Duration) -> Self {
        Self {
            entries: Cache::builder()
                .time_to_live(ttl)
                .max_capacity(10_000)
                .build(),
        }
    }

    /// Resolve the cache TTL from configuration.
    ///
    /// `0` disables caching, which makes every authorisation check hit the
    /// directory — correct, expensive, and available to anyone who wants
    /// revocation to be immediate.
    pub fn from_env() -> Self {
        let secs = std::env::var("SANSHAIN_DIRECTORY_GROUP_TTL_SECS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(DEFAULT_GROUP_CACHE_TTL_SECS);
        Self::new(Duration::from_secs(secs))
    }

    /// A cached answer, if there is one.
    ///
    /// Lets a caller skip building a directory connection on the common path:
    /// on a hit nothing needs to be constructed at all.
    pub async fn cached(&self, username: &str) -> Option<Arc<Vec<String>>> {
        self.entries.get(username).await
    }

    /// Directory groups for a user, from cache or the directory.
    ///
    /// **When the directory is unreachable this returns no groups**, rather than
    /// failing the request or serving an indefinitely stale answer. The choice is
    /// deliberate: directory-derived roles are purely additive, so omitting them
    /// can only ever remove privilege, never grant it. An outage therefore
    /// degrades a directory-backed administrator to whatever Sanshain itself has
    /// granted them — and root, which is held in configuration, still works. The
    /// alternative, serving a stale answer indefinitely, would mean a revoked
    /// administrator kept their access for exactly as long as the outage lasted.
    pub async fn groups_for(
        &self,
        directory: &impl DirectoryGroups,
        username: &str,
    ) -> Arc<Vec<String>> {
        if let Some(cached) = self.entries.get(username).await {
            return cached;
        }

        match directory.groups_for(username).await {
            Ok(groups) => {
                let groups = Arc::new(groups);
                self.entries
                    .insert(username.to_string(), groups.clone())
                    .await;
                groups
            }
            Err(e) => {
                tracing::warn!(
                    "Could not read directory groups for '{}'; continuing without \
                     directory-derived roles: {}",
                    username,
                    e
                );
                Arc::new(Vec::new())
            }
        }
    }

    /// Forget what is cached for a user, so the next check re-reads the
    /// directory. Used when an operator wants a change to take effect at once.
    pub async fn invalidate(&self, username: &str) {
        self.entries.invalidate(username).await;
    }
}

/// Roles conferred by the directory groups a user is in.
///
/// A directory group means nothing to Sanshain until a role is attached to it,
/// so a user in fifty groups Sanshain has never heard of gains nothing.
pub async fn roles_from_directory(
    repo: &impl SpecRepository,
    group_names: &[String],
) -> Result<Vec<String>, AppError> {
    if group_names.is_empty() {
        return Ok(Vec::new());
    }

    let mut roles = Vec::new();
    for group in repo.list_groups().await? {
        if group.source != GroupSource::Ldap || !group_names.contains(&group.name) {
            continue;
        }
        for role in repo.list_group_roles(group.id).await? {
            if !roles.contains(&role) {
                roles.push(role);
            }
        }
    }
    roles.sort();
    Ok(roles)
}

/// Carry a previously configured single admin group over to the group mapping.
///
/// Without this, upgrading would silently strip every directory-backed
/// administrator of their access: the old configuration key stops being consulted
/// and nothing would have taken its place.
pub async fn migrate_admin_group(
    repo: &impl SpecRepository,
    admin_group: &str,
) -> Result<(), AppError> {
    let admin_group = admin_group.trim();
    if admin_group.is_empty() {
        return Ok(());
    }

    let group = repo.create_group(admin_group, GroupSource::Ldap).await?;
    let mut roles = repo.list_group_roles(group.id).await?;
    if roles.iter().any(|r| r == "admin") {
        return Ok(());
    }
    roles.push("admin".to_string());
    repo.set_group_roles(group.id, &roles).await?;
    tracing::info!(
        "Directory group '{}' now grants the admin role, carried over from the \
         previous admin-group setting",
        admin_group
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::mock_repo::MockRepo;
    use crate::domain::ports::AuthProviderError;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct StubDirectory {
        groups: Mutex<Vec<String>>,
        calls: AtomicUsize,
        fail: Mutex<bool>,
    }

    impl StubDirectory {
        fn new(groups: &[&str]) -> Self {
            Self {
                groups: Mutex::new(groups.iter().map(|g| g.to_string()).collect()),
                calls: AtomicUsize::new(0),
                fail: Mutex::new(false),
            }
        }
        fn calls(&self) -> usize {
            self.calls.load(Ordering::SeqCst)
        }
        fn set_groups(&self, groups: &[&str]) {
            *self.groups.lock().unwrap_or_else(|e| e.into_inner()) =
                groups.iter().map(|g| g.to_string()).collect();
        }
        fn set_failing(&self, failing: bool) {
            *self.fail.lock().unwrap_or_else(|e| e.into_inner()) = failing;
        }
    }

    impl DirectoryGroups for StubDirectory {
        async fn groups_for(&self, _username: &str) -> Result<Vec<String>, AuthProviderError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            if *self.fail.lock().unwrap_or_else(|e| e.into_inner()) {
                return Err(AuthProviderError::ConnectionFailed("unreachable".into()));
            }
            Ok(self
                .groups
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .clone())
        }
    }

    #[tokio::test]
    async fn membership_is_read_from_the_directory_and_cached() {
        let directory = StubDirectory::new(&["cn=admins"]);
        let cache = DirectoryRoleCache::new(Duration::from_secs(300));

        let first = cache.groups_for(&directory, "alice").await;
        let second = cache.groups_for(&directory, "alice").await;
        assert_eq!(*first, vec!["cn=admins".to_string()]);
        assert_eq!(*second, vec!["cn=admins".to_string()]);
        assert_eq!(directory.calls(), 1, "the second read came from the cache");
    }

    /// The bug this replaces: membership captured once and never refreshed.
    #[tokio::test]
    async fn a_directory_change_takes_effect_once_the_entry_expires() {
        let directory = StubDirectory::new(&["cn=admins"]);
        let cache = DirectoryRoleCache::new(Duration::from_secs(300));

        assert_eq!(
            *cache.groups_for(&directory, "alice").await,
            vec!["cn=admins"]
        );
        directory.set_groups(&[]);
        cache.invalidate("alice").await;

        assert!(
            cache.groups_for(&directory, "alice").await.is_empty(),
            "a demotion in the directory must take effect"
        );
    }

    #[tokio::test]
    async fn a_zero_ttl_reads_the_directory_every_time() {
        let directory = StubDirectory::new(&["cn=admins"]);
        let cache = DirectoryRoleCache::new(Duration::from_secs(0));

        cache.groups_for(&directory, "alice").await;
        cache.groups_for(&directory, "alice").await;
        assert_eq!(directory.calls(), 2);
    }

    /// An outage removes directory-derived privilege rather than granting it or
    /// failing the request outright.
    #[tokio::test]
    async fn an_unreachable_directory_yields_no_groups() {
        let directory = StubDirectory::new(&["cn=admins"]);
        directory.set_failing(true);
        let cache = DirectoryRoleCache::new(Duration::from_secs(300));

        assert!(
            cache.groups_for(&directory, "alice").await.is_empty(),
            "an outage must not grant privilege, and must not fail the request"
        );
    }

    #[tokio::test]
    async fn only_groups_sanshain_knows_about_confer_roles() {
        let repo = MockRepo::new();
        let known = repo
            .create_group("cn=admins", GroupSource::Ldap)
            .await
            .expect("group");
        repo.set_group_roles(known.id, &["admin".to_string()])
            .await
            .expect("roles");

        let roles = roles_from_directory(
            &repo,
            &[
                "cn=admins".to_string(),
                "cn=some-other-group-entirely".to_string(),
            ],
        )
        .await
        .expect("roles");
        assert_eq!(roles, vec!["admin".to_string()]);
    }

    /// A native group of the same name must not be mistaken for the directory's.
    #[tokio::test]
    async fn a_native_group_of_the_same_name_confers_nothing_via_the_directory() {
        let repo = MockRepo::new();
        let native = repo
            .create_group("developers", GroupSource::Native)
            .await
            .expect("group");
        repo.set_group_roles(native.id, &["admin".to_string()])
            .await
            .expect("roles");

        let roles = roles_from_directory(&repo, &["developers".to_string()])
            .await
            .expect("roles");
        assert!(
            roles.is_empty(),
            "membership of a native group comes from Sanshain, not the directory"
        );
    }

    #[tokio::test]
    async fn no_directory_groups_means_no_roles() {
        let repo = MockRepo::new();
        assert!(
            roles_from_directory(&repo, &[])
                .await
                .expect("roles")
                .is_empty()
        );
    }

    #[tokio::test]
    async fn the_previous_admin_group_setting_is_carried_over() {
        let repo = MockRepo::new();
        migrate_admin_group(&repo, "cn=admins,dc=example,dc=com")
            .await
            .expect("migration");

        let roles = roles_from_directory(&repo, &["cn=admins,dc=example,dc=com".to_string()])
            .await
            .expect("roles");
        assert_eq!(
            roles,
            vec!["admin".to_string()],
            "a directory-backed administrator keeps their access across the upgrade"
        );
    }

    #[tokio::test]
    async fn carrying_over_twice_is_a_no_op() {
        let repo = MockRepo::new();
        migrate_admin_group(&repo, "cn=admins")
            .await
            .expect("first");
        migrate_admin_group(&repo, "cn=admins")
            .await
            .expect("second");

        let groups = repo.list_groups().await.expect("groups");
        assert_eq!(groups.len(), 1);
        assert_eq!(
            repo.list_group_roles(groups[0].id).await.expect("roles"),
            vec!["admin".to_string()]
        );
    }

    #[tokio::test]
    async fn an_empty_admin_group_setting_creates_nothing() {
        let repo = MockRepo::new();
        migrate_admin_group(&repo, "   ").await.expect("migration");
        assert!(repo.list_groups().await.expect("groups").is_empty());
    }
}
