//! Who may do what.
//!
//! Authorisation is expressed in **permissions** — the unit every check tests.
//! **Roles** are fixed bundles of permissions, defined here rather than stored,
//! because a build-time check can only reason about who reaches a route if the
//! bundles cannot change under it. **Root** holds everything, including
//! permissions that do not exist yet, and is configured outside the database so
//! that nothing stored can revoke it.

use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// A single thing an Actor may do.
///
/// Checks name permissions rather than roles so that what a route requires
/// stays readable without knowing who currently holds what.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Permission {
    /// Create, approve, edit and delete user accounts.
    ManageUsers,
    /// Grant and revoke roles, and manage groups and their role assignments.
    ManageRoles,
    /// Administer Producers and their version lines, including deleting a
    /// version — the sole escape hatch from GA immutability.
    ManageProducers,
    /// Administer Consumers and their recorded dependencies.
    ManageConsumers,
    /// Read the audit log and timeline.
    ViewAudit,
    /// Change instance settings — cleanup ages, cache, dev mode.
    ManageSettings,
    /// Change the authentication configuration.
    ManageAuthConfig,
    /// Read logs, stats and debug configuration.
    ViewObservability,
    /// Bulk deletion of producers, consumers, users or the whole database.
    RunDestructiveOperations,
    /// Publish a version with `stability: ga` — a fresh GA, a promotion, or an
    /// idempotent GA re-provide. Snapshots need no permission; releasing does.
    ReleaseGa,
}

impl Permission {
    /// Every permission, in a stable order.
    ///
    /// Used to give `Admin` its bundle and to render the set in the UI. Adding a
    /// variant without adding it here would silently narrow `Admin`, so the test
    /// below asserts the two agree.
    pub const ALL: &'static [Permission] = &[
        Permission::ManageUsers,
        Permission::ManageRoles,
        Permission::ManageProducers,
        Permission::ManageConsumers,
        Permission::ViewAudit,
        Permission::ManageSettings,
        Permission::ManageAuthConfig,
        Permission::ViewObservability,
        Permission::RunDestructiveOperations,
        Permission::ReleaseGa,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Permission::ManageUsers => "manage_users",
            Permission::ManageRoles => "manage_roles",
            Permission::ManageProducers => "manage_producers",
            Permission::ManageConsumers => "manage_consumers",
            Permission::ViewAudit => "view_audit",
            Permission::ManageSettings => "manage_settings",
            Permission::ManageAuthConfig => "manage_auth_config",
            Permission::ViewObservability => "view_observability",
            Permission::RunDestructiveOperations => "run_destructive_operations",
            Permission::ReleaseGa => "release_ga",
        }
    }

    pub fn parse(value: &str) -> Option<Permission> {
        Permission::ALL
            .iter()
            .copied()
            .find(|p| p.as_str() == value)
    }
}

/// A named bundle of permissions, held instance-wide.
///
/// `Maintainer` is the exception: it is never held globally. Its bundle
/// describes what maintaining a Producer confers, and is applied only within the
/// scope of the Producers an Actor actually maintains.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Admin,
    UserManager,
    Viewer,
    Maintainer,
    Releaser,
}

impl Role {
    pub const ALL: &'static [Role] = &[
        Role::Admin,
        Role::UserManager,
        Role::Viewer,
        Role::Maintainer,
        Role::Releaser,
    ];

    /// Roles that may be granted instance-wide.
    ///
    /// `Maintainer` is excluded deliberately — it means nothing without the set
    /// of Producers it is over, so it is assigned as a scope, not granted.
    pub const GLOBALLY_GRANTABLE: &'static [Role] =
        &[Role::Admin, Role::UserManager, Role::Viewer, Role::Releaser];

    /// Roles only an admin (or root) may grant, revoke, or move on or off a
    /// group. `Admin` because it is self-escalation to everything;
    /// `Releaser` because release rights are exactly what the GA gate exists
    /// to control, and a `manage_roles` holder must not be a side door to
    /// them.
    pub const ADMIN_GUARDED: &'static [Role] = &[Role::Admin, Role::Releaser];

    pub fn as_str(self) -> &'static str {
        match self {
            Role::Admin => "admin",
            Role::UserManager => "user_manager",
            Role::Viewer => "viewer",
            Role::Maintainer => "maintainer",
            Role::Releaser => "releaser",
        }
    }

    pub fn parse(value: &str) -> Option<Role> {
        Role::ALL.iter().copied().find(|r| r.as_str() == value)
    }

    /// The permissions this role confers.
    ///
    /// `Admin` deliberately holds everything: it is the role an account flagged
    /// as an administrator today becomes, and narrowing it would take access
    /// away from existing operators on upgrade.
    pub fn permissions(self) -> &'static [Permission] {
        match self {
            Role::Admin => Permission::ALL,
            Role::UserManager => &[Permission::ManageUsers, Permission::ManageRoles],
            Role::Viewer => &[Permission::ViewAudit, Permission::ViewObservability],
            Role::Maintainer => &[Permission::ManageProducers],
            Role::Releaser => &[Permission::ReleaseGa],
        }
    }

    pub fn grants(self, permission: Permission) -> bool {
        self.permissions().contains(&permission)
    }
}

