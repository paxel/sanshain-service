//! Resolving what an authenticated caller may do.
//!
//! Kept apart from authentication: by the time anything here runs the caller's
//! identity is settled, and the only question left is which roles that identity
//! carries.

use crate::domain::models::{AppError, Group, GroupSource, User};
use crate::domain::permissions::{Actor, Permission, Role, RootUsers};
use crate::domain::ports::SpecRepository;

/// Build the [`Actor`] for an authenticated user.
///
/// Roles come from stored grants — direct, and via a group whose membership
/// Sanshain stores — unioned with those conferred by the directory groups the
/// user is currently in. There is no other source: the administrator flag it
/// replaced is gone, so this is the whole answer.
pub async fn resolve_actor(
    repo: &impl SpecRepository,
    user: &User,
    root_users: &RootUsers,
    directory_groups: &[String],
) -> Result<Actor, AppError> {
    // Deliberately propagated rather than defaulted. Treating an unreadable
    // grant table as "no roles" would silently strip every permission from
    // everyone but root the moment the database hiccuped, and the caller would
    // see an ordinary 403 with nothing to distinguish it from a real refusal.
    // Failing the request is louder and cannot be mistaken for a policy answer.
    let stored = repo.effective_stored_roles(user.id).await?;

    // Roles conferred by the directory groups this user is currently in. Read
    // live (behind a short cache) rather than copied at login, so a change in the
    // directory takes effect without the user signing in again.
    let from_directory =
        crate::application::directory_roles::roles_from_directory(repo, directory_groups).await?;

    let mut roles: Vec<Role> = stored
        .iter()
        .chain(from_directory.iter())
        .filter_map(|r| Role::parse(r))
        .collect();
    roles.sort();
    roles.dedup();

    Ok(Actor {
        user_id: user.id,
        username: user.username.clone(),
        is_root: root_users.contains(&user.username),
        roles,
        directory_groups: directory_groups.to_vec(),
    })
}

/// Grant a role to a user.
///
/// Only globally grantable roles are accepted: `maintainer` is a scope over a
/// set of Producers, not something held instance-wide, so granting it here would
/// confer its permissions everywhere.
/// Only an admin (or root) may hand out or take away an admin-guarded role
/// ([`Role::ADMIN_GUARDED`]): `admin`, because a `manage_roles` holder could
/// otherwise escalate themselves to every permission — and `releaser`,
/// because release rights are exactly what the GA gate exists to control.
fn require_admin_for_guarded_roles(actor: Option<&Actor>, roles: &[&str]) -> Result<(), AppError> {
    let touches_guarded = Role::ADMIN_GUARDED
        .iter()
        .any(|guarded| roles.contains(&guarded.as_str()));
    if !touches_guarded {
        return Ok(());
    }
    let allowed = actor.is_some_and(|a| a.is_root || a.roles.contains(&Role::Admin));
    if allowed {
        Ok(())
    } else {
        Err(AppError::Forbidden)
    }
}

pub async fn grant_user_role(
    repo: &impl SpecRepository,
    actor: Option<&Actor>,
    user_id: i64,
    role: &str,
) -> Result<(), AppError> {
    let parsed = parse_grantable_role(role)?;
    require_admin_for_guarded_roles(actor, &[parsed.as_str()])?;
    repo.grant_user_role(user_id, parsed.as_str()).await?;
    Ok(())
}

pub async fn revoke_user_role(
    repo: &impl SpecRepository,
    actor: Option<&Actor>,
    user_id: i64,
    role: &str,
) -> Result<bool, AppError> {
    let parsed = Role::parse(role).ok_or_else(|| AppError::BadRequest(unknown_role(role)))?;
    require_admin_for_guarded_roles(actor, &[parsed.as_str()])?;
    Ok(repo.revoke_user_role(user_id, parsed.as_str()).await?)
}

pub async fn list_user_roles(
    repo: &impl SpecRepository,
    user_id: i64,
) -> Result<Vec<String>, AppError> {
    Ok(repo.list_user_roles(user_id).await?)
}

/// A group with everything the management UI needs to render it.
#[derive(Debug, Clone, serde::Serialize)]
pub struct GroupDetail {
    pub id: i64,
    pub name: String,
    pub source: GroupSource,
    pub roles: Vec<String>,
    pub member_ids: Vec<i64>,
}

pub async fn list_groups(repo: &impl SpecRepository) -> Result<Vec<GroupDetail>, AppError> {
    let groups = repo.list_groups().await?;
    let mut out = Vec::with_capacity(groups.len());
    for group in groups {
        out.push(detail_for(repo, group).await?);
    }
    Ok(out)
}

async fn detail_for(repo: &impl SpecRepository, group: Group) -> Result<GroupDetail, AppError> {
    let roles = repo.list_group_roles(group.id).await?;
    // A directory-sourced group stores no membership, so this is empty by
    // construction rather than by accident.
    let member_ids = repo.list_group_member_ids(group.id).await?;
    Ok(GroupDetail {
        id: group.id,
        name: group.name,
        source: group.source,
        roles,
        member_ids,
    })
}

/// Create a Sanshain-owned group.
///
/// Directory-sourced groups are not created here: their existence follows from
/// the directory, so minting one by hand would produce a group nobody can ever
/// be a member of.
pub async fn create_native_group(
    repo: &impl SpecRepository,
    name: &str,
) -> Result<Group, AppError> {
    let name = validate_group_name(name)?;
    Ok(repo.create_group(&name, GroupSource::Native).await?)
}

pub async fn rename_group(
    repo: &impl SpecRepository,
    group_id: i64,
    name: &str,
) -> Result<(), AppError> {
    let group = require_group(repo, group_id).await?;
    if group.source != GroupSource::Native {
        return Err(AppError::BadRequest(
            "Only Sanshain's own groups can be renamed; a directory group's name is the directory's."
                .to_string(),
        ));
    }
    let name = validate_group_name(name)?;
    repo.rename_group(group_id, &name).await?;
    Ok(())
}

