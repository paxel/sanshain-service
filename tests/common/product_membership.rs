//! One scenario for both engines (#76): a Product's members by hand and by
//! chart import, its Product Maintainers directly and through a group, and
//! its deletion taking members and grants along while leaving nothing else.

use sanshain_service::domain::models::{GroupSource, ProductMemberOrigin};
use sanshain_service::domain::ports::{RepositoryError, SpecRepository};

// `cfg(test)` marks the helper as test code for clippy's `allow-unwrap-in-tests`.
#[cfg(test)]
pub async fn product_membership_round_trips(repo: &impl SpecRepository) {
    let now = "2026-11-08T10:00:00Z";
    let shop = repo.create_product("shop", now).await.unwrap();
    assert_eq!(shop.name, "shop");
    assert!(shop.members.is_empty());
    assert!(matches!(
        repo.create_product("shop", now).await,
        Err(RepositoryError::Conflict)
    ));

    assert!(
        repo.add_product_member(shop.id, "notes", ProductMemberOrigin::Manual)
            .await
            .unwrap()
    );
    assert!(
        repo.add_product_member(shop.id, "orders", ProductMemberOrigin::Manual)
            .await
            .unwrap()
    );
    assert!(
        !repo
            .add_product_member(shop.id, "orders", ProductMemberOrigin::Helm)
            .await
            .unwrap(),
        "a member keeps its row and origin"
    );

    let changes = repo
        .apply_product_import(shop.id, &["orders".to_string(), "billing".to_string()])
        .await
        .unwrap();
    assert_eq!(changes.added, ["billing"]);
    assert!(changes.removed.is_empty());
    let members = |p: &sanshain_service::domain::models::Product| -> Vec<(String, &'static str)> {
        p.members
            .iter()
            .map(|m| (m.system.clone(), m.origin.as_str()))
            .collect()
    };
    let found = repo.find_product("shop").await.unwrap().unwrap();
    assert_eq!(
        members(&found),
        [
            ("billing".to_string(), "helm"),
            ("notes".to_string(), "manual"),
            ("orders".to_string(), "helm"),
        ]
    );

    let changes = repo
        .apply_product_import(shop.id, &["billing".to_string()])
        .await
        .unwrap();
    assert!(changes.added.is_empty());
    assert_eq!(changes.removed, ["orders"]);
    assert!(repo.remove_product_member(shop.id, "notes").await.unwrap());
    assert!(!repo.remove_product_member(shop.id, "notes").await.unwrap());

    let user = repo.create_user("pm-user", "hash", true).await.unwrap();
    let member = repo.create_user("pm-member", "hash", true).await.unwrap();
    let team = repo
        .create_group("pm-team", GroupSource::Native)
        .await
        .unwrap();
    let directory = repo
        .create_group("cn=pm,dc=corp", GroupSource::Ldap)
        .await
        .unwrap();
    repo.add_group_member(team.id, member.id).await.unwrap();
    repo.add_product_user_maintainer(shop.id, user.id)
        .await
        .unwrap();
    repo.add_product_user_maintainer(shop.id, user.id)
        .await
        .unwrap();
    repo.add_product_group_maintainer(shop.id, team.id)
        .await
        .unwrap();
    repo.add_product_group_maintainer(shop.id, directory.id)
        .await
        .unwrap();
    assert_eq!(
        repo.list_product_user_maintainer_ids(shop.id)
            .await
            .unwrap(),
        [user.id]
    );
    assert_eq!(
        repo.list_product_group_maintainer_ids(shop.id)
            .await
            .unwrap()
            .len(),
        2
    );
    assert!(repo.maintains_product(user.id, shop.id).await.unwrap());
    assert!(repo.maintains_product(member.id, shop.id).await.unwrap());
    assert_eq!(
        repo.list_maintained_products(member.id).await.unwrap(),
        ["shop"]
    );
    assert_eq!(
        repo.list_group_maintained_products(&[directory.id])
            .await
            .unwrap(),
        ["shop"]
    );
    assert!(
        repo.remove_product_user_maintainer(shop.id, user.id)
            .await
            .unwrap()
    );
    assert!(!repo.maintains_product(user.id, shop.id).await.unwrap());
    assert!(
        repo.remove_product_group_maintainer(shop.id, team.id)
            .await
            .unwrap()
    );
    assert!(!repo.maintains_product(member.id, shop.id).await.unwrap());

    repo.create_product("other", now).await.unwrap();
    let names: Vec<String> = repo
        .list_products()
        .await
        .unwrap()
        .into_iter()
        .map(|p| p.name)
        .collect();
    assert_eq!(names, ["other", "shop"]);

    assert!(repo.delete_product(shop.id).await.unwrap());
    assert!(!repo.delete_product(shop.id).await.unwrap());
    assert!(repo.find_product("shop").await.unwrap().is_none());
    assert!(
        repo.list_product_group_maintainer_ids(shop.id)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        repo.list_group_maintained_products(&[directory.id])
            .await
            .unwrap()
            .is_empty()
    );
}