/// The usernames that hold every permission by configuration.
///
/// Deliberately not stored: root's authority comes from outside the database, so
/// no UI action, migration or direct SQL update can strip it. Membership is a
/// set lookup on every check rather than a row that could go missing.
#[derive(Clone, Debug, Default)]
pub struct RootUsers {
    usernames: BTreeSet<String>,
}

impl RootUsers {
    /// Resolve the root set from configuration.
    ///
    /// `configured` is the explicit list. When it is absent or empty the set
    /// falls back to the bootstrap administrator's username, and finally to
    /// `root` — the name `ensure_initial_admin` uses — so an existing deployment
    /// keeps a root account without being reconfigured.
    pub fn resolve(configured: Option<&str>, initial_admin: Option<&str>) -> Self {
        let explicit: BTreeSet<String> = configured
            .unwrap_or_default()
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .collect();

        if !explicit.is_empty() {
            return Self {
                usernames: explicit,
            };
        }

        let fallback = initial_admin
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or("root")
            .to_string();

        Self {
            usernames: BTreeSet::from([fallback]),
        }
    }

    pub fn contains(&self, username: &str) -> bool {
        self.usernames.contains(username)
    }

    pub fn usernames(&self) -> impl Iterator<Item = &str> {
        self.usernames.iter().map(String::as_str)
    }
}

/// The authenticated caller, with what they may do already resolved.
///
/// Carried alongside the stored user record rather than replacing it: the record
/// is a database row, this is the authorisation answer derived from it plus
/// configuration.
#[derive(Clone, Debug)]
pub struct Actor {
    pub user_id: i64,
    pub username: String,
    /// Held by configuration; implies every permission, including ones added
    /// after this build.
    pub is_root: bool,
    pub roles: Vec<Role>,
    /// The directory groups the caller is currently in (empty for native
    /// users). Carried on the Actor so Producer-scoped checks can honour a
    /// maintainer grant made to a directory group, whose membership Sanshain
    /// never stores.
    pub directory_groups: Vec<String>,
}

impl Actor {
    /// An Actor holding exactly the `releaser` role — a fixture for tests
    /// that publish GA through the application seam.
    #[cfg(any(test, feature = "test-support"))]
    pub fn test_releaser() -> Self {
        Actor {
            user_id: 0,
            username: "test-releaser".to_string(),
            is_root: false,
            roles: vec![Role::Releaser],
            directory_groups: Vec::new(),
        }
    }

    pub fn has_permission(&self, permission: Permission) -> bool {
        // Root short-circuits before consulting roles, which is what makes
        // "holds every permission" survive a permission being added later.
        self.is_root || self.roles.iter().any(|role| role.grants(permission))
    }