pub async fn delete_group(
    repo: &impl SpecRepository,
    actor: Option<&Actor>,
    group_id: i64,
) -> Result<bool, AppError> {
    // Deleting a guarded-role group strips admin/releaser from every member
    // at once — the bulk form of the removal that require_admin_for_guarded_group
    // already guards, so it gets the same guard.
    require_admin_for_guarded_group(repo, actor, group_id).await?;
    Ok(repo.delete_group(group_id).await?)
}

/// Replace a group's role grants.
///
/// Applies to groups of either source: attaching roles to a directory group is
/// exactly how directory membership comes to mean something in Sanshain.
pub async fn set_group_roles(
    repo: &impl SpecRepository,
    actor: Option<&Actor>,
    group_id: i64,
    roles: &[String],
) -> Result<(), AppError> {
    require_group(repo, group_id).await?;
    let mut parsed = Vec::with_capacity(roles.len());
    for role in roles {
        parsed.push(parse_grantable_role(role)?.as_str().to_string());
    }
    parsed.sort();
    parsed.dedup();
    // Adding a guarded role (admin, releaser) to the set — or stripping one
    // from a group that holds it — needs an admin behind it.
    let current = repo.list_group_roles(group_id).await?;
    let touched_guarded: Vec<&str> = Role::ADMIN_GUARDED
        .iter()
        .filter(|guarded| {
            parsed.iter().any(|r| r == guarded.as_str())
                != current.iter().any(|r| r == guarded.as_str())
        })
        .map(|g| g.as_str())
        .collect();
    require_admin_for_guarded_roles(actor, &touched_guarded)?;
    repo.set_group_roles(group_id, &parsed).await?;
    Ok(())
}

/// Membership of a group that holds a guarded role IS a grant of that role,
/// so it gets the same admin-guard as granting the role directly — without
/// this, joining a releaser-holding group would be the side door the direct
/// grant closes.
async fn require_admin_for_guarded_group(
    repo: &impl SpecRepository,
    actor: Option<&Actor>,
    group_id: i64,
) -> Result<(), AppError> {
    let roles = repo.list_group_roles(group_id).await?;
    let guarded: Vec<&str> = roles
        .iter()
        .map(String::as_str)
        .filter(|r| Role::ADMIN_GUARDED.iter().any(|g| g.as_str() == *r))
        .collect();
    require_admin_for_guarded_roles(actor, &guarded)
}

/// Deleting a user strips every role they hold at once — for an
/// `admin`/`releaser` holder that is the bulk form of the revocation
/// [`require_admin_for_guarded_roles`] guards, so it gets the same admin-guard.
/// Without this, deleting the account would be the side door the direct
/// revocation closes. Only stored roles (direct, and via stored groups) count:
/// directory-conferred roles belong to the directory, not the deleted record.
pub async fn require_admin_to_delete_user(
    repo: &impl SpecRepository,
    root_users: &RootUsers,
    actor: Option<&Actor>,
    user_id: i64,
) -> Result<(), AppError> {
    // A claimed root account is superuser by pure name match and deliberately
    // holds no stored roles, so the role check below cannot see it: destroying
    // it un-claims the reserved name (and locks root out until a restart),
    // which outranks any single role change. Same guard as touching `admin`.
    let target_is_root = repo
        .list_users()
        .await?
        .into_iter()
        .any(|u| u.id == user_id && root_users.contains(&u.username));
    if target_is_root {
        return require_admin_for_guarded_roles(actor, &[Role::Admin.as_str()]);
    }

    let stored = repo.effective_stored_roles(user_id).await?;
    let guarded: Vec<&str> = stored
        .iter()
        .map(String::as_str)
        .filter(|r| Role::ADMIN_GUARDED.iter().any(|g| g.as_str() == *r))
        .collect();
    require_admin_for_guarded_roles(actor, &guarded)
}

pub async fn add_group_member(
    repo: &impl SpecRepository,
    actor: Option<&Actor>,
    group_id: i64,
    user_id: i64,
) -> Result<(), AppError> {
    let group = require_native_group(repo, group_id).await?;
    require_admin_for_guarded_group(repo, actor, group.id).await?;
    repo.add_group_member(group.id, user_id).await?;
    Ok(())
}

pub async fn remove_group_member(
    repo: &impl SpecRepository,
    actor: Option<&Actor>,
    group_id: i64,
    user_id: i64,
) -> Result<bool, AppError> {
    let group = require_native_group(repo, group_id).await?;
    // Guarded in both directions: removal strips the member's admin/releaser
    // rights, which is as much an admin-role change as granting them.
    require_admin_for_guarded_group(repo, actor, group.id).await?;
    Ok(repo.remove_group_member(group.id, user_id).await?)
}

// --- Maintainer scope ---

/// Who maintains a Producer.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Maintainers {
    pub user_ids: Vec<i64>,
    pub group_ids: Vec<i64>,
}

/// Whether an Actor maintains a Producer.
///
/// Root maintains everything by the same short-circuit that gives it every
/// permission — otherwise the one account that cannot be locked out could still
/// be locked out of Producer-scoped actions.
pub async fn maintains_producer(
    repo: &impl SpecRepository,
    actor: &Actor,
    producer: &str,
) -> Result<bool, AppError> {
    if actor.is_root {
        return Ok(true);
    }
    let Some(service_id) = repo.find_service(producer).await? else {
        return Ok(false);
    };
    if repo.maintains_producer(actor.user_id, service_id).await? {
        return Ok(true);
    }
    // A maintainer grant to a *directory* group has no stored membership to
    // match against — check it against the groups the directory says the
    // caller is in right now.
    let directory_group_ids = directory_maintainer_group_ids(repo, actor).await?;
    if directory_group_ids.is_empty() {
        return Ok(false);
    }
    let maintainer_groups = repo.list_group_maintainer_ids(service_id).await?;
    Ok(maintainer_groups
        .iter()
        .any(|id| directory_group_ids.contains(id)))
}

