//! Reading a Backstage `catalog-info.yaml` (#46, #48): only what the network
//! graph and the system pages need of the Component that *is* the pushing
//! system — the `resource:` refs in its `dependsOn`, its owner, description,
//! source location, links and tags, and the `component:` refs that name no
//! Sanshain system (ADR-0010). Everything else in the file (other entities,
//! `consumesApis`, lifecycle, any further field) is
//! ignored: edges between systems stay Sanshain's own (`require`), so nothing
//! here can duplicate them or needs a name mapping between Backstage and
//! Sanshain.

use crate::domain::models::{SystemContext, SystemLink, SystemOwner};
use crate::yaml::Value;
use serde::Deserialize;
use std::collections::BTreeMap;

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum CatalogError {
    #[error("catalog-info.yaml is not valid YAML: {0}")]
    InvalidYaml(String),
    #[error("document {0} of catalog-info.yaml is not a Backstage entity: {1}")]
    NotAnEntity(usize, String),
    #[error(
        "catalog-info.yaml has no Component named '{0}' - the Component's metadata.name must equal the serviceName in sanshain.yaml"
    )]
    NoMatchingComponent(String),
    #[error(
        "catalog-info.yaml has more than one Component named '{0}' - Sanshain cannot tell whose dependsOn to read"
    )]
    AmbiguousComponent(String),
    #[error("dependsOn of Component '{0}' is malformed: {1}")]
    MalformedDependsOn(String, String),
}

/// The `kind` an unqualified `dependsOn` ref is read as. Backstage leaves the
/// default to the field; for a periphery the only kind Sanshain draws is
/// `resource`, so a bare `orders-db` means `resource:<namespace>/orders-db`.
const DEFAULT_KIND: &str = "resource";

/// The `kind` of a `dependsOn` ref drawn as a missing component (ADR-0010).
const COMPONENT_KIND: &str = "component";

/// The `kind` an unqualified `spec.system` ref is read as.
const DEFAULT_SYSTEM_KIND: &str = "system";

/// The `kind` an unqualified `spec.owner` ref is read as — Backstage's
/// default for owners.
const DEFAULT_OWNER_KIND: &str = "group";

/// The namespace Backstage assumes when an entity declares none.
const DEFAULT_NAMESPACE: &str = "default";

/// The annotation that names where a Component's source lives.
const SOURCE_LOCATION_ANNOTATION: &str = "backstage.io/source-location";

/// A description longer than this is cut, ellipsis included (#48).
const MAX_DESCRIPTION_CHARS: usize = 1000;

/// Links beyond this many are ignored (#48).
const MAX_LINKS: usize = 20;

/// A link's `title`, `type` or `icon` longer than this is cut, ellipsis
/// included (#48).
const MAX_LINK_TEXT_CHARS: usize = 200;

/// `metadata.tags` beyond this many are ignored (ADR-0010).
const MAX_TAGS: usize = 20;

/// A link or source-location URL longer than this is dropped (#48).
const MAX_URL_CHARS: usize = 2048;

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

/// The context fields (#48) are read leniently: a value of the wrong shape
/// is simply not there, never a rejected file.
#[derive(Deserialize)]
struct Metadata {
    name: String,
    #[serde(default)]
    namespace: Option<String>,
    #[serde(default)]
    description: Option<Value>,
    #[serde(default)]
    annotations: Option<BTreeMap<String, Value>>,
    #[serde(default)]
    links: Option<Value>,
    #[serde(default)]
    tags: Option<Value>,
}

/// What Sanshain read of the system's Component: its periphery (#46) and
/// its context (#48).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogComponent {
    /// The full, lower-cased `resource:<namespace>/<name>` refs the Component
    /// depends on, sorted and deduplicated. Refs to any other kind are
    /// dropped. The namespace defaults to the Component's own, as in
    /// Backstage; refs are lower-cased because Backstage compares them
    /// case-insensitively, and one shared resource must merge into one node.
    pub resources: Vec<String>,
    /// The full, lower-cased `component:<namespace>/<name>` refs the
    /// Component depends on (ADR-0010), sorted and deduplicated. Drawn as a
    /// missing component while no Sanshain system of that name exists.
    pub components: Vec<String>,
    pub context: SystemContext,
    /// The name of the Backstage system the Component belongs to
    /// (`spec.system`, lower-cased, namespace dropped): the graph's natural
    /// cluster, filling a Producer's empty domain (ADR-0010).
    pub system: Option<String>,
}

