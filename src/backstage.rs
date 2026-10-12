//! Reading a Backstage `catalog-info.yaml` (#46): only what the network graph
//! needs to show a system's periphery — the `resource:` refs in the `dependsOn`
//! of the Component that *is* the pushing system. Everything else in the file
//! (other entities, metadata, `consumesApis`, `component:` refs) is ignored:
//! edges between systems stay Sanshain's own (`require`), so nothing here can
//! duplicate them or needs a name mapping between Backstage and Sanshain.

use serde::Deserialize;
use serde_yaml_ng::Value;

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum CatalogError {
    #[error("catalog-info.yaml is not valid YAML: {0}")]
    InvalidYaml(String),
    #[error("document {0} of catalog-info.yaml is not a Backstage entity: {1}")]
    NotAnEntity(usize, String),
    #[error(
        "catalog-info.yaml has no Component named '{0}' — the Component's metadata.name must equal the serviceName in sanshain.yaml"
    )]
    NoMatchingComponent(String),
    #[error(
        "catalog-info.yaml has more than one Component named '{0}' — Sanshain cannot tell whose dependsOn to read"
    )]
    AmbiguousComponent(String),
    #[error("dependsOn of Component '{0}' is malformed: {1}")]
    MalformedDependsOn(String, String),
}

/// The `kind` an unqualified `dependsOn` ref is read as. Backstage leaves the
/// default to the field; for a periphery the only kind Sanshain draws is
/// `resource`, so a bare `orders-db` means `resource:<namespace>/orders-db`.
const DEFAULT_KIND: &str = "resource";

/// The namespace Backstage assumes when an entity declares none.
const DEFAULT_NAMESPACE: &str = "default";

/// The entity envelope Sanshain reads. `apiVersion`/`kind`/`metadata.name` are
/// what makes a document a Backstage entity at all; unknown fields are
/// ignored so any valid descriptor is accepted.
#[derive(Deserialize)]
struct Entity {
    #[serde(rename = "apiVersion")]
    api_version: String,
    kind: String,
    metadata: Metadata,
    #[serde(default)]
    spec: Option<Value>,
}

#[derive(Deserialize)]
struct Metadata {
    name: String,
    #[serde(default)]
    namespace: Option<String>,
}