/// The stored ids of the LDAP-source groups the Actor is currently in,
/// according to the directory. Empty for native users, and for directory
/// groups Sanshain has never been told about.
async fn directory_maintainer_group_ids(
    repo: &impl SpecRepository,
    actor: &Actor,
) -> Result<Vec<i64>, AppError> {
    if actor.directory_groups.is_empty() {
        return Ok(Vec::new());
    }
    Ok(repo
        .list_groups()
        .await?
        .into_iter()
        .filter(|g| g.source == GroupSource::Ldap && actor.directory_groups.contains(&g.name))
        .map(|g| g.id)
        .collect())
}

/// Authorise a Producer-scoped action.
///
/// Passes if the Actor holds `permission` instance-wide, **or** maintains this
/// Producer and the maintainer bundle includes it. The second arm is what makes
/// "responsible for a bunch of services" mean something: the same permission is
/// granted, but only over the Producers the Actor is actually responsible for.
pub async fn require_producer_permission(
    repo: &impl SpecRepository,
    actor: &Actor,
    permission: Permission,
    producer: &str,
) -> Result<(), AppError> {
    if actor.has_permission(permission) {
        return Ok(());
    }
    if Role::Maintainer.grants(permission) && maintains_producer(repo, actor, producer).await? {
        return Ok(());
    }
    Err(AppError::Forbidden)
}

/// One Producer's maintainer sets, named, for the aggregate listing.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ProducerMaintainers {
    pub producer: String,
    pub user_ids: Vec<i64>,
    pub group_ids: Vec<i64>,
}

/// Who maintains what, across every Producer, in one answer.
///
/// Exists for the dashboard: filling its Maintainers section by asking per
/// Producer meant one HTTP round-trip each — 26 requests for 25 Producers,
/// sequentially. Producers with no maintainers are included, so the same
/// payload also populates the assignment picker.
pub async fn all_maintainers(
    repo: &impl SpecRepository,
) -> Result<Vec<ProducerMaintainers>, AppError> {
    // Three queries total, however many Producers there are — asking per
    // Producer made this 3N+1 and the dashboard paid it on every load.
    let mut by_producer: std::collections::HashMap<String, ProducerMaintainers> = repo
        .list_producers()
        .await?
        .into_iter()
        .map(|producer| {
            (
                producer.clone(),
                ProducerMaintainers {
                    producer,
                    user_ids: Vec::new(),
                    group_ids: Vec::new(),
                },
            )
        })
        .collect();
    for (producer, user_id) in repo.list_all_user_maintainers().await? {
        if let Some(entry) = by_producer.get_mut(&producer) {
            entry.user_ids.push(user_id);
        }
    }
    for (producer, group_id) in repo.list_all_group_maintainers().await? {
        if let Some(entry) = by_producer.get_mut(&producer) {
            entry.group_ids.push(group_id);
        }
    }
    let mut out: Vec<ProducerMaintainers> = by_producer.into_values().collect();
    out.sort_by(|a, b| a.producer.cmp(&b.producer));
    Ok(out)
}

pub async fn list_maintainers(
    repo: &impl SpecRepository,
    producer: &str,
) -> Result<Maintainers, AppError> {
    let service_id = require_producer(repo, producer).await?;
    Ok(Maintainers {
        user_ids: repo.list_user_maintainer_ids(service_id).await?,
        group_ids: repo.list_group_maintainer_ids(service_id).await?,
    })
}

pub async fn assign_user_maintainer(
    repo: &impl SpecRepository,
    producer: &str,
    user_id: i64,
) -> Result<(), AppError> {
    let service_id = require_producer(repo, producer).await?;
    repo.add_user_maintainer(service_id, user_id).await?;
    Ok(())
}

pub async fn unassign_user_maintainer(
    repo: &impl SpecRepository,
    producer: &str,
    user_id: i64,
) -> Result<bool, AppError> {
    let service_id = require_producer(repo, producer).await?;
    Ok(repo.remove_user_maintainer(service_id, user_id).await?)
}

pub async fn assign_group_maintainer(
    repo: &impl SpecRepository,
    producer: &str,
    group_id: i64,
) -> Result<(), AppError> {
    let service_id = require_producer(repo, producer).await?;
    require_group(repo, group_id).await?;
    repo.add_group_maintainer(service_id, group_id).await?;
    Ok(())
}

pub async fn unassign_group_maintainer(
    repo: &impl SpecRepository,
    producer: &str,
    group_id: i64,
) -> Result<bool, AppError> {
    let service_id = require_producer(repo, producer).await?;
    Ok(repo.remove_group_maintainer(service_id, group_id).await?)
}

/// The Producers an Actor is responsible for.
///
/// Root is not expanded to every Producer here: the list answers "what has been
/// assigned", and inventing assignments for root would misreport the record.
/// The Producers the *caller* maintains — stored grants plus grants made to
/// the directory groups they are currently in. This is what `/auth/me`
/// reports, so the UI's gating matches what `require_producer_permission`
/// would actually allow.
pub async fn maintained_producers_for_actor(
    repo: &impl SpecRepository,
    actor: &Actor,
) -> Result<Vec<String>, AppError> {
    let mut producers = repo.list_maintained_producers(actor.user_id).await?;
    let directory_group_ids = directory_maintainer_group_ids(repo, actor).await?;
    if !directory_group_ids.is_empty() {
        for producer in repo
            .list_group_maintained_producers(&directory_group_ids)
            .await?
        {
            if !producers.contains(&producer) {
                producers.push(producer);
            }
        }
        producers.sort();
    }
    Ok(producers)
}

