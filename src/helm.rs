//! Reading a Helm umbrella `Chart.yaml` (#76, ADR-0011): the system names
//! its `dependencies` stand for — each dependency's `alias`, else its
//! `name` — and, for a Product snapshot (san-1033), the chart's own
//! `version`, each dependency's `version` as written (a range stays a range)
//! and the sales name in the annotation `sanshain/sales-name`. Everything
//! else (repositories, conditions, tags) is ignored; which names are
//! Sanshain systems is decided by the caller.

use serde::Deserialize;

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum ChartError {
    #[error("Chart.yaml is not valid YAML: {0}")]
    InvalidYaml(String),
    #[error(
        "Chart.yaml lists no dependencies - paste the umbrella chart whose dependencies are the Product's systems"
    )]
    NoDependencies,
    #[error("dependency {0} of Chart.yaml has neither a name nor an alias")]
    UnnamedDependency(usize),
}

#[derive(Deserialize)]
struct Chart {
    #[serde(default)]
    version: Option<serde_json::Value>,
    #[serde(default)]
    annotations: Option<std::collections::BTreeMap<String, serde_json::Value>>,
    #[serde(default)]
    dependencies: Option<Vec<Dependency>>,
}

/// The annotation of an umbrella chart that names the release for sales.
pub const SALES_NAME_ANNOTATION: &str = "sanshain/sales-name";

/// One dependency of an umbrella chart: the system name and the chart
/// version the umbrella asks for, as written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChartMember {
    pub name: String,
    pub version: Option<String>,
}

/// What a Product snapshot reads from an umbrella chart.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChartRelease {
    /// The chart's own `version`, the Product version.
    pub version: Option<String>,
    /// The `sanshain/sales-name` annotation.
    pub sales_name: Option<String>,
    /// The dependencies in the chart's order without repeated names.
    pub members: Vec<ChartMember>,
}

/// A scalar as text: YAML reads `version: 1.4` as a number.
fn scalar_text(value: Option<&serde_json::Value>) -> Option<String> {
    match value? {
        serde_json::Value::String(s) => present(Some(s)).map(str::to_string),
        serde_json::Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

#[derive(Deserialize)]
struct Dependency {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    alias: Option<String>,
    #[serde(default)]
    version: Option<serde_json::Value>,
}

/// A trimmed, non-blank value.
fn present(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|v| !v.is_empty())
}

/// The system names of a chart's dependencies, in the chart's order without
/// repeats: a dependency's `alias` when it has one, else its `name`.
pub fn dependency_names(chart_yaml: &str) -> Result<Vec<String>, ChartError> {
    Ok(read_chart(chart_yaml)?
        .members
        .into_iter()
        .map(|m| m.name)
        .collect())
}

/// The chart's version, sales name and members with their chart versions.
pub fn read_chart(chart_yaml: &str) -> Result<ChartRelease, ChartError> {
    let chart: Chart = crate::yaml::from_str(chart_yaml).map_err(ChartError::InvalidYaml)?;
    let dependencies = chart.dependencies.unwrap_or_default();
    if dependencies.is_empty() {
        return Err(ChartError::NoDependencies);
    }
    let mut members: Vec<ChartMember> = Vec::with_capacity(dependencies.len());
    for (index, dependency) in dependencies.iter().enumerate() {
        let name = present(dependency.alias.as_deref())
            .or_else(|| present(dependency.name.as_deref()))
            .ok_or(ChartError::UnnamedDependency(index.saturating_add(1)))?;
        if !members.iter().any(|m| m.name == name) {
            members.push(ChartMember {
                name: name.to_string(),
                version: scalar_text(dependency.version.as_ref()),
            });
        }
    }
    Ok(ChartRelease {
        version: scalar_text(chart.version.as_ref()),
        sales_name: scalar_text(
            chart
                .annotations
                .as_ref()
                .and_then(|a| a.get(SALES_NAME_ANNOTATION)),
        ),
        members,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_alias_wins_over_the_name_and_repeats_collapse() {
        let chart = r#"
apiVersion: v2
name: shop
version: 1.4.0
dependencies:
  - name: orders-chart
    alias: orders
    version: 2.1.0
    repository: oci://charts.example.test
  - name: billing
    version: 1.0.0
  - name: redis
    alias: "  "
  - name: billing
"#;
        assert_eq!(
            dependency_names(chart).unwrap(),
            ["orders", "billing", "redis"]
        );
    }

    #[test]
    fn the_release_reads_versions_and_the_sales_name() {
        let chart = r#"
apiVersion: v2
name: shop
version: 1.3.0
annotations:
  sanshain/sales-name: "Bulbao ]|[ - alpha"
dependencies:
  - name: orders-chart
    alias: orders
    version: "~2.1.0"
  - name: billing
    version: 1.4
  - name: redis
"#;
        let release = read_chart(chart).unwrap();
        assert_eq!(release.version.as_deref(), Some("1.3.0"));
        assert_eq!(release.sales_name.as_deref(), Some("Bulbao ]|[ - alpha"));
        assert_eq!(
            release.members,
            vec![
                ChartMember {
                    name: "orders".to_string(),
                    version: Some("~2.1.0".to_string())
                },
                ChartMember {
                    name: "billing".to_string(),
                    version: Some("1.4".to_string())
                },
                ChartMember {
                    name: "redis".to_string(),
                    version: None
                },
            ]
        );
        let bare = read_chart("dependencies:\n  - name: a\n").unwrap();
        assert_eq!(bare.version, None);
        assert_eq!(bare.sales_name, None);
    }

    #[test]
    fn a_chart_without_dependencies_is_refused() {
        assert_eq!(
            dependency_names("apiVersion: v2\nname: shop\n"),
            Err(ChartError::NoDependencies)
        );
        assert_eq!(
            dependency_names("name: shop\ndependencies: []\n"),
            Err(ChartError::NoDependencies)
        );
    }

    #[test]
    fn malformed_yaml_and_unnamed_dependencies_are_refused() {
        assert!(matches!(
            dependency_names("dependencies: [unclosed"),
            Err(ChartError::InvalidYaml(_))
        ));
        assert!(matches!(
            dependency_names("dependencies: nope"),
            Err(ChartError::InvalidYaml(_))
        ));
        assert_eq!(
            dependency_names("dependencies:\n  - name: a\n  - version: 1.0.0\n"),
            Err(ChartError::UnnamedDependency(2))
        );
    }
}
