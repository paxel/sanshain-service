//! Products (#76, ADR-0011): named envelopes of systems, defined from outside
//! and above. A Product collects systems by name and passes no power over
//! them; its Product Maintainers may change its membership and nothing else.
//! Members come by hand (known systems only) or from a pasted Helm umbrella
//! `Chart.yaml`, whose current version is the truth for what it lists.

use super::catalog_service::participants;
use super::run_cpu_bound;
use crate::domain::models::{
    AppError, GroupSource, Product, ProductImportChanges, ProductMemberOrigin,
};
use crate::domain::ports::{RepositoryError, SpecRepository};
use serde::Serialize;
use tracing::instrument;

/// The longest Product name accepted.
pub const MAX_PRODUCT_NAME_CHARS: usize = 100;

/// A Product in the list: its name and how many members it has.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ProductSummary {
    pub name: String,
    pub created_at: String,
    pub member_count: usize,
}

/// A member as the Product page shows it: its origin and the roles its
/// system has right now. Neither role means "not in Sanshain".
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ProductMemberView {
    pub system: String,
    pub origin: ProductMemberOrigin,
    pub producer: bool,
    pub consumer: bool,
}

/// A user maintaining a Product, named for the page.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ProductMaintainerUser {
    pub id: i64,
    pub username: String,
    pub display_name: Option<String>,
}

/// A group maintaining a Product, named for the page.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ProductMaintainerGroup {
    pub id: i64,
    pub name: String,
    pub source: GroupSource,
}

/// Who maintains a Product.
#[derive(Debug, Clone, Serialize, PartialEq, Eq, Default)]
pub struct ProductMaintainers {
    pub users: Vec<ProductMaintainerUser>,
    pub groups: Vec<ProductMaintainerGroup>,
}

/// Everything the Product page shows.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ProductDetail {
    pub name: String,
    pub created_at: String,
    pub members: Vec<ProductMemberView>,
    pub maintainers: ProductMaintainers,
}

/// What a `Chart.yaml` import changed, and the chart's names that matched
/// no Producer and no Consumer. All three sorted.
#[derive(Debug, Clone, Serialize, PartialEq, Eq, Default)]
pub struct ProductImportReport {
    pub added: Vec<String>,
    pub removed: Vec<String>,
    pub skipped: Vec<String>,
}

fn validate_product_name(name: &str) -> Result<String, AppError> {
    let name = name.trim();
    if name.is_empty() {
        return Err(AppError::BadRequest(
            "a Product needs a non-empty name".to_string(),
        ));
    }
    if name.chars().count() > MAX_PRODUCT_NAME_CHARS {
        return Err(AppError::BadRequest(format!(
            "a Product name has at most {MAX_PRODUCT_NAME_CHARS} characters"
        )));
    }
    // `/` would split the Product's route, `,` the graph's focus list.
    if name.chars().any(|c| c == '/' || c == ',' || c.is_control()) {
        return Err(AppError::BadRequest(
            "a Product name may not contain '/', ',' or control characters".to_string(),
        ));
    }
    Ok(name.to_string())
}

/// The Product of that name, or `NotFound`.
pub async fn require_product(repo: &impl SpecRepository, name: &str) -> Result<Product, AppError> {
    repo.find_product(name)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("no Product '{name}'")))
}

/// Create a Product with no members. Its name is unique among Products only:
/// a system of the same name is no conflict.
#[instrument(skip(repo))]
pub async fn create_product(repo: &impl SpecRepository, name: &str) -> Result<Product, AppError> {
    let name = validate_product_name(name)?;
    repo.create_product(&name, &super::now_iso())
        .await
        .map_err(|e| match e {
            RepositoryError::Conflict => {
                AppError::Conflict(format!("a Product named '{name}' already exists"))
            }
            other => other.into(),
        })
}

/// Delete a Product. Its member systems stay exactly as they are.
#[instrument(skip(repo))]
pub async fn delete_product(repo: &impl SpecRepository, name: &str) -> Result<(), AppError> {
    let product = require_product(repo, name).await?;
    repo.delete_product(product.id).await?;
    Ok(())
}