pub async fn maintained_producers(
    repo: &impl SpecRepository,
    user_id: i64,
) -> Result<Vec<String>, AppError> {
    Ok(repo.list_maintained_producers(user_id).await?)
}

async fn require_producer(repo: &impl SpecRepository, producer: &str) -> Result<i64, AppError> {
    repo.find_service(producer)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("Producer '{}' not found", producer)))
}

/// Permissions an Actor holds, as strings, for reporting to the UI.
pub fn permission_names(actor: &Actor) -> Vec<String> {
    actor
        .effective_permissions()
        .into_iter()
        .map(|p| p.as_str().to_string())
        .collect()
}

/// Refuse the request unless the Actor holds `permission`.
pub fn require_permission(actor: &Actor, permission: Permission) -> Result<(), AppError> {
    if actor.has_permission(permission) {
        Ok(())
    } else {
        Err(AppError::Forbidden)
    }
}

fn parse_grantable_role(role: &str) -> Result<Role, AppError> {
    let parsed = Role::parse(role).ok_or_else(|| AppError::BadRequest(unknown_role(role)))?;
    if !Role::GLOBALLY_GRANTABLE.contains(&parsed) {
        return Err(AppError::BadRequest(format!(
            "Role '{}' is scoped to specific Producers and cannot be granted instance-wide.",
            parsed.as_str()
        )));
    }
    Ok(parsed)
}

fn unknown_role(role: &str) -> String {
    format!(
        "Unknown role '{}'. Known roles: {}.",
        role,
        Role::ALL
            .iter()
            .map(|r| r.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    )
}

fn validate_group_name(name: &str) -> Result<String, AppError> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err(AppError::BadRequest(
            "Group name must not be empty.".to_string(),
        ));
    }
    Ok(trimmed.to_string())
}

async fn require_group(repo: &impl SpecRepository, group_id: i64) -> Result<Group, AppError> {
    repo.list_groups()
        .await?
        .into_iter()
        .find(|g| g.id == group_id)
        .ok_or_else(|| AppError::NotFound(format!("Group {} not found", group_id)))
}