/// Read the Component named `system` out of `yaml`: its periphery and its
/// context. The periphery must be well-formed (a malformed `dependsOn`
/// rejects the file); the context fields are taken when they are usable and
/// left empty otherwise, so a partial catalog-info keeps working.
pub fn read_component(yaml: &str, system: &str) -> Result<CatalogComponent, CatalogError> {
    let mut found: Option<Entity> = None;
    let documents: Vec<Value> =
        crate::yaml::from_str_multiple(yaml).map_err(CatalogError::InvalidYaml)?;
    for (index, value) in documents.into_iter().enumerate() {
        // An empty document (a leading or trailing `---`) is no entity.
        if value.is_null() {
            continue;
        }
        let entity: Entity = serde_json::from_value(value)
            .map_err(|e| CatalogError::NotAnEntity(index.saturating_add(1), e.to_string()))?;
        // `backstage.io/v1alpha1`, but also plugin groups such as
        // `scaffolder.backstage.io/v1beta3`: a Template in the same file is a
        // valid entity, merely not the one Sanshain reads.
        let group = entity.api_version.split('/').next().unwrap_or_default();
        if !(group == "backstage.io" || group.ends_with(".backstage.io")) {
            return Err(CatalogError::NotAnEntity(
                index.saturating_add(1),
                format!(
                    "apiVersion '{}' is not a backstage.io group",
                    entity.api_version
                ),
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
    let (resources, components) = dependencies(&component, namespace)?;
    let context = SystemContext {
        owner: component
            .spec
            .as_ref()
            .and_then(|s| s.get("owner"))
            .and_then(Value::as_str)
            .and_then(owner_ref),
        description: component
            .metadata
            .description
            .as_ref()
            .and_then(Value::as_str)
            .and_then(bounded_description),
        source_location: component
            .metadata
            .annotations
            .as_ref()
            .and_then(|a| a.get(SOURCE_LOCATION_ANNOTATION))
            .and_then(Value::as_str)
            .and_then(source_location),
        links: component
            .metadata
            .links
            .as_ref()
            .and_then(Value::as_array)
            .map(|items| items.iter().filter_map(link).take(MAX_LINKS).collect())
            .unwrap_or_default(),
        tags: component
            .metadata
            .tags
            .as_ref()
            .and_then(Value::as_array)
            .map(|items| tags(items))
            .unwrap_or_default(),
    };
    let system = component
        .spec
        .as_ref()
        .and_then(|s| s.get("system"))
        .and_then(Value::as_str)
        .and_then(|raw| split_ref(raw, DEFAULT_SYSTEM_KIND, namespace))
        .filter(|(kind, _, _)| kind == DEFAULT_SYSTEM_KIND)
        .map(|(_, _, name)| name);
    Ok(CatalogComponent {
        resources,
        components,
        context,
        system,
    })
}

/// `spec.dependsOn` as full refs: the periphery half (#46, `resource:` refs)
/// and the `component:` refs (ADR-0010), each sorted and deduplicated.
type Dependencies = (Vec<String>, Vec<String>);
fn dependencies(component: &Entity, namespace: &str) -> Result<Dependencies, CatalogError> {
    let depends_on = match component.spec.as_ref().and_then(|s| s.get("dependsOn")) {
        None | Some(Value::Null) => return Ok((Vec::new(), Vec::new())),
        Some(Value::Array(items)) => items,
        Some(_) => {
            return Err(CatalogError::MalformedDependsOn(
                component.metadata.name.clone(),
                "it must be a list of entity refs".to_string(),
            ));
        }
    };

    let mut refs = std::collections::BTreeSet::new();
    let mut components = std::collections::BTreeSet::new();
    for item in depends_on {
        let Some(raw) = item.as_str() else {
            return Err(CatalogError::MalformedDependsOn(
                component.metadata.name.clone(),
                "every entry must be an entity ref string".to_string(),
            ));
        };
        let Some((kind, ns, name)) = split_ref(raw, DEFAULT_KIND, namespace) else {
            return Err(CatalogError::MalformedDependsOn(
                component.metadata.name.clone(),
                format!("'{raw}' is not an entity ref of the form [kind:][namespace/]name"),
            ));
        };
        if kind == DEFAULT_KIND {
            refs.insert(format!("{DEFAULT_KIND}:{ns}/{name}"));
        } else if kind == COMPONENT_KIND {
            components.insert(format!("{COMPONENT_KIND}:{ns}/{name}"));
        }
    }
    Ok((refs.into_iter().collect(), components.into_iter().collect()))
}

/// The name part of a `component:<namespace>/<name>` ref, the one a Sanshain
/// system name is compared with (ADR-0010).
pub fn component_ref_name(reference: &str) -> Option<&str> {
    let rest = reference.strip_prefix("component:")?;
    Some(rest.rsplit_once('/').map_or(rest, |(_, name)| name))
}

/// Split a Backstage string ref `[kind:][namespace/]name` into its three
/// lower-cased parts, `None` when any part is no ref part.
fn split_ref<'a>(
    raw: &'a str,
    default_kind: &'a str,
    default_namespace: &'a str,
) -> Option<(String, String, String)> {
    let (kind, rest) = raw.split_once(':').unwrap_or((default_kind, raw));
    let (ns, name) = rest.split_once('/').unwrap_or((default_namespace, rest));
    let parts = [kind.trim(), ns.trim(), name.trim()];
    if !parts.iter().all(|part| is_ref_part(part)) {
        return None;
    }
    let [kind, ns, name] = parts.map(str::to_lowercase);
    Some((kind, ns, name))
}

/// `spec.owner` (#48) as kind and name; a bare name is a `group`. A value
/// that is no ref is no owner.
fn owner_ref(raw: &str) -> Option<SystemOwner> {
    split_ref(raw, DEFAULT_OWNER_KIND, DEFAULT_NAMESPACE)
        .map(|(kind, _, name)| SystemOwner { kind, name })
}

/// `metadata.description` (#48): trimmed, at most 1000 characters, the cut
/// marked with an ellipsis. Blank is absent.
fn bounded_description(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(cut(trimmed, MAX_DESCRIPTION_CHARS))
}

/// `text` as is when it has at most `max` characters, otherwise cut to `max`
/// with the last one an ellipsis.
fn cut(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let mut cut: String = text.chars().take(max.saturating_sub(1)).collect();
    cut.push('…');
    cut
}

/// The `backstage.io/source-location` annotation (#48) without Backstage's
/// `url:` prefix, kept as written when it is an `http` or `https` URL.
fn source_location(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    let location = trimmed.strip_prefix("url:").unwrap_or(trimmed).trim();
    http_url(location).map(|_| location.to_string())
}

/// One of `metadata.links` (#48): needs an `http`/`https` `url`; `title`
/// defaults to the host; `type` and `icon` pass through as free text. Each
/// text is cut at [`MAX_LINK_TEXT_CHARS`].
fn link(item: &Value) -> Option<SystemLink> {
    let text = |key: &str| item.get(key).and_then(Value::as_str).map(str::trim);
    let bounded = |key: &str| text(key).map(|t| cut(t, MAX_LINK_TEXT_CHARS));
    let url = text("url")?;
    let parsed = http_url(url)?;
    let title = text("title")
        .filter(|t| !t.is_empty())
        .or_else(|| parsed.host_str())
        .map(|t| cut(t, MAX_LINK_TEXT_CHARS))?;
    Some(SystemLink {
        url: url.to_string(),
        title,
        link_type: bounded("type"),
        icon: bounded("icon"),
    })
}

/// `metadata.tags` (ADR-0010): Backstage's tag charset is lower-case letters,
/// digits, `-`, `+` and `#`, at most 63 characters; a tag outside it is no tag.
/// Lower-cased, deduplicated in order, at most [`MAX_TAGS`].
fn tags(items: &[Value]) -> Vec<String> {
    let mut seen = std::collections::BTreeSet::new();
    let mut out = Vec::new();
    for item in items {
        let Some(raw) = item.as_str() else { continue };
        let tag = raw.trim().to_lowercase();
        let valid = !tag.is_empty()
            && tag.len() <= 63
            && tag.chars().all(|c| {
                c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '-' | '+' | '#')
            });
        if valid && seen.insert(tag.clone()) {
            out.push(tag);
            if out.len() == MAX_TAGS {
                break;
            }
        }
    }
    out
}

/// `raw` parsed as an absolute `http` or `https` URL with a host of at most
/// [`MAX_URL_CHARS`] — the only shape a link or source location may have, so
/// nothing else (a `javascript:` URL, say) ever reaches an `href`.
fn http_url(raw: &str) -> Option<url::Url> {
    let raw = raw.trim();
    if raw.chars().count() > MAX_URL_CHARS {
        return None;
    }
    let parsed = url::Url::parse(raw).ok()?;
    (matches!(parsed.scheme(), "http" | "https") && parsed.host_str().is_some()).then_some(parsed)
}

/// Backstage's character set for kinds, namespaces and names: letters,
/// digits and `-_.`, at most 63 characters. Anything else — quotes, spaces,
/// markup — is no entity ref, and would otherwise travel into graph labels.
fn is_ref_part(part: &str) -> bool {
    !part.is_empty()
        && part.len() <= 63
        && part
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The periphery half alone, as the #46 tests read it.
    fn component_resource_dependencies(
        yaml: &str,
        system: &str,
    ) -> Result<Vec<String>, CatalogError> {
        read_component(yaml, system).map(|c| c.resources)
    }

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
    fn component_refs_are_kept_apart_from_the_periphery() {
        let read = read_component(ORDERS, "orders-service").unwrap();
        assert_eq!(read.components, vec!["component:default/billing-service"]);
        assert_eq!(read.resources.len(), 3);
        assert_eq!(
            component_ref_name("component:default/billing-service"),
            Some("billing-service")
        );
        assert_eq!(component_ref_name("component:billing"), Some("billing"));
        assert_eq!(component_ref_name("resource:default/db"), None);
    }

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
    fn plugin_entities_in_the_same_file_are_accepted() {
        let yaml = format!(
            "apiVersion: scaffolder.backstage.io/v1beta3\nkind: Template\nmetadata:\n  name: new-service\n---\n{ORDERS}"
        );
        assert_eq!(
            component_resource_dependencies(&yaml, "orders-service").map(|r| r.len()),
            Ok(3)
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
        let lookalike = "apiVersion: notbackstage.io/v1\nkind: Component\nmetadata:\n  name: svc\n";
        assert!(matches!(
            component_resource_dependencies(lookalike, "svc"),
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
        let quoted = "apiVersion: backstage.io/v1alpha1\nkind: Component\nmetadata:\n  name: svc\nspec:\n  dependsOn: ['resource:default/db\"x']\n";
        assert!(matches!(
            component_resource_dependencies(quoted, "svc"),
            Err(CatalogError::MalformedDependsOn(_, _))
        ));
    }

    #[test]
    fn reads_owner_description_source_location_and_links() {
        let yaml = r#"
apiVersion: backstage.io/v1alpha1
kind: Component
metadata:
  name: orders-service
  description: "  Takes orders.  "
  annotations:
    backstage.io/source-location: url:https://github.com/acme/orders/tree/main/
  links:
    - url: https://wiki.acme.corp/orders
      title: Wiki
      type: docs
      icon: docs
    - url: https://jira.acme.corp/browse/ORD
spec:
  type: service
  owner: Team-Orders
"#;
        let context = read_component(yaml, "orders-service").unwrap().context;
        assert_eq!(
            context,
            SystemContext {
                owner: Some(SystemOwner {
                    kind: "group".to_string(),
                    name: "team-orders".to_string(),
                }),
                description: Some("Takes orders.".to_string()),
                source_location: Some("https://github.com/acme/orders/tree/main/".to_string()),
                links: vec![
                    SystemLink {
                        url: "https://wiki.acme.corp/orders".to_string(),
                        title: "Wiki".to_string(),
                        link_type: Some("docs".to_string()),
                        icon: Some("docs".to_string()),
                    },
                    SystemLink {
                        url: "https://jira.acme.corp/browse/ORD".to_string(),
                        title: "jira.acme.corp".to_string(),
                        link_type: None,
                        icon: None,
                    },
                ],
                tags: vec![],
            }
        );
    }

    #[test]
    fn tags_are_lower_cased_deduplicated_and_bounded() {
        let yaml = "apiVersion: backstage.io/v1alpha1\nkind: Component\nmetadata:\n  name: svc\n  tags:\n    - Java\n    - kafka\n    - java\n    - ' tier-1 '\n    - 'has space'\n    - 7\n    - c++\n";
        assert_eq!(
            read_component(yaml, "svc").unwrap().context.tags,
            vec!["java", "kafka", "tier-1", "c++"]
        );
        let many: String = (0..25).map(|i| format!("    - t{i}\n")).collect();
        let yaml = format!(
            "apiVersion: backstage.io/v1alpha1\nkind: Component\nmetadata:\n  name: svc\n  tags:\n{many}"
        );
        assert_eq!(read_component(&yaml, "svc").unwrap().context.tags.len(), 20);
        let not_a_list = "apiVersion: backstage.io/v1alpha1\nkind: Component\nmetadata:\n  name: svc\n  tags: java\n";
        assert!(
            read_component(not_a_list, "svc")
                .unwrap()
                .context
                .tags
                .is_empty()
        );
    }

    #[test]
    fn spec_system_is_read_as_a_lower_cased_name_and_other_kinds_are_not() {
        let with = |system: &str| {
            format!(
                "apiVersion: backstage.io/v1alpha1\nkind: Component\nmetadata:\n  name: svc\nspec:\n  system: {system}\n"
            )
        };
        assert_eq!(
            read_component(&with("Order-Management"), "svc")
                .unwrap()
                .system
                .as_deref(),
            Some("order-management")
        );
        assert_eq!(
            read_component(&with("system:acme/shop"), "svc")
                .unwrap()
                .system
                .as_deref(),
            Some("shop")
        );
        assert_eq!(
            read_component(&with("group:shop"), "svc").unwrap().system,
            None
        );
        assert_eq!(
            read_component(&with("'not a ref'"), "svc").unwrap().system,
            None
        );
        assert_eq!(
            read_component(ORDERS, "orders-service").unwrap().system,
            None
        );
    }

    #[test]
    fn a_component_without_context_fields_has_an_empty_context() {
        assert_eq!(
            read_component(ORDERS, "orders-service").unwrap().context,
            SystemContext {
                owner: Some(SystemOwner {
                    kind: "group".to_string(),
                    name: "team-a".to_string(),
                }),
                ..SystemContext::default()
            }
        );
        let bare = "apiVersion: backstage.io/v1alpha1\nkind: Component\nmetadata:\n  name: svc\n";
        assert_eq!(
            read_component(bare, "svc").unwrap().context,
            SystemContext::default()
        );
    }

    #[test]
    fn owner_refs_keep_their_kind_and_drop_the_namespace() {
        let owner = |raw: &str| {
            let yaml = format!(
                "apiVersion: backstage.io/v1alpha1\nkind: Component\nmetadata:\n  name: svc\nspec:\n  owner: {raw}\n"
            );
            read_component(&yaml, "svc").unwrap().context.owner
        };
        assert_eq!(
            owner("user:default/Alice"),
            Some(SystemOwner {
                kind: "user".to_string(),
                name: "alice".to_string(),
            })
        );
        assert_eq!(
            owner("group:shop/team-orders"),
            Some(SystemOwner {
                kind: "group".to_string(),
                name: "team-orders".to_string(),
            })
        );
        assert_eq!(owner("'not a ref'"), None);
        assert_eq!(owner("[a, b]"), None);
    }

    #[test]
    fn descriptions_are_trimmed_and_cut_with_an_ellipsis() {
        let describe = |text: &str| {
            let yaml = format!(
                "apiVersion: backstage.io/v1alpha1\nkind: Component\nmetadata:\n  name: svc\n  description: \"{text}\"\n"
            );
            read_component(&yaml, "svc").unwrap().context.description
        };
        assert_eq!(describe("   "), None);
        let exact = "ä".repeat(1000);
        assert_eq!(describe(&exact), Some(exact));
        let long = "ä".repeat(1001);
        let cut = describe(&long).unwrap();
        assert_eq!(cut.chars().count(), 1000);
        assert!(cut.ends_with('…'));
    }

    #[test]
    fn link_texts_are_cut_and_overlong_urls_are_dropped() {
        let exact = "ä".repeat(200);
        let long = "ä".repeat(201);
        let fits = format!("https://example.com/{}", "x".repeat(2048 - 20));
        let too_long = format!("{fits}x");
        assert_eq!(fits.len(), 2048);
        let yaml = format!(
            "apiVersion: backstage.io/v1alpha1\nkind: Component\nmetadata:\n  name: svc\n  \
             annotations:\n    backstage.io/source-location: url:{too_long}\n  links:\n    \
             - url: {fits}\n      title: \"{long}\"\n      type: \"{exact}\"\n      icon: \"{long}\"\n    \
             - url: {too_long}\n      title: dropped\n"
        );
        let context = read_component(&yaml, "svc").unwrap().context;
        assert_eq!(
            context.source_location, None,
            "a 2049-character URL is dropped"
        );
        assert_eq!(
            context.links.len(),
            1,
            "the link with the 2049-character URL is dropped"
        );
        let link = &context.links[0];
        assert_eq!(link.url, fits);
        assert_eq!(link.title.chars().count(), 200);
        assert!(link.title.ends_with('…'));
        assert_eq!(link.link_type.as_deref(), Some(exact.as_str()));
        let icon = link.icon.as_deref().unwrap();
        assert_eq!(icon.chars().count(), 200);
        assert!(icon.ends_with('…'));
    }

    #[test]
    fn only_http_urls_survive_as_source_location_or_link() {
        let yaml = r#"
apiVersion: backstage.io/v1alpha1
kind: Component
metadata:
  name: svc
  annotations:
    backstage.io/source-location: file:///srv/svc
  links:
    - url: javascript:alert(1)
      title: Bad
    - url: ftp://files.example/x
    - title: no url
    - just a string
    - url: HTTP://Example.COM/path
      title: "  "
"#;
        let context = read_component(yaml, "svc").unwrap().context;
        assert_eq!(context.source_location, None);
        assert_eq!(
            context.links,
            vec![SystemLink {
                url: "HTTP://Example.COM/path".to_string(),
                title: "example.com".to_string(),
                link_type: None,
                icon: None,
            }]
        );
    }

    #[test]
    fn links_beyond_the_twentieth_are_ignored() {
        let links: String = (1..=25)
            .map(|i| format!("    - url: https://l{i}.example\n"))
            .collect();
        let yaml = format!(
            "apiVersion: backstage.io/v1alpha1\nkind: Component\nmetadata:\n  name: svc\n  links:\n{links}"
        );
        let context = read_component(&yaml, "svc").unwrap().context;
        assert_eq!(context.links.len(), 20);
        assert_eq!(context.links[19].url, "https://l20.example");
    }
}