/// Every Product, sorted by name.
#[instrument(skip_all)]
pub async fn list_products(repo: &impl SpecRepository) -> Result<Vec<ProductSummary>, AppError> {
    Ok(repo
        .list_products()
        .await?
        .into_iter()
        .map(|p| ProductSummary {
            member_count: p.members.len(),
            name: p.name,
            created_at: p.created_at,
        })
        .collect())
}

/// One Product with its members' current roles and its maintainers.
#[instrument(skip(repo))]
pub async fn describe_product(
    repo: &impl SpecRepository,
    name: &str,
) -> Result<ProductDetail, AppError> {
    let product = require_product(repo, name).await?;
    let producers = repo.list_producers().await?;
    let consumers = repo.list_consumers().await?;
    let members = product
        .members
        .into_iter()
        .map(|m| ProductMemberView {
            producer: producers.contains(&m.system),
            consumer: consumers.contains(&m.system),
            system: m.system,
            origin: m.origin,
        })
        .collect();
    Ok(ProductDetail {
        maintainers: maintainers_of(repo, product.id).await?,
        name: product.name,
        created_at: product.created_at,
        members,
    })
}

async fn maintainers_of(
    repo: &impl SpecRepository,
    product_id: i64,
) -> Result<ProductMaintainers, AppError> {
    let user_ids = repo.list_product_user_maintainer_ids(product_id).await?;
    let group_ids = repo.list_product_group_maintainer_ids(product_id).await?;
    let mut users = Vec::with_capacity(user_ids.len());
    if !user_ids.is_empty() {
        let profiles = repo.list_user_profiles().await?;
        for user in repo.list_users().await? {
            if user_ids.contains(&user.id) {
                users.push(ProductMaintainerUser {
                    display_name: profiles
                        .iter()
                        .find(|p| p.user_id == user.id)
                        .and_then(|p| p.display_name.clone()),
                    id: user.id,
                    username: user.username,
                });
            }
        }
        users.sort_by(|a, b| a.username.cmp(&b.username));
    }
    let mut groups: Vec<ProductMaintainerGroup> = repo
        .list_groups()
        .await?
        .into_iter()
        .filter(|g| group_ids.contains(&g.id))
        .map(|g| ProductMaintainerGroup {
            id: g.id,
            name: g.name,
            source: g.source,
        })
        .collect();
    groups.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(ProductMaintainers { users, groups })
}

/// Add a known system by hand. Only a Producer or a Consumer can be added;
/// a system that is a member already keeps its origin. Answers whether it
/// was added.
#[instrument(skip(repo))]
pub async fn add_product_member(
    repo: &impl SpecRepository,
    product: &str,
    system: &str,
) -> Result<bool, AppError> {
    let product = require_product(repo, product).await?;
    let system = system.trim();
    if !participants(repo).await?.iter().any(|p| p == system) {
        return Err(AppError::BadRequest(format!(
            "'{system}' is neither a Producer nor a Consumer, so it cannot be added by hand"
        )));
    }
    Ok(repo
        .add_product_member(product.id, system, ProductMemberOrigin::Manual)
        .await?)
}

/// Remove a member, whatever its origin. `NotFound` when it is none.
#[instrument(skip(repo))]
pub async fn remove_product_member(
    repo: &impl SpecRepository,
    product: &str,
    system: &str,
) -> Result<(), AppError> {
    let found = require_product(repo, product).await?;
    if !repo.remove_product_member(found.id, system).await? {
        return Err(AppError::NotFound(format!(
            "'{system}' is no member of Product '{product}'"
        )));
    }
    Ok(())
}

/// Apply a pasted umbrella `Chart.yaml`: each dependency's `alias`, else its
/// `name`, is a system name. Names that match no Producer and no Consumer
/// are skipped; every other one becomes a `helm` member (a `manual` one is
/// claimed), every `helm` member the chart no longer lists is removed, and
/// `manual` members stay. A chart that does not parse, or lists no
/// dependencies, changes nothing.
#[instrument(skip(repo, chart_yaml))]
pub async fn import_product_chart(
    repo: &impl SpecRepository,
    product: &str,
    chart_yaml: &str,
) -> Result<ProductImportReport, AppError> {
    let product = require_product(repo, product).await?;
    let yaml = chart_yaml.to_string();
    let names = run_cpu_bound(move || {
        crate::helm::dependency_names(&yaml).map_err(|e| AppError::BadRequest(e.to_string()))
    })
    .await?;
    let known = participants(repo).await?;
    let (matched, mut skipped): (Vec<String>, Vec<String>) =
        names.into_iter().partition(|name| known.contains(name));
    skipped.sort();
    let ProductImportChanges { added, removed } =
        repo.apply_product_import(product.id, &matched).await?;
    Ok(ProductImportReport {
        added,
        removed,
        skipped,
    })
}

