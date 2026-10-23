//! Folding one account into another (#70: an LDAP user who signed in with
//! both their short name and their e-mail address had two accounts) on both
//! engines. The merge is one transaction over six tables, and the two
//! dialects differ in placeholders and in how a correlated update reads.

use sanshain_service::domain::models::{ApiTokenScope, GroupSource};
use sanshain_service::domain::ports::{NewApiToken, SpecRepository};
use sanshain_service::infrastructure::postgres_repository::PostgresSpecRepository;
use sanshain_service::infrastructure::sqlite_repository::SqliteSpecRepository;
use sqlx::postgres::PgPoolOptions;
use sqlx::sqlite::SqlitePoolOptions;

mod common;

// `cfg(test)` is always true in this crate; the attribute marks the helper as
// test code so clippy's `allow-*-in-tests` exemptions apply to it.
#[cfg(test)]
async fn merges_everything_and_keeps_what_the_target_has(repo: &impl SpecRepository) {
    let axel = repo.create_user("axel", "hash", true).await.unwrap();
    let dup = repo
        .create_user("a.smart@alva.com", "hash", true)
        .await
        .unwrap();
    let orders = repo.ensure_service("orders").await.unwrap();
    let group = repo
        .create_group("team", GroupSource::Native)
        .await
        .unwrap();

    // Both hold the same role, group and maintainership; only dup holds more.
    for user in [axel.id, dup.id] {
        repo.grant_user_role(user, "viewer").await.unwrap();
        repo.add_group_member(group.id, user).await.unwrap();
        repo.add_user_maintainer(orders, user).await.unwrap();
        repo.add_user_favorite(user, "service", "orders")
            .await
            .unwrap();
    }
    repo.grant_user_role(dup.id, "releaser").await.unwrap();
    repo.add_user_favorite(dup.id, "client", "billing")
        .await
        .unwrap();
    repo.create_api_token(NewApiToken {
        id: "tok-1",
        user_id: dup.id,
        name: "ci",
        token_hash: "hash-1",
        scope: ApiTokenScope::Full,
        created_at: "2026-10-23T00:00:00Z",
        expires_at: "2099-01-01T00:00:00Z",
        issued_by: None,
    })
    .await
    .unwrap();
    let session = repo.create_session(dup.id, "2099-01-01").await.unwrap();
    repo.set_directory_id(dup.id, "uuid:1").await.unwrap();

    repo.merge_user_into(dup.id, axel.id).await.unwrap();

    assert!(repo.find_user("a.smart@alva.com").await.unwrap().is_none());
    let mut roles = repo.list_user_roles(axel.id).await.unwrap();
    roles.sort();
    assert_eq!(roles, vec!["releaser".to_string(), "viewer".to_string()]);
    assert_eq!(
        repo.list_group_member_ids(group.id).await.unwrap(),
        vec![axel.id]
    );
    assert_eq!(
        repo.list_user_maintainer_ids(orders).await.unwrap(),
        vec![axel.id]
    );
    assert_eq!(
        repo.get_user_favorites(axel.id, "service").await.unwrap(),
        vec!["orders".to_string()]
    );
    assert_eq!(
        repo.get_user_favorites(axel.id, "client").await.unwrap(),
        vec!["billing".to_string()]
    );
    let tokens = repo.list_api_tokens(axel.id).await.unwrap();
    assert_eq!(tokens.len(), 1);
    assert_eq!(tokens[0].name, "ci");
    let (owner, _) = repo
        .validate_session(&session.token)
        .await
        .unwrap()
        .expect("the session survives");
    assert_eq!(owner.id, axel.id);
    // The duplicate's profile went with it, so its directory ID is free.
    assert!(
        repo.find_user_by_directory_id("uuid:1")
            .await
            .unwrap()
            .is_none()
    );
    repo.set_directory_id(axel.id, "uuid:1").await.unwrap();
    assert_eq!(
        repo.find_user_by_directory_id("uuid:1")
            .await
            .unwrap()
            .map(|u| u.id),
        Some(axel.id)
    );

    repo.rename_user(axel.id, "axel.dumb").await.unwrap();
    assert_eq!(
        repo.find_user("axel.dumb").await.unwrap().map(|u| u.id),
        Some(axel.id)
    );
}

#[tokio::test]
async fn sqlite_folds_one_account_into_another() {
    let pool = SqlitePoolOptions::new()
        .connect("sqlite::memory:")
        .await
        .unwrap();
    let repo = SqliteSpecRepository::new(pool);
    repo.run_migrations().await.unwrap();
    merges_everything_and_keeps_what_the_target_has(&repo).await;
}

#[tokio::test]
async fn postgres_folds_one_account_into_another() {
    let container = common::start_postgres().await;
    let host = container.get_host().await.expect("host");
    let port = container.get_host_port_ipv4(5432).await.expect("port");
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&format!(
            "postgres://postgres:postgres@{host}:{port}/postgres"
        ))
        .await
        .expect("connects");
    let repo = PostgresSpecRepository::new(pool);
    repo.run_migrations().await.expect("migrations run");
    merges_everything_and_keeps_what_the_target_has(&repo).await;
}
