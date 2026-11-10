//! The row shapes the Product tables (#76) are read with, their mapping into
//! the domain, and the import plan — shared by the SQLite and Postgres
//! repositories so the two dialects cannot disagree on what an import does.

use crate::domain::models::{Product, ProductImportChanges, ProductMember, ProductMemberOrigin};
use crate::domain::ports::RepositoryError;
use std::collections::BTreeSet;

/// `(id, name, created_at)` of a `products` row.
pub(crate) type ProductRow = (i64, String, String);

/// `(product_id, system_name, origin)` of a `product_members` row.
pub(crate) type ProductMemberRow = (i64, String, String);

fn origin(raw: &str) -> Result<ProductMemberOrigin, RepositoryError> {
    ProductMemberOrigin::parse(raw)
        .ok_or_else(|| RepositoryError::Internal(format!("unknown product member origin '{raw}'")))
}

/// The Products of `rows`, each with its members from `members` sorted by
/// system name; the Products keep the order of `rows`.
pub(crate) fn products_from_rows(
    rows: Vec<ProductRow>,
    members: Vec<ProductMemberRow>,
) -> Result<Vec<Product>, RepositoryError> {
    let mut products: Vec<Product> = rows
        .into_iter()
        .map(|(id, name, created_at)| Product {
            id,
            name,
            created_at,
            members: Vec::new(),
        })
        .collect();
    for (product_id, system, raw_origin) in members {
        if let Some(product) = products.iter_mut().find(|p| p.id == product_id) {
            product.members.push(ProductMember {
                system,
                origin: origin(&raw_origin)?,
            });
        }
    }
    for product in &mut products {
        product.members.sort_by(|a, b| a.system.cmp(&b.system));
    }
    Ok(products)
}

/// What an import does to the members `current` (`(system, origin)` rows):
/// every listed system ends up a `helm` member, every `helm` member not
/// listed goes, `manual` members not listed stay.
pub(crate) struct ImportPlan {
    /// Systems to insert as `helm`, or to switch to `helm` when present.
    pub upsert: Vec<String>,
    /// `helm` members to delete.
    pub remove: Vec<String>,
    /// What the caller is told.
    pub changes: ProductImportChanges,
}

pub(crate) fn import_plan(
    current: &[(String, String)],
    systems: &[String],
) -> Result<ImportPlan, RepositoryError> {
    let listed: BTreeSet<&str> = systems.iter().map(String::as_str).collect();
    let present: BTreeSet<&str> = current.iter().map(|(s, _)| s.as_str()).collect();
    let mut remove = Vec::new();
    for (system, raw_origin) in current {
        if origin(raw_origin)? == ProductMemberOrigin::Helm && !listed.contains(system.as_str()) {
            remove.push(system.clone());
        }
    }
    remove.sort();
    let added: Vec<String> = listed
        .iter()
        .filter(|s| !present.contains(*s))
        .map(|s| s.to_string())
        .collect();
    Ok(ImportPlan {
        upsert: listed.iter().map(|s| s.to_string()).collect(),
        remove: remove.clone(),
        changes: ProductImportChanges {
            added,
            removed: remove,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(system: &str, origin: &str) -> (String, String) {
        (system.to_string(), origin.to_string())
    }

    #[test]
    fn an_import_adds_new_names_drops_unlisted_helm_members_and_keeps_manual_ones() {
        let current = [
            row("billing", "helm"),
            row("ledger", "helm"),
            row("notes", "manual"),
            row("search", "manual"),
        ];
        let systems = [
            "orders".to_string(),
            "billing".to_string(),
            "search".to_string(),
        ];
        let plan = import_plan(&current, &systems).unwrap();
        assert_eq!(plan.upsert, ["billing", "orders", "search"]);
        assert_eq!(plan.remove, ["ledger"]);
        assert_eq!(plan.changes.added, ["orders"]);
        assert_eq!(plan.changes.removed, ["ledger"]);
    }

    #[test]
    fn an_unknown_stored_origin_is_an_error() {
        assert!(import_plan(&[row("x", "chart")], &[]).is_err());
    }

    #[test]
    fn members_are_attached_to_their_product_sorted() {
        let products = products_from_rows(
            vec![
                (1, "shop".into(), "t".into()),
                (2, "empty".into(), "t".into()),
            ],
            vec![
                (1, "orders".into(), "manual".into()),
                (1, "billing".into(), "helm".into()),
                (9, "stray".into(), "helm".into()),
            ],
        )
        .unwrap();
        assert_eq!(products.len(), 2);
        let names: Vec<&str> = products[0]
            .members
            .iter()
            .map(|m| m.system.as_str())
            .collect();
        assert_eq!(names, ["billing", "orders"]);
        assert_eq!(products[0].members[0].origin, ProductMemberOrigin::Helm);
        assert!(products[1].members.is_empty());
    }
}
