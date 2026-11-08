//! Reading a Helm umbrella `Chart.yaml` (#76, ADR-0011): only the system
//! names its `dependencies` stand for — each dependency's `alias`, else its
//! `name`. Everything else in the file (version, repositories, conditions,
//! tags) is ignored; which names are Sanshain systems is decided by the
//! caller.

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
    dependencies: Option<Vec<Dependency>>,
}

#[derive(Deserialize)]
struct Dependency {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    alias: Option<String>,
}

/// A trimmed, non-blank value.
fn present(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|v| !v.is_empty())
}

/// The system names of a chart's dependencies, in the chart's order without
/// repeats: a dependency's `alias` when it has one, else its `name`.
pub fn dependency_names(chart_yaml: &str) -> Result<Vec<String>, ChartError> {
    let chart: Chart = crate::yaml::from_str(chart_yaml).map_err(ChartError::InvalidYaml)?;
    let dependencies = chart.dependencies.unwrap_or_default();
    if dependencies.is_empty() {
        return Err(ChartError::NoDependencies);
    }
    let mut names: Vec<String> = Vec::with_capacity(dependencies.len());
    for (index, dependency) in dependencies.iter().enumerate() {
        let name = present(dependency.alias.as_deref())
            .or_else(|| present(dependency.name.as_deref()))
            .ok_or(ChartError::UnnamedDependency(index.saturating_add(1)))?;
        if !names.iter().any(|n| n == name) {
            names.push(name.to_string());
        }
    }
    Ok(names)
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
