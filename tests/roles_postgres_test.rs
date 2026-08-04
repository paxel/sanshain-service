//! Role and group storage against a real PostgreSQL server.
//!
//! The SQLite suite proves the behaviour; this proves the Postgres dialect. The
//! two backends differ exactly where these methods are least alike — `ON
//! CONFLICT ... DO NOTHING` in place of `INSERT OR IGNORE`, numbered rather than
//! positional placeholders, and `BIGSERIAL` keys — so compiling is not evidence
//! that any of it runs.

use sanshain_service::domain::models::GroupSource;
use sanshain_service::domain::ports::SpecRepository;
use sanshain_service::infrastructure::postgres_repository::PostgresSpecRepository;
use sqlx::postgres::PgPoolOptions;
use testcontainers::runners::AsyncRunner;
use testcontainers_modules::postgres::Postgres;

// `cfg(test)` is always true in this crate; the attribute marks the helper as
// test code so clippy's `allow-*-in-tests` exemptions apply to it.
#[cfg(test)]
async fn repo_and_container() -> (
    PostgresSpecRepository,
    testcontainers::ContainerAsync<Postgres>,
) {
    let container = Postgres::default().start().await.expect("container starts");
    let host = container.get_host().await.expect("host");
    let port = container.get_host_port_ipv4(5432).await.expect("port");
    let db_url = format!("postgres://postgres:postgres@{}:{}/postgres", host, port);

    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&db_url)
        .await
        .expect("connects");
    let repo = PostgresSpecRepository::new(pool);
    repo.run_migrations().await.expect("migrations run");
    (repo, container)
}

#[tokio::test]
async fn roles_and_groups_behave_the_same_on_postgres() {
    let (repo, _container) = repo_and_container().await;

    let alice = repo
        .create_user("alice", "hash", true)
        .await
        .expect("user created");

    // Direct grants, including the idempotence that ON CONFLICT provides.
    repo.grant_user_role(alice.id, "viewer")
        .await
        .expect("grant");
    repo.grant_user_role(alice.id, "viewer")
        .await
        .expect("granting twice is a no-op");
    assert_eq!(
        repo.list_user_roles(alice.id).await.expect("roles"),
        vec!["viewer".to_string()]
    );

    // create_group is the one method whose two-statement shape had to be
    // translated by hand: insert-if-absent, then read the key back.
    let native = repo
        .create_group("developers", GroupSource::Native)
        .await
        .expect("native group");
    let native_again = repo
        .create_group("developers", GroupSource::Native)
        .await
        .expect("second create returns the same group");
    assert_eq!(native.id, native_again.id);

    // The uniqueness constraint is on (name, source), so the same name from the
    // directory is a different group.
    let directory = repo
        .create_group("developers", GroupSource::Ldap)
        .await
        .expect("directory group");
    assert_ne!(native.id, directory.id);
    assert_eq!(repo.list_groups().await.expect("groups").len(), 2);

    // The UNION query behind effective_stored_roles.
    repo.set_group_roles(native.id, &["admin".to_string()])
        .await
        .expect("group roles");
    repo.add_group_member(native.id, alice.id)
        .await
        .expect("member added");
    let mut effective = repo
        .effective_stored_roles(alice.id)
        .await
        .expect("effective roles");
    effective.sort();
    assert_eq!(
        effective,
        vec!["admin".to_string(), "viewer".to_string()],
        "roles from a direct grant and from group membership are unioned"
    );

    assert_eq!(
        repo.list_group_member_ids(native.id)
            .await
            .expect("members"),
        vec![alice.id]
    );
    assert!(
        repo.list_group_member_ids(directory.id)
            .await
            .expect("members")
            .is_empty(),
        "a directory group stores no membership"
    );

    // set_group_roles replaces rather than appends.
    repo.set_group_roles(native.id, &["viewer".to_string()])
        .await
        .expect("roles replaced");
    assert_eq!(
        repo.list_group_roles(native.id).await.expect("group roles"),
        vec!["viewer".to_string()]
    );

    assert!(
        repo.rename_group(native.id, "platform")
            .await
            .expect("rename")
    );
    assert!(
        repo.list_groups()
            .await
            .expect("groups")
            .iter()
            .any(|g| g.name == "platform" && g.source == GroupSource::Native)
    );

    assert!(
        repo.remove_group_member(native.id, alice.id)
            .await
            .expect("member removed")
    );
    assert!(
        !repo
            .remove_group_member(native.id, alice.id)
            .await
            .expect("removing twice reports no change")
    );

    assert!(
        repo.revoke_user_role(alice.id, "viewer")
            .await
            .expect("revoke")
    );
    assert!(
        !repo
            .revoke_user_role(alice.id, "viewer")
            .await
            .expect("revoking twice reports no change")
    );

    // Deleting a group takes its membership and grants with it.
    repo.set_group_roles(native.id, &["admin".to_string()])
        .await
        .expect("roles");
    repo.add_group_member(native.id, alice.id)
        .await
        .expect("member");
    assert!(repo.delete_group(native.id).await.expect("delete"));
    assert!(
        repo.effective_stored_roles(alice.id)
            .await
            .expect("effective roles")
            .is_empty()
    );
    assert!(!repo.delete_group(native.id).await.expect("deleting twice"));
}

#[tokio::test]
async fn maintainer_scope_behaves_the_same_on_postgres() {
    let (repo, _container) = repo_and_container().await;

    let ida = repo
        .create_user("ida", "hash", true)
        .await
        .expect("user created");
    let orders = repo.ensure_service("orders").await.expect("producer");
    let billing = repo.ensure_service("billing").await.expect("producer");

    repo.add_user_maintainer(orders, ida.id)
        .await
        .expect("assignment");
    repo.add_user_maintainer(orders, ida.id)
        .await
        .expect("assigning twice is a no-op");
    assert!(
        repo.maintains_producer(ida.id, orders)
            .await
            .expect("check")
    );
    assert!(
        !repo
            .maintains_producer(ida.id, billing)
            .await
            .expect("check")
    );

    // Through a group, which is the arm with the join.
    let group = repo
        .create_group("platform", GroupSource::Native)
        .await
        .expect("group");
    repo.add_group_member(group.id, ida.id)
        .await
        .expect("member");
    repo.add_group_maintainer(billing, group.id)
        .await
        .expect("assignment");
    assert!(
        repo.maintains_producer(ida.id, billing)
            .await
            .expect("check")
    );

    let mut maintained = repo
        .list_maintained_producers(ida.id)
        .await
        .expect("maintained");
    maintained.sort();
    assert_eq!(
        maintained,
        vec!["billing".to_string(), "orders".to_string()]
    );

    assert_eq!(
        repo.list_user_maintainer_ids(orders)
            .await
            .expect("user maintainers"),
        vec![ida.id]
    );
    assert_eq!(
        repo.list_group_maintainer_ids(billing)
            .await
            .expect("group maintainers"),
        vec![group.id]
    );

    assert!(
        repo.remove_user_maintainer(orders, ida.id)
            .await
            .expect("unassign")
    );
    assert!(
        !repo
            .remove_user_maintainer(orders, ida.id)
            .await
            .expect("unassigning twice reports no change")
    );
    assert!(
        repo.remove_group_maintainer(billing, group.id)
            .await
            .expect("unassign group")
    );
    assert!(
        repo.list_maintained_producers(ida.id)
            .await
            .expect("maintained")
            .is_empty()
    );
}

// Note: the producer-onboarding test that lived here was removed with ADR-0003
// (Sanshain 2.0): onboarding and the pending-spec review flow no longer exist,
// so the repository has no onboarding methods to prove on Postgres.