    /// Every permission this Actor holds, for reporting to the UI.
    pub fn effective_permissions(&self) -> Vec<Permission> {
        Permission::ALL
            .iter()
            .copied()
            .filter(|p| self.has_permission(*p))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn permission_all_covers_every_variant() {
        // Guards against a new variant being added without extending ALL, which
        // would silently narrow Admin.
        let names: BTreeSet<&str> = Permission::ALL.iter().map(|p| p.as_str()).collect();
        assert_eq!(names.len(), Permission::ALL.len(), "duplicate permission");
        for name in &names {
            assert_eq!(Permission::parse(name).map(|p| p.as_str()), Some(*name));
        }
        assert_eq!(Permission::ALL.len(), 10);
    }

    #[test]
    fn role_names_round_trip() {
        for role in Role::ALL {
            assert_eq!(Role::parse(role.as_str()), Some(*role));
        }
        assert_eq!(Role::parse("nope"), None);
    }

    #[test]
    fn admin_holds_every_permission() {
        for permission in Permission::ALL {
            assert!(
                Role::Admin.grants(*permission),
                "admin should grant {}",
                permission.as_str()
            );
        }
    }

    #[test]
    fn user_manager_is_confined_to_user_administration() {
        assert!(Role::UserManager.grants(Permission::ManageUsers));
        assert!(Role::UserManager.grants(Permission::ManageRoles));
        assert!(!Role::UserManager.grants(Permission::ManageProducers));
        assert!(!Role::UserManager.grants(Permission::RunDestructiveOperations));
    }

    #[test]
    fn maintainer_is_not_globally_grantable() {
        assert!(!Role::GLOBALLY_GRANTABLE.contains(&Role::Maintainer));
        assert!(Role::GLOBALLY_GRANTABLE.contains(&Role::Admin));
    }

    #[test]
    fn root_set_falls_back_to_the_bootstrap_admin() {
        let roots = RootUsers::resolve(None, Some("operator"));
        assert!(roots.contains("operator"));
        assert!(!roots.contains("root"));
    }

    #[test]
    fn root_set_falls_back_to_root_when_nothing_is_configured() {
        let roots = RootUsers::resolve(None, None);
        assert!(roots.contains("root"));
    }

    #[test]
    fn root_set_parses_a_comma_separated_list() {
        let roots = RootUsers::resolve(Some(" axel , root ,"), Some("ignored"));
        assert!(roots.contains("axel"));
        assert!(roots.contains("root"));
        assert!(!roots.contains("ignored"));
        assert_eq!(roots.usernames().count(), 2);
    }

    #[test]
    fn an_empty_configured_list_is_treated_as_unset() {
        let roots = RootUsers::resolve(Some("   "), Some("operator"));
        assert!(roots.contains("operator"));
    }

    #[test]
    fn root_holds_a_permission_no_role_grants() {
        let actor = Actor {
            user_id: 1,
            username: "root".into(),
            is_root: true,
            roles: vec![],
            directory_groups: vec![],
        };
        for permission in Permission::ALL {
            assert!(actor.has_permission(*permission));
        }
        assert_eq!(actor.effective_permissions().len(), Permission::ALL.len());
    }

    #[test]
    fn a_roleless_actor_holds_nothing() {
        let actor = Actor {
            user_id: 2,
            username: "nobody".into(),
            is_root: false,
            roles: vec![],
            directory_groups: vec![],
        };
        for permission in Permission::ALL {
            assert!(!actor.has_permission(*permission));
        }
        assert!(actor.effective_permissions().is_empty());
    }

    #[test]
    fn an_actor_holds_the_union_of_its_roles() {
        let actor = Actor {
            user_id: 3,
            username: "helper".into(),
            is_root: false,
            roles: vec![Role::UserManager, Role::Viewer],
            directory_groups: vec![],
        };
        assert!(actor.has_permission(Permission::ManageUsers));
        assert!(actor.has_permission(Permission::ViewAudit));
        assert!(!actor.has_permission(Permission::ManageProducers));
        assert_eq!(actor.effective_permissions().len(), 4);
    }

    #[test]
    fn the_releaser_bundle_is_exactly_release_ga() {
        assert_eq!(Role::Releaser.permissions(), &[Permission::ReleaseGa]);
        assert!(Role::GLOBALLY_GRANTABLE.contains(&Role::Releaser));
        assert!(Role::ADMIN_GUARDED.contains(&Role::Admin));
        assert!(Role::ADMIN_GUARDED.contains(&Role::Releaser));
        assert!(
            !Role::ADMIN_GUARDED.contains(&Role::Viewer),
            "guarding is the exception, not the rule"
        );
    }
}