/// Assign a user as Product Maintainer.
#[instrument(skip(repo))]
pub async fn assign_product_user_maintainer(
    repo: &impl SpecRepository,
    product: &str,
    user_id: i64,
) -> Result<(), AppError> {
    let product = require_product(repo, product).await?;
    if !repo.list_users().await?.iter().any(|u| u.id == user_id) {
        return Err(AppError::NotFound(format!("User {user_id} not found")));
    }
    repo.add_product_user_maintainer(product.id, user_id)
        .await?;
    Ok(())
}

/// Assign a group (native or directory) as Product Maintainer.
#[instrument(skip(repo))]
pub async fn assign_product_group_maintainer(
    repo: &impl SpecRepository,
    product: &str,
    group_id: i64,
) -> Result<(), AppError> {
    let product = require_product(repo, product).await?;
    if !repo.list_groups().await?.iter().any(|g| g.id == group_id) {
        return Err(AppError::NotFound(format!("Group {group_id} not found")));
    }
    repo.add_product_group_maintainer(product.id, group_id)
        .await?;
    Ok(())
}

/// Take a user's Product maintainership away. Answers whether they held it.
#[instrument(skip(repo))]
pub async fn unassign_product_user_maintainer(
    repo: &impl SpecRepository,
    product: &str,
    user_id: i64,
) -> Result<bool, AppError> {
    let product = require_product(repo, product).await?;
    Ok(repo
        .remove_product_user_maintainer(product.id, user_id)
        .await?)
}