async fn require_native_group(
    repo: &impl SpecRepository,
    group_id: i64,
) -> Result<Group, AppError> {
    let group = require_group(repo, group_id).await?;
    if group.source != GroupSource::Native {
        return Err(AppError::BadRequest(format!(
            "Group '{}' comes from the directory, so its membership is the directory's to change.",
            group.name
        )));
    }
    Ok(group)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::mock_repo::MockRepo;

    /// An acting administrator for calls whose admin-role guard is not what
    /// the test is about.
    fn admin_actor() -> Actor {
        Actor {
            user_id: 999,
            username: "acting-admin".into(),
            is_root: false,
            roles: vec![Role::Admin],
            directory_groups: vec![],
        }
    }

    async fn user(repo: &MockRepo, username: &str, admin: bool) -> User {
        let user = repo
            .create_user(username, "hash", true)
            .await
            .expect("user should be created");
        if admin {
            repo.grant_user_role(user.id, "admin")
                .await
                .expect("admin grant");
        }
        user
    }

    fn roots(names: &str) -> RootUsers {
        RootUsers::resolve(Some(names), None)
    }

    #[tokio::test]
    async fn an_administrator_still_resolves_to_the_admin_role() {
        let repo = MockRepo::new();
        let alice = user(&repo, "alice", true).await;
        let actor = resolve_actor(&repo, &alice, &roots("root"), &[])
            .await
            .expect("roles resolve");
        assert_eq!(actor.roles, vec![Role::Admin]);
        assert!(actor.has_permission(Permission::ManageProducers));
    }

    #[tokio::test]
    async fn a_granted_role_takes_effect() {
        let repo = MockRepo::new();
        let bob = user(&repo, "bob", false).await;
        grant_user_role(&repo, Some(&admin_actor()), bob.id, "user_manager")
            .await
            .expect("grant should succeed");

        let actor = resolve_actor(&repo, &bob, &roots("root"), &[])
            .await
            .expect("roles resolve");
        assert_eq!(actor.roles, vec![Role::UserManager]);
        assert!(actor.has_permission(Permission::ManageUsers));
        assert!(!actor.has_permission(Permission::ManageProducers));
    }

    #[tokio::test]
    async fn revoking_a_role_removes_the_permission() {
        let repo = MockRepo::new();
        let bob = user(&repo, "bob", false).await;
        grant_user_role(&repo, Some(&admin_actor()), bob.id, "user_manager")
            .await
            .expect("grant should succeed");
        assert!(
            revoke_user_role(&repo, Some(&admin_actor()), bob.id, "user_manager")
                .await
                .expect("revoke should succeed")
        );

        let actor = resolve_actor(&repo, &bob, &roots("root"), &[])
            .await
            .expect("roles resolve");
        assert!(actor.roles.is_empty());
        assert!(!actor.has_permission(Permission::ManageUsers));
    }

    #[tokio::test]
    async fn a_role_arrives_through_native_group_membership() {
        let repo = MockRepo::new();
        let carol = user(&repo, "carol", false).await;
        let group = create_native_group(&repo, "platform")
            .await
            .expect("group should be created");
        set_group_roles(
            &repo,
            Some(&admin_actor()),
            group.id,
            &["viewer".to_string()],
        )
        .await
        .expect("roles should be set");
        add_group_member(&repo, Some(&admin_actor()), group.id, carol.id)
            .await
            .expect("member should be added");

        let actor = resolve_actor(&repo, &carol, &roots("root"), &[])
            .await
            .expect("roles resolve");
        assert_eq!(actor.roles, vec![Role::Viewer]);
        assert!(actor.has_permission(Permission::ViewAudit));
    }

    #[tokio::test]
    async fn removing_a_member_removes_the_role_they_held_through_the_group() {
        let repo = MockRepo::new();
        let carol = user(&repo, "carol", false).await;
        let group = create_native_group(&repo, "platform")
            .await
            .expect("group should be created");
        set_group_roles(
            &repo,
            Some(&admin_actor()),
            group.id,
            &["viewer".to_string()],
        )
        .await
        .expect("roles should be set");
        add_group_member(&repo, Some(&admin_actor()), group.id, carol.id)
            .await
            .expect("member should be added");
        assert!(
            remove_group_member(&repo, Some(&admin_actor()), group.id, carol.id)
                .await
                .expect("removal should succeed")
        );

        let actor = resolve_actor(&repo, &carol, &roots("root"), &[])
            .await
            .expect("roles resolve");
        assert!(actor.roles.is_empty());
    }

    #[tokio::test]
    async fn roles_from_grants_and_groups_are_unioned() {
        let repo = MockRepo::new();
        let dave = user(&repo, "dave", false).await;
        grant_user_role(&repo, Some(&admin_actor()), dave.id, "user_manager")
            .await
            .expect("grant should succeed");
        let group = create_native_group(&repo, "readers")
            .await
            .expect("group should be created");
        set_group_roles(
            &repo,
            Some(&admin_actor()),
            group.id,
            &["viewer".to_string()],
        )
        .await
        .expect("roles should be set");
        add_group_member(&repo, Some(&admin_actor()), group.id, dave.id)
            .await
            .expect("member should be added");

        let actor = resolve_actor(&repo, &dave, &roots("root"), &[])
            .await
            .expect("roles resolve");
        assert_eq!(actor.roles, vec![Role::UserManager, Role::Viewer]);
        assert!(actor.has_permission(Permission::ManageUsers));
        assert!(actor.has_permission(Permission::ViewAudit));
    }

    #[tokio::test]
    async fn root_is_root_without_any_grant() {
        let repo = MockRepo::new();
        let breakglass = user(&repo, "breakglass", false).await;
        let actor = resolve_actor(&repo, &breakglass, &roots("breakglass"), &[])
            .await
            .expect("roles resolve");
        assert!(actor.is_root);
        assert!(actor.roles.is_empty());
        for permission in Permission::ALL {
            assert!(actor.has_permission(*permission));
        }
    }

    #[tokio::test]
    async fn maintainer_cannot_be_granted_instance_wide() {
        let repo = MockRepo::new();
        let eve = user(&repo, "eve", false).await;
        let err = grant_user_role(&repo, Some(&admin_actor()), eve.id, "maintainer")
            .await
            .expect_err("maintainer should be refused");
        assert!(matches!(err, AppError::BadRequest(_)));
    }

    #[tokio::test]
    async fn an_unknown_role_is_refused() {
        let repo = MockRepo::new();
        let eve = user(&repo, "eve", false).await;
        let err = grant_user_role(&repo, Some(&admin_actor()), eve.id, "wizard")
            .await
            .expect_err("unknown role should be refused");
        assert!(matches!(err, AppError::BadRequest(_)));
    }

    #[tokio::test]
    async fn a_directory_group_refuses_membership_edits() {
        let repo = MockRepo::new();
        let frank = user(&repo, "frank", false).await;
        let group = repo
            .create_group("ad-admins", GroupSource::Ldap)
            .await
            .expect("group should be created");

        let err = add_group_member(&repo, Some(&admin_actor()), group.id, frank.id)
            .await
            .expect_err("directory membership should be refused");
        assert!(matches!(err, AppError::BadRequest(_)));

        // Attaching roles to it is still allowed — that is the whole point.
        set_group_roles(
            &repo,
            Some(&admin_actor()),
            group.id,
            &["admin".to_string()],
        )
        .await
        .expect("roles should attach to a directory group");
        assert_eq!(
            repo.list_group_roles(group.id)
                .await
                .expect("roles should be readable"),
            vec!["admin".to_string()]
        );
    }

    #[tokio::test]
    async fn a_native_and_a_directory_group_may_share_a_name() {
        let repo = MockRepo::new();
        let native = create_native_group(&repo, "developers")
            .await
            .expect("native group should be created");
        let directory = repo
            .create_group("developers", GroupSource::Ldap)
            .await
            .expect("directory group should be created");
        assert_ne!(native.id, directory.id);
        assert_eq!(
            list_groups(&repo)
                .await
                .expect("groups should be listable")
                .len(),
            2
        );
    }

    #[tokio::test]
    async fn creating_the_same_group_twice_returns_the_existing_one() {
        let repo = MockRepo::new();
        let first = create_native_group(&repo, "platform")
            .await
            .expect("first create");
        let second = create_native_group(&repo, " platform ")
            .await
            .expect("second create");
        assert_eq!(first.id, second.id);
    }

    #[tokio::test]
    async fn deleting_a_group_takes_its_roles_with_it() {
        let repo = MockRepo::new();
        let gail = user(&repo, "gail", false).await;
        let group = create_native_group(&repo, "temp")
            .await
            .expect("group should be created");
        set_group_roles(
            &repo,
            Some(&admin_actor()),
            group.id,
            &["viewer".to_string()],
        )
        .await
        .expect("roles should be set");
        add_group_member(&repo, Some(&admin_actor()), group.id, gail.id)
            .await
            .expect("member should be added");

        assert!(
            delete_group(&repo, Some(&admin_actor()), group.id)
                .await
                .expect("delete should succeed")
        );
        let actor = resolve_actor(&repo, &gail, &roots("root"), &[])
            .await
            .expect("roles resolve");
        assert!(actor.roles.is_empty());
    }

    #[tokio::test]
    async fn require_permission_refuses_an_actor_without_it() {
        let actor = Actor {
            user_id: 1,
            username: "nobody".into(),
            is_root: false,
            roles: vec![],
            directory_groups: vec![],
        };
        assert!(matches!(
            require_permission(&actor, Permission::ManageUsers),
            Err(AppError::Forbidden)
        ));

        let admin = Actor {
            user_id: 2,
            username: "admin".into(),
            is_root: false,
            roles: vec![Role::Admin],
            directory_groups: vec![],
        };
        assert!(require_permission(&admin, Permission::ManageUsers).is_ok());
    }

    #[tokio::test]
    async fn group_detail_reports_roles_and_members() {
        let repo = MockRepo::new();
        let hana = user(&repo, "hana", false).await;
        let group = create_native_group(&repo, "platform")
            .await
            .expect("group should be created");
        set_group_roles(
            &repo,
            Some(&admin_actor()),
            group.id,
            &["viewer".to_string(), "admin".to_string()],
        )
        .await
        .expect("roles should be set");
        add_group_member(&repo, Some(&admin_actor()), group.id, hana.id)
            .await
            .expect("member should be added");

        let groups = list_groups(&repo).await.expect("groups should be listable");
        let detail = groups
            .iter()
            .find(|g| g.id == group.id)
            .expect("group should be listed");
        assert_eq!(
            detail.roles,
            vec!["admin".to_string(), "viewer".to_string()]
        );
        assert_eq!(detail.member_ids, vec![hana.id]);
        assert_eq!(detail.source, GroupSource::Native);
    }
    // --- Maintainer scope ---

    #[tokio::test]
    async fn a_maintainer_is_authorised_only_for_producers_they_maintain() {
        let repo = MockRepo::new();
        let ida = user(&repo, "ida", false).await;
        repo.ensure_service("orders").await.expect("producer");
        repo.ensure_service("billing").await.expect("producer");
        assign_user_maintainer(&repo, "orders", ida.id)
            .await
            .expect("assignment");

        let actor = resolve_actor(&repo, &ida, &roots("root"), &[])
            .await
            .expect("roles resolve");
        assert!(
            require_producer_permission(&repo, &actor, Permission::ManageProducers, "orders")
                .await
                .is_ok()
        );
        assert!(matches!(
            require_producer_permission(&repo, &actor, Permission::ManageProducers, "billing")
                .await,
            Err(AppError::Forbidden)
        ));
    }

    /// Maintainership confers the maintainer bundle and nothing beyond it.
    #[tokio::test]
    async fn maintainership_does_not_confer_unrelated_permissions() {
        let repo = MockRepo::new();
        let ida = user(&repo, "ida", false).await;
        repo.ensure_service("orders").await.expect("producer");
        assign_user_maintainer(&repo, "orders", ida.id)
            .await
            .expect("assignment");

        let actor = resolve_actor(&repo, &ida, &roots("root"), &[])
            .await
            .expect("roles resolve");
        assert!(matches!(
            require_producer_permission(&repo, &actor, Permission::ManageUsers, "orders").await,
            Err(AppError::Forbidden)
        ));
        assert!(!actor.has_permission(Permission::ManageProducers));
    }

    #[tokio::test]
    async fn maintainership_arrives_through_group_membership() {
        let repo = MockRepo::new();
        let jon = user(&repo, "jon", false).await;
        repo.ensure_service("orders").await.expect("producer");
        let group = create_native_group(&repo, "platform").await.expect("group");
        add_group_member(&repo, Some(&admin_actor()), group.id, jon.id)
            .await
            .expect("member");
        assign_group_maintainer(&repo, "orders", group.id)
            .await
            .expect("assignment");

        let actor = resolve_actor(&repo, &jon, &roots("root"), &[])
            .await
            .expect("roles resolve");
        assert!(
            maintains_producer(&repo, &actor, "orders")
                .await
                .expect("check")
        );
        assert!(
            require_producer_permission(&repo, &actor, Permission::ManageProducers, "orders")
                .await
                .is_ok()
        );
    }

    #[tokio::test]
    async fn an_admin_passes_a_producer_scoped_check_without_maintaining_it() {
        let repo = MockRepo::new();
        let kim = user(&repo, "kim", true).await;
        repo.ensure_service("orders").await.expect("producer");

        let actor = resolve_actor(&repo, &kim, &roots("root"), &[])
            .await
            .expect("roles resolve");
        assert!(
            !maintains_producer(&repo, &actor, "orders")
                .await
                .expect("check")
        );
        assert!(
            require_producer_permission(&repo, &actor, Permission::ManageProducers, "orders")
                .await
                .is_ok()
        );
    }

    #[tokio::test]
    async fn root_maintains_every_producer() {
        let repo = MockRepo::new();
        let breakglass = user(&repo, "breakglass", false).await;
        repo.ensure_service("orders").await.expect("producer");

        let actor = resolve_actor(&repo, &breakglass, &roots("breakglass"), &[])
            .await
            .expect("roles resolve");
        assert!(
            maintains_producer(&repo, &actor, "orders")
                .await
                .expect("check")
        );
        assert!(
            require_producer_permission(&repo, &actor, Permission::ManageProducers, "orders")
                .await
                .is_ok()
        );
    }

    #[tokio::test]
    async fn unassigning_removes_the_scope() {
        let repo = MockRepo::new();
        let ida = user(&repo, "ida", false).await;
        repo.ensure_service("orders").await.expect("producer");
        assign_user_maintainer(&repo, "orders", ida.id)
            .await
            .expect("assignment");
        assert!(
            unassign_user_maintainer(&repo, "orders", ida.id)
                .await
                .expect("unassignment")
        );
        assert!(
            !unassign_user_maintainer(&repo, "orders", ida.id)
                .await
                .expect("second unassignment reports no change")
        );

        let actor = resolve_actor(&repo, &ida, &roots("root"), &[])
            .await
            .expect("roles resolve");
        assert!(
            !maintains_producer(&repo, &actor, "orders")
                .await
                .expect("check")
        );
    }

    #[tokio::test]
    async fn maintained_producers_lists_direct_and_group_assignments() {
        let repo = MockRepo::new();
        let ida = user(&repo, "ida", false).await;
        repo.ensure_service("orders").await.expect("producer");
        repo.ensure_service("shipping").await.expect("producer");
        repo.ensure_service("billing").await.expect("producer");
        assign_user_maintainer(&repo, "orders", ida.id)
            .await
            .expect("assignment");
        let group = create_native_group(&repo, "platform").await.expect("group");
        add_group_member(&repo, Some(&admin_actor()), group.id, ida.id)
            .await
            .expect("member");
        assign_group_maintainer(&repo, "shipping", group.id)
            .await
            .expect("assignment");

        let mut producers = maintained_producers(&repo, ida.id).await.expect("list");
        producers.sort();
        assert_eq!(
            producers,
            vec!["orders".to_string(), "shipping".to_string()]
        );
    }

    #[tokio::test]
    async fn listing_maintainers_reports_both_kinds() {
        let repo = MockRepo::new();
        let ida = user(&repo, "ida", false).await;
        repo.ensure_service("orders").await.expect("producer");
        let group = create_native_group(&repo, "platform").await.expect("group");
        assign_user_maintainer(&repo, "orders", ida.id)
            .await
            .expect("assignment");
        assign_group_maintainer(&repo, "orders", group.id)
            .await
            .expect("assignment");

        let maintainers = list_maintainers(&repo, "orders").await.expect("list");
        assert_eq!(maintainers.user_ids, vec![ida.id]);
        assert_eq!(maintainers.group_ids, vec![group.id]);
    }

    #[tokio::test]
    async fn assigning_to_an_unknown_producer_is_not_found() {
        let repo = MockRepo::new();
        let ida = user(&repo, "ida", false).await;
        assert!(matches!(
            assign_user_maintainer(&repo, "nope", ida.id).await,
            Err(AppError::NotFound(_))
        ));
    }

    /// Releaser is admin-guarded for the same reason admin is: the GA gate
    /// exists to control release rights, so `manage_roles` must not be a side
    /// door to them.
    #[tokio::test]
    async fn a_user_manager_cannot_grant_or_revoke_releaser() {
        let repo = MockRepo::new();
        let bob = user(&repo, "bob", false).await;
        let user_manager = Actor {
            user_id: 500,
            username: "manager".into(),
            is_root: false,
            roles: vec![Role::UserManager],
            directory_groups: vec![],
        };

        assert!(matches!(
            grant_user_role(&repo, Some(&user_manager), bob.id, "releaser").await,
            Err(AppError::Forbidden)
        ));
        grant_user_role(&repo, Some(&admin_actor()), bob.id, "releaser")
            .await
            .expect("an admin may");
        assert!(matches!(
            revoke_user_role(&repo, Some(&user_manager), bob.id, "releaser").await,
            Err(AppError::Forbidden)
        ));
    }

    /// The escalation the admin-role guard closes: `manage_roles` alone must
    /// not be enough to hand out — or take away — the admin role itself.
    #[tokio::test]
    async fn a_user_manager_cannot_grant_or_revoke_admin() {
        let repo = MockRepo::new();
        let bob = user(&repo, "bob", false).await;
        let user_manager = Actor {
            user_id: 500,
            username: "manager".into(),
            is_root: false,
            roles: vec![Role::UserManager],
            directory_groups: vec![],
        };

        assert!(matches!(
            grant_user_role(&repo, Some(&user_manager), bob.id, "admin").await,
            Err(AppError::Forbidden)
        ));
        let victim = user(&repo, "carol", true).await;
        assert!(matches!(
            revoke_user_role(&repo, Some(&user_manager), victim.id, "admin").await,
            Err(AppError::Forbidden)
        ));
        // Non-admin roles stay within `manage_roles`.
        grant_user_role(&repo, Some(&user_manager), bob.id, "viewer")
            .await
            .expect("a non-admin role is theirs to grant");
    }

    #[tokio::test]
    async fn only_an_admin_may_put_admin_on_a_group_or_strip_it() {
        let repo = MockRepo::new();
        let group = create_native_group(&repo, "ops").await.expect("group");
        let user_manager = Actor {
            user_id: 500,
            username: "manager".into(),
            is_root: false,
            roles: vec![Role::UserManager],
            directory_groups: vec![],
        };

        assert!(matches!(
            set_group_roles(&repo, Some(&user_manager), group.id, &["admin".to_string()]).await,
            Err(AppError::Forbidden)
        ));
        set_group_roles(
            &repo,
            Some(&admin_actor()),
            group.id,
            &["admin".to_string()],
        )
        .await
        .expect("an admin may");
        assert!(matches!(
            set_group_roles(&repo, Some(&user_manager), group.id, &[]).await,
            Err(AppError::Forbidden)
        ));
        // A role set that does not touch admin needs no admin behind it.
        set_group_roles(
            &repo,
            Some(&user_manager),
            group.id,
            &["admin".to_string(), "viewer".to_string()],
        )
        .await
        .expect("admin kept, viewer added — not an admin-role change");
    }

    /// The side door the direct-grant guard would otherwise leave open:
    /// membership of a guarded-role group IS the grant.
    #[tokio::test]
    async fn joining_a_guarded_role_group_requires_an_admin() {
        let repo = MockRepo::new();
        let bob = user(&repo, "bob", false).await;
        let group = create_native_group(&repo, "releasers")
            .await
            .expect("group");
        set_group_roles(
            &repo,
            Some(&admin_actor()),
            group.id,
            &["releaser".to_string()],
        )
        .await
        .expect("roles");
        let user_manager = Actor {
            user_id: 500,
            username: "manager".into(),
            is_root: false,
            roles: vec![Role::UserManager],
            directory_groups: vec![],
        };

        assert!(matches!(
            add_group_member(&repo, Some(&user_manager), group.id, bob.id).await,
            Err(AppError::Forbidden)
        ));
        add_group_member(&repo, Some(&admin_actor()), group.id, bob.id)
            .await
            .expect("an admin may");
        assert!(matches!(
            remove_group_member(&repo, Some(&user_manager), group.id, bob.id).await,
            Err(AppError::Forbidden)
        ));

        // Plain groups stay within manage_roles.
        let plain = create_native_group(&repo, "viewers").await.expect("group");
        set_group_roles(
            &repo,
            Some(&user_manager),
            plain.id,
            &["viewer".to_string()],
        )
        .await
        .expect("viewer roles need no admin");
        add_group_member(&repo, Some(&user_manager), plain.id, bob.id)
            .await
            .expect("membership of an unguarded group needs no admin");
    }

    /// The bulk side door of the revocation guard: deleting an account strips
    /// every admin/releaser role it holds, so it needs an admin behind it too.
    #[tokio::test]
    async fn deleting_a_guarded_role_holder_requires_an_admin() {
        let repo = MockRepo::new();
        let bob = user(&repo, "bob", false).await;
        grant_user_role(&repo, Some(&admin_actor()), bob.id, "releaser")
            .await
            .expect("grant");
        let user_manager = Actor {
            user_id: 500,
            username: "manager".into(),
            is_root: false,
            roles: vec![Role::UserManager],
            directory_groups: vec![],
        };

        assert!(matches!(
            require_admin_to_delete_user(&repo, &roots("root"), Some(&user_manager), bob.id).await,
            Err(AppError::Forbidden)
        ));
        require_admin_to_delete_user(&repo, &roots("root"), Some(&admin_actor()), bob.id)
            .await
            .expect("an admin may");

        // Guarded via stored group membership counts the same as a direct grant.
        let carol = user(&repo, "carol", false).await;
        let group = create_native_group(&repo, "admins").await.expect("group");
        set_group_roles(
            &repo,
            Some(&admin_actor()),
            group.id,
            &["admin".to_string()],
        )
        .await
        .expect("roles");
        add_group_member(&repo, Some(&admin_actor()), group.id, carol.id)
            .await
            .expect("member");
        assert!(matches!(
            require_admin_to_delete_user(&repo, &roots("root"), Some(&user_manager), carol.id)
                .await,
            Err(AppError::Forbidden)
        ));

        // A claimed root account holds no stored roles at all — the name alone
        // must trip the guard.
        let ops_root = user(&repo, "ops-root", false).await;
        assert!(matches!(
            require_admin_to_delete_user(
                &repo,
                &roots("ops-root"),
                Some(&user_manager),
                ops_root.id
            )
            .await,
            Err(AppError::Forbidden)
        ));
        require_admin_to_delete_user(&repo, &roots("ops-root"), Some(&admin_actor()), ops_root.id)
            .await
            .expect("an admin may");

        // A plain user stays within manage_users.
        let dave = user(&repo, "dave", false).await;
        require_admin_to_delete_user(&repo, &roots("root"), Some(&user_manager), dave.id)
            .await
            .expect("no guarded role — no admin needed");
    }

    /// A maintainer grant to a *directory* group has no stored membership;
    /// it must match against the groups the directory says the caller is in.
    #[tokio::test]
    async fn a_directory_group_maintainer_grant_applies_to_its_members() {
        let repo = MockRepo::new();
        repo.ensure_service("orders").await.expect("producer");
        let group = repo
            .create_group("cn=team-orders", GroupSource::Ldap)
            .await
            .expect("group");
        assign_group_maintainer(&repo, "orders", group.id)
            .await
            .expect("assignment");

        let member = Actor {
            user_id: 7,
            username: "ldap-user".into(),
            is_root: false,
            roles: vec![],
            directory_groups: vec!["cn=team-orders".to_string()],
        };
        assert!(
            maintains_producer(&repo, &member, "orders")
                .await
                .expect("check"),
            "directory membership must satisfy the group grant"
        );
        assert_eq!(
            maintained_producers_for_actor(&repo, &member)
                .await
                .expect("list"),
            vec!["orders".to_string()]
        );

        let outsider = Actor {
            user_id: 8,
            username: "other".into(),
            is_root: false,
            roles: vec![],
            directory_groups: vec!["cn=unrelated".to_string()],
        };
        assert!(
            !maintains_producer(&repo, &outsider, "orders")
                .await
                .expect("check")
        );
    }

    /// The aggregate listing after batching: same answer, three queries.
    #[tokio::test]
    async fn all_maintainers_reports_every_producer_with_its_assignments() {
        let repo = MockRepo::new();
        let ida = user(&repo, "ida", false).await;
        repo.ensure_service("orders").await.expect("producer");
        repo.ensure_service("empty").await.expect("producer");
        assign_user_maintainer(&repo, "orders", ida.id)
            .await
            .expect("assignment");
        let group = create_native_group(&repo, "platform").await.expect("group");
        assign_group_maintainer(&repo, "orders", group.id)
            .await
            .expect("assignment");

        let all = all_maintainers(&repo).await.expect("aggregate");
        assert_eq!(all.len(), 2);
        let orders = all
            .iter()
            .find(|m| m.producer == "orders")
            .expect("orders entry");
        assert_eq!(orders.user_ids, vec![ida.id]);
        assert_eq!(orders.group_ids, vec![group.id]);
        let empty = all
            .iter()
            .find(|m| m.producer == "empty")
            .expect("producers with no maintainers are included");
        assert!(empty.user_ids.is_empty() && empty.group_ids.is_empty());
    }
}