/// The full, lower-cased `resource:<namespace>/<name>` refs the Component named
/// `system` depends on, sorted and deduplicated. Refs to any other kind are
/// dropped. The namespace defaults to the Component's own, as in Backstage;
/// refs are lower-cased because Backstage compares them case-insensitively,
/// and one shared resource must merge into one node.
pub fn component_resource_dependencies(
    yaml: &str,
    system: &str,
) -> Result<Vec<String>, CatalogError> {
    let mut found: Option<Entity> = None;
    for (index, document) in serde_yaml_ng::Deserializer::from_str(yaml).enumerate() {
        let value =
            Value::deserialize(document).map_err(|e| CatalogError::InvalidYaml(e.to_string()))?;
        // An empty document (a leading or trailing `---`) is no entity.
        if value.is_null() {
            continue;
        }
        let entity: Entity = serde_yaml_ng::from_value(value)
            .map_err(|e| CatalogError::NotAnEntity(index + 1, e.to_string()))?;
        if !entity.api_version.starts_with("backstage.io/") {
            return Err(CatalogError::NotAnEntity(
                index + 1,
                format!("apiVersion '{}' is not backstage.io/…", entity.api_version),
            ));
        }
        if entity.kind.eq_ignore_ascii_case("component")
            && entity.metadata.name.eq_ignore_ascii_case(system)
        {
            if found.is_some() {
                return Err(CatalogError::AmbiguousComponent(system.to_string()));
            }
            found = Some(entity);
        }
    }
    let component = found.ok_or_else(|| CatalogError::NoMatchingComponent(system.to_string()))?;
    let namespace = component
        .metadata
        .namespace
        .as_deref()
        .unwrap_or(DEFAULT_NAMESPACE);
    let depends_on = match component.spec.as_ref().and_then(|s| s.get("dependsOn")) {
        None | Some(Value::Null) => return Ok(Vec::new()),
        Some(Value::Sequence(items)) => items,
        Some(_) => {
            return Err(CatalogError::MalformedDependsOn(
                component.metadata.name,
                "it must be a list of entity refs".to_string(),
            ));
        }
    };

    let mut refs = std::collections::BTreeSet::new();
    for item in depends_on {
        let Some(raw) = item.as_str() else {
            return Err(CatalogError::MalformedDependsOn(
                component.metadata.name,
                "every entry must be an entity ref string".to_string(),
            ));
        };
        let (kind, rest) = raw.split_once(':').unwrap_or((DEFAULT_KIND, raw));
        let (ns, name) = rest.split_once('/').unwrap_or((namespace, rest));
        if kind.trim().is_empty() || ns.trim().is_empty() || name.trim().is_empty() {
            return Err(CatalogError::MalformedDependsOn(
                component.metadata.name,
                format!("'{raw}' is not an entity ref of the form [kind:][namespace/]name"),
            ));
        }
        if kind.trim().eq_ignore_ascii_case(DEFAULT_KIND) {
            refs.insert(format!(
                "{DEFAULT_KIND}:{}/{}",
                ns.trim().to_lowercase(),
                name.trim().to_lowercase()
            ));
        }
    }
    Ok(refs.into_iter().collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    const ORDERS: &str = r#"
apiVersion: backstage.io/v1alpha1
kind: Component
metadata:
  name: orders-service
spec:
  type: service
  lifecycle: production
  owner: team-a
  dependsOn:
    - resource:default/orders-db
    - resource:orders-bucket
    - payments-queue
    - component:default/billing-service
  consumesApis:
    - api:default/stripe-api
"#;

    #[test]
    fn reads_only_resource_refs_and_qualifies_them() {
        assert_eq!(
            component_resource_dependencies(ORDERS, "orders-service"),
            Ok(vec![
                "resource:default/orders-bucket".to_string(),
                "resource:default/orders-db".to_string(),
                "resource:default/payments-queue".to_string(),
            ])
        );
    }

    #[test]
    fn picks_the_component_named_like_the_system_in_a_multi_document_file() {
        let yaml = r#"---
apiVersion: backstage.io/v1alpha1
kind: Resource
metadata:
  name: orders-db
spec:
  type: database
  dependsOn: [resource:default/backups]
---
apiVersion: backstage.io/v1alpha1
kind: Component
metadata:
  name: other-service
spec:
  dependsOn: [resource:default/other-db]
---
apiVersion: backstage.io/v1alpha1
kind: Component
metadata:
  name: Orders-Service
spec:
  dependsOn: [resource:default/orders-db]
---
"#;
        assert_eq!(
            component_resource_dependencies(yaml, "orders-service"),
            Ok(vec!["resource:default/orders-db".to_string()])
        );
    }

    #[test]
    fn refs_default_to_the_components_namespace_and_compare_lower_cased() {
        let yaml = r#"
apiVersion: backstage.io/v1alpha1
kind: Component
metadata:
  name: orders-service
  namespace: shop
spec:
  dependsOn:
    - Orders-DB
    - Resource:Default/Orders-DB
    - resource:orders-db
"#;
        assert_eq!(
            component_resource_dependencies(yaml, "orders-service"),
            Ok(vec![
                "resource:default/orders-db".to_string(),
                "resource:shop/orders-db".to_string(),
            ])
        );
    }

    #[test]
    fn a_component_without_depends_on_has_no_periphery() {
        let yaml = "apiVersion: backstage.io/v1alpha1\nkind: Component\nmetadata:\n  name: svc\nspec:\n  type: service\n";
        assert_eq!(component_resource_dependencies(yaml, "svc"), Ok(vec![]));
        let no_spec =
            "apiVersion: backstage.io/v1alpha1\nkind: Component\nmetadata:\n  name: svc\n";
        assert_eq!(component_resource_dependencies(no_spec, "svc"), Ok(vec![]));
    }

    #[test]
    fn a_missing_component_is_rejected_by_name() {
        assert_eq!(
            component_resource_dependencies(ORDERS, "billing-service"),
            Err(CatalogError::NoMatchingComponent(
                "billing-service".to_string()
            ))
        );
    }

    #[test]
    fn two_components_with_the_systems_name_are_ambiguous() {
        let yaml = "apiVersion: backstage.io/v1alpha1\nkind: Component\nmetadata:\n  name: svc\n---\napiVersion: backstage.io/v1alpha1\nkind: Component\nmetadata:\n  name: svc\n  namespace: other\n";
        assert_eq!(
            component_resource_dependencies(yaml, "svc"),
            Err(CatalogError::AmbiguousComponent("svc".to_string()))
        );
    }

    #[test]
    fn invalid_yaml_is_rejected() {
        assert!(matches!(
            component_resource_dependencies("kind: [unclosed", "svc"),
            Err(CatalogError::InvalidYaml(_))
        ));
    }

    #[test]
    fn a_document_that_is_no_backstage_entity_is_rejected_with_its_position() {
        assert!(matches!(
            component_resource_dependencies("serviceName: svc\n", "svc"),
            Err(CatalogError::NotAnEntity(1, _))
        ));
        let foreign = "apiVersion: v1\nkind: ConfigMap\nmetadata:\n  name: svc\n";
        assert!(matches!(
            component_resource_dependencies(foreign, "svc"),
            Err(CatalogError::NotAnEntity(1, _))
        ));
    }

    #[test]
    fn malformed_depends_on_is_rejected() {
        let not_a_list = "apiVersion: backstage.io/v1alpha1\nkind: Component\nmetadata:\n  name: svc\nspec:\n  dependsOn: resource:default/db\n";
        assert!(matches!(
            component_resource_dependencies(not_a_list, "svc"),
            Err(CatalogError::MalformedDependsOn(_, _))
        ));
        let not_a_string = "apiVersion: backstage.io/v1alpha1\nkind: Component\nmetadata:\n  name: svc\nspec:\n  dependsOn: [{name: db}]\n";
        assert!(matches!(
            component_resource_dependencies(not_a_string, "svc"),
            Err(CatalogError::MalformedDependsOn(_, _))
        ));
        let empty_name = "apiVersion: backstage.io/v1alpha1\nkind: Component\nmetadata:\n  name: svc\nspec:\n  dependsOn: ['resource:default/']\n";
        assert!(matches!(
            component_resource_dependencies(empty_name, "svc"),
            Err(CatalogError::MalformedDependsOn(_, _))
        ));
    }
}