/// Take a group's Product maintainership away. Answers whether it held it.
#[instrument(skip(repo))]
pub async fn unassign_product_group_maintainer(
    repo: &impl SpecRepository,
    product: &str,
    group_id: i64,
) -> Result<bool, AppError> {
    let product = require_product(repo, product).await?;
    Ok(repo
        .remove_product_group_maintainer(product.id, group_id)
        .await?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::mock_repo::MockRepo;

    const CHART: &str = "apiVersion: v2\nname: shop\ndependencies:\n  - name: orders-chart\n    alias: orders\n  - name: billing\n  - name: redis\n";

    async fn repo_with_systems() -> MockRepo {
        let repo = MockRepo::new();
        repo.ensure_service("orders").await.unwrap();
        repo.ensure_service("billing").await.unwrap();
        repo.ensure_client("frontend").await.unwrap();
        repo
    }

    #[tokio::test]
    async fn a_product_name_is_validated_and_unique_among_products() {
        let repo = repo_with_systems().await;
        assert!(matches!(
            create_product(&repo, "  ").await,
            Err(AppError::BadRequest(_))
        ));
        assert!(matches!(
            create_product(&repo, "a,b").await,
            Err(AppError::BadRequest(_))
        ));
        assert!(matches!(
            create_product(&repo, &"x".repeat(MAX_PRODUCT_NAME_CHARS + 1)).await,
            Err(AppError::BadRequest(_))
        ));
        // A Product may share its name with a system.
        let created = create_product(&repo, " orders ").await.unwrap();
        assert_eq!(created.name, "orders");
        assert!(matches!(
            create_product(&repo, "orders").await,
            Err(AppError::Conflict(_))
        ));
        assert_eq!(
            list_products(&repo).await.unwrap(),
            [ProductSummary {
                name: "orders".to_string(),
                created_at: created.created_at,
                member_count: 0,
            }]
        );
        delete_product(&repo, "orders").await.unwrap();
        assert!(matches!(
            delete_product(&repo, "orders").await,
            Err(AppError::NotFound(_))
        ));
    }

    #[tokio::test]
    async fn only_known_systems_are_added_by_hand_and_members_show_their_roles() {
        let repo = repo_with_systems().await;
        create_product(&repo, "shop").await.unwrap();
        assert!(add_product_member(&repo, "shop", "frontend").await.unwrap());
        assert!(!add_product_member(&repo, "shop", "frontend").await.unwrap());
        assert!(matches!(
            add_product_member(&repo, "shop", "ghost").await,
            Err(AppError::BadRequest(_))
        ));
        assert!(matches!(
            add_product_member(&repo, "nope", "frontend").await,
            Err(AppError::NotFound(_))
        ));
        let detail = describe_product(&repo, "shop").await.unwrap();
        assert_eq!(
            detail.members,
            [ProductMemberView {
                system: "frontend".to_string(),
                origin: ProductMemberOrigin::Manual,
                producer: false,
                consumer: true,
            }]
        );
        remove_product_member(&repo, "shop", "frontend")
            .await
            .unwrap();
        assert!(matches!(
            remove_product_member(&repo, "shop", "frontend").await,
            Err(AppError::NotFound(_))
        ));
    }

    #[tokio::test]
    async fn an_import_skips_unknown_names_and_follows_the_current_chart() {
        let repo = repo_with_systems().await;
        create_product(&repo, "shop").await.unwrap();
        add_product_member(&repo, "shop", "frontend").await.unwrap();
        let report = import_product_chart(&repo, "shop", CHART).await.unwrap();
        assert_eq!(report.added, ["billing", "orders"]);
        assert!(report.removed.is_empty());
        assert_eq!(report.skipped, ["redis"]);

        let report = import_product_chart(
            &repo,
            "shop",
            "dependencies:\n  - name: orders\n  - name: frontend\n",
        )
        .await
        .unwrap();
        assert!(report.added.is_empty());
        assert_eq!(report.removed, ["billing"]);
        let origins: Vec<(String, ProductMemberOrigin)> = describe_product(&repo, "shop")
            .await
            .unwrap()
            .members
            .into_iter()
            .map(|m| (m.system, m.origin))
            .collect();
        assert_eq!(
            origins,
            [
                ("frontend".to_string(), ProductMemberOrigin::Helm),
                ("orders".to_string(), ProductMemberOrigin::Helm),
            ]
        );
        assert!(matches!(
            import_product_chart(&repo, "shop", "name: shop\n").await,
            Err(AppError::BadRequest(_))
        ));
        assert_eq!(
            describe_product(&repo, "shop").await.unwrap().members.len(),
            2,
            "a refused chart changes nothing"
        );
    }

    #[tokio::test]
    async fn maintainers_are_assigned_named_and_unassigned() {
        let repo = repo_with_systems().await;
        create_product(&repo, "shop").await.unwrap();
        let user = repo.create_user("alva", "hash", true).await.unwrap();
        let group = repo
            .create_group("team", GroupSource::Native)
            .await
            .unwrap();
        assign_product_user_maintainer(&repo, "shop", user.id)
            .await
            .unwrap();
        assign_product_group_maintainer(&repo, "shop", group.id)
            .await
            .unwrap();
        assert!(matches!(
            assign_product_user_maintainer(&repo, "shop", 999).await,
            Err(AppError::NotFound(_))
        ));
        assert!(matches!(
            assign_product_group_maintainer(&repo, "shop", 999).await,
            Err(AppError::NotFound(_))
        ));
        let maintainers = describe_product(&repo, "shop").await.unwrap().maintainers;
        assert_eq!(maintainers.users.len(), 1);
        assert_eq!(maintainers.users[0].username, "alva");
        assert_eq!(maintainers.groups[0].name, "team");
        assert!(
            unassign_product_user_maintainer(&repo, "shop", user.id)
                .await
                .unwrap()
        );
        assert!(
            !unassign_product_user_maintainer(&repo, "shop", user.id)
                .await
                .unwrap()
        );
        assert!(
            unassign_product_group_maintainer(&repo, "shop", group.id)
                .await
                .unwrap()
        );
        assert_eq!(
            describe_product(&repo, "shop").await.unwrap().maintainers,
            ProductMaintainers::default()
        );
    }
}
