//! What an AI assistant needs to set a system up with Sanshain (#88): the
//! facts about the five official clients, kept here as data. The MCP setup
//! tools answer from them; `tests/site_clients_table_test.rs` checks them
//! against the client table of `site/developer/clients.html`, so the page
//! and the answers cannot drift apart.

use crate::domain::models::{AnnotationEntry, AnnotationScope, Annotations, ApiType, AppError};
use serde::Serialize;

/// The address of this Sanshain instance as a setup answer names it:
/// `PUBLIC_BASE_URL` when the operator set it, otherwise the address the
/// caller reached, marked `derived` because a proxy may have rewritten it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct InstanceUrl {
    pub url: String,
    pub derived: bool,
}

/// The request headers the instance address is derived from.
#[derive(Clone, Copy, Debug, Default)]
pub struct RequestAddress<'a> {
    pub forwarded_proto: Option<&'a str>,
    pub forwarded_host: Option<&'a str>,
    pub host: Option<&'a str>,
}

/// The first entry of a header a chain of proxies may have appended to.
fn first_entry(value: Option<&str>) -> Option<&str> {
    value
        .and_then(|v| v.split(',').next())
        .map(str::trim)
        .filter(|v| !v.is_empty())
}

/// `configured` (`PUBLIC_BASE_URL`, already validated at start) when set;
/// else `X-Forwarded-Proto` (`http` when absent or neither `http` nor
/// `https`) and `X-Forwarded-Host`, falling back to `Host`. A host holding
/// anything but the characters of a host and port is not used; without a
/// usable host the address is unknown.
pub fn instance_url(configured: Option<&str>, request: &RequestAddress) -> Option<InstanceUrl> {
    if let Some(url) = configured.map(|u| u.trim().trim_end_matches('/'))
        && !url.is_empty()
    {
        return Some(InstanceUrl {
            url: url.to_string(),
            derived: false,
        });
    }
    let scheme = match first_entry(request.forwarded_proto).map(str::to_ascii_lowercase) {
        Some(p) if p == "https" => "https",
        _ => "http",
    };
    let host = first_entry(request.forwarded_host).or_else(|| first_entry(request.host))?;
    let plausible = host
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | ':' | '[' | ']'));
    plausible.then(|| InstanceUrl {
        url: format!("{scheme}://{host}"),
        derived: true,
    })
}

/// One `provides` entry of a drafted `sanshain.yaml`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DraftProvide {
    pub file: String,
    pub api_type: Option<ApiType>,
    pub retired: bool,
}

/// One endpoint of a drafted `requires` entry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DraftEndpoint {
    pub method: String,
    pub path: String,
}

/// One `requires` entry of a drafted `sanshain.yaml`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DraftRequire {
    pub service_name: String,
    pub version: String,
    pub output_directory: String,
    pub endpoints: Vec<DraftEndpoint>,
    pub api_type: Option<ApiType>,
    /// `format: openapi`: the payload schemas of an AsyncAPI requirement as
    /// an OpenAPI document.
    pub format_openapi: bool,
}

/// What a caller names for its `sanshain.yaml`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DraftRequest {
    pub service_name: String,
    pub catalog_info: Option<String>,
    pub auto_ga: bool,
    pub annotations: Vec<AnnotationEntry>,
    pub provides: Vec<DraftProvide>,
    pub requires: Vec<DraftRequire>,
}

/// The file as the clients read it, in the order the documentation writes
/// it. Defaults are left out, so the draft says only what was asked for.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SanshainYaml<'a> {
    service_name: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    catalog_info: Option<&'a str>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    auto_ga: bool,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    annotations: Vec<YamlAnnotation<'a>>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    provides: Vec<YamlProvide<'a>>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    requires: Vec<YamlRequire<'a>>,
}

#[derive(Serialize)]
struct YamlAnnotation<'a> {
    scope: AnnotationScope,
    name: &'a str,
    value: &'a str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct YamlProvide<'a> {
    file: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    api_type: Option<ApiType>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    retired: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct YamlRequire<'a> {
    service_name: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    api_type: Option<ApiType>,
    #[serde(skip_serializing_if = "Option::is_none")]
    format: Option<&'static str>,
    version: String,
    output_directory: &'a str,
    endpoints: Vec<YamlEndpoint<'a>>,
}

#[derive(Serialize)]
struct YamlEndpoint<'a> {
    method: &'a str,
    path: &'a str,
}

fn required<'a>(value: &'a str, what: &str) -> Result<&'a str, AppError> {
    let value = value.trim();
    if value.is_empty() {
        return Err(AppError::BadRequest(format!("{what} must not be empty")));
    }
    Ok(value)
}

/// An exact pin, `MAJOR.MINOR.PATCH` and nothing else: the file is the
/// contract a reader trusts, so it carries the stored form.
fn exact_pin(raw: &str, producer: &str) -> Result<String, AppError> {
    let raw = raw.trim();
    let three_numbers = raw.split('.').count() == 3
        && raw
            .split('.')
            .all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()));
    let parsed = three_numbers
        .then(|| raw.parse::<crate::domain::models::SemVer>().ok())
        .flatten();
    parsed.map(|v| v.to_string()).ok_or_else(|| {
        AppError::BadRequest(format!(
            "the pin of '{producer}' is '{raw}': a require pins an exact MAJOR.MINOR.PATCH, no ranges and no latest"
        ))
    })
}

/// A `sanshain.yaml` from what the caller names. Refuses what every client
/// or the service would refuse later: an empty name, file or path, a pin
/// that is not exact, a require without endpoints, `format: openapi`
/// outside AsyncAPI, and annotations out of the service's bounds.
pub fn draft_sanshain_yaml(request: &DraftRequest) -> Result<String, AppError> {
    let service_name = required(&request.service_name, "service_name")?;
    Annotations::from_entries(&request.annotations)
        .map_err(|e| AppError::BadRequest(format!("annotations: {e}")))?;
    let annotations = request
        .annotations
        .iter()
        .map(|a| YamlAnnotation {
            scope: a.scope,
            name: a.name.trim(),
            value: a.value.trim(),
        })
        .collect();
    let provides = request
        .provides
        .iter()
        .map(|p| {
            Ok(YamlProvide {
                file: required(&p.file, "a provide's file")?,
                api_type: p.api_type.filter(|t| *t != ApiType::OpenApi),
                retired: p.retired,
            })
        })
        .collect::<Result<Vec<_>, AppError>>()?;
    let requires = request
        .requires
        .iter()
        .map(|r| {
            let producer = required(&r.service_name, "a require's service_name")?;
            if r.endpoints.is_empty() {
                return Err(AppError::BadRequest(format!(
                    "the require of '{producer}' names no endpoints"
                )));
            }
            if r.format_openapi && r.api_type != Some(ApiType::AsyncApi) {
                return Err(AppError::BadRequest(format!(
                    "the require of '{producer}': format openapi only goes with api_type asyncapi"
                )));
            }
            let endpoints = r
                .endpoints
                .iter()
                .map(|e| {
                    Ok(YamlEndpoint {
                        method: required(&e.method, "an endpoint's method")?,
                        path: required(&e.path, "an endpoint's path")?,
                    })
                })
                .collect::<Result<Vec<_>, AppError>>()?;
            Ok(YamlRequire {
                service_name: producer,
                api_type: r.api_type.filter(|t| *t != ApiType::OpenApi),
                format: r.format_openapi.then_some("openapi"),
                version: exact_pin(&r.version, producer)?,
                output_directory: required(&r.output_directory, "a require's output_directory")?,
                endpoints,
            })
        })
        .collect::<Result<Vec<_>, AppError>>()?;
    let file = SanshainYaml {
        service_name,
        catalog_info: request
            .catalog_info
            .as_deref()
            .map(str::trim)
            .filter(|c| !c.is_empty()),
        auto_ga: request.auto_ga,
        annotations,
        provides,
        requires,
    };
    crate::yaml::to_string(&file)
        .map_err(|e| AppError::Internal(format!("could not write sanshain.yaml: {e}")))
}

/// The build tool a system builds with, one per official client.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum BuildTool {
    Maven,
    Cargo,
    Go,
    Npm,
    Conan,
}

impl BuildTool {
    pub fn as_str(&self) -> &'static str {
        match self {
            BuildTool::Maven => "maven",
            BuildTool::Cargo => "cargo",
            BuildTool::Go => "go",
            BuildTool::Npm => "npm",
            BuildTool::Conan => "conan",
        }
    }
}

/// One official client: where it lives, how it is installed and the command
/// lines a build runs. Every command is complete as written; `R1` stands for
/// the sanshain-branch name of a release pipeline.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct ClientFacts {
    pub build_tool: BuildTool,
    pub name: &'static str,
    pub repository: &'static str,
    pub version: &'static str,
    /// The install line, or for Maven the plugin coordinates in `pom.xml`.
    pub install: &'static str,
    pub provide: &'static str,
    pub provide_ga: &'static str,
    pub provide_trunk: &'static str,
    pub provide_tag: &'static str,
    pub require: &'static str,
    pub catalog: &'static str,
    /// How a build fills an annotation placeholder `{{NAME}}`; the
    /// environment variable `SANSHAIN_SET_NAME` works with every client.
    pub set_placeholder: &'static str,
    /// The dry run of a provide, for the clients that send `dry_run`.
    pub dry_run: Option<&'static str>,
    /// Where required documents conventionally go, one folder per Producer.
    pub output_root: &'static str,
}

/// The five official clients, in the order `clients.html` lists them.
pub const CLIENTS: [ClientFacts; 5] = [
    ClientFacts {
        build_tool: BuildTool::Maven,
        name: "sanshain-maven-plugin",
        repository: "https://github.com/paxel/sanshain-maven-plugin",
        version: "2.5.0",
        install: "io.github.paxel.sanshain:sanshain-maven-plugin:2.5.0",
        provide: "mvn sanshain:provide",
        provide_ga: "mvn sanshain:provide -Dsanshain.ga=true",
        provide_trunk: "mvn sanshain:provide -Dsanshain.trunk=true",
        provide_tag: "mvn sanshain:provide -Dsanshain.tag=R1",
        require: "mvn sanshain:require",
        catalog: "mvn sanshain:catalog",
        set_placeholder: "-DNAME=value",
        dry_run: Some("mvn sanshain:provide -Dsanshain.dry.run=true"),
        output_root: "target/generated-sources/sanshain",
    },
    ClientFacts {
        build_tool: BuildTool::Cargo,
        name: "cargo-sanshain",
        repository: "https://github.com/paxel/sanshain-rs",
        version: "4.0.0",
        install: "cargo install cargo-sanshain",
        provide: "cargo sanshain provide",
        provide_ga: "cargo sanshain provide --ga",
        provide_trunk: "cargo sanshain provide --trunk",
        provide_tag: "cargo sanshain provide --tag R1",
        require: "cargo sanshain require",
        catalog: "cargo sanshain catalog",
        set_placeholder: "--set NAME=value",
        dry_run: None,
        output_root: "generated",
    },
    ClientFacts {
        build_tool: BuildTool::Go,
        name: "sanshain-go",
        repository: "https://github.com/paxel/sanshain-go",
        version: "2.5.0",
        install: "go install github.com/paxel/sanshain-go/v2/cmd/sanshain-go@latest",
        provide: "sanshain-go provide",
        provide_ga: "sanshain-go --ga provide",
        provide_trunk: "sanshain-go --trunk provide",
        provide_tag: "sanshain-go --tag R1 provide",
        require: "sanshain-go require",
        catalog: "sanshain-go catalog",
        set_placeholder: "--set NAME=value",
        dry_run: None,
        output_root: "generated",
    },
    ClientFacts {
        build_tool: BuildTool::Npm,
        name: "sanshain",
        repository: "https://github.com/paxel/sanshain-js",
        version: "2.6.0",
        install: "npm install --save-dev sanshain",
        provide: "npx sanshain provide",
        provide_ga: "npx sanshain provide --ga",
        provide_trunk: "npx sanshain provide --trunk",
        provide_tag: "npx sanshain provide --tag R1",
        require: "npx sanshain require",
        catalog: "npx sanshain catalog",
        set_placeholder: "--set NAME=value",
        dry_run: Some("npx sanshain provide --dry-run"),
        output_root: "generated",
    },
    ClientFacts {
        build_tool: BuildTool::Conan,
        name: "sanshain-conan",
        repository: "https://github.com/paxel/sanshain-conan",
        version: "2.5.0",
        install: "pip install sanshain-conan",
        provide: "sanshain-conan provide",
        provide_ga: "sanshain-conan --ga provide",
        provide_trunk: "sanshain-conan --trunk provide",
        provide_tag: "sanshain-conan --tag R1 provide",
        require: "sanshain-conan require",
        catalog: "sanshain-conan catalog",
        set_placeholder: "--set NAME=value",
        dry_run: None,
        output_root: "generated",
    },
];

/// The official client for `build_tool`.
pub fn client(build_tool: BuildTool) -> &'static ClientFacts {
    match build_tool {
        BuildTool::Maven => &CLIENTS[0],
        BuildTool::Cargo => &CLIENTS[1],
        BuildTool::Go => &CLIENTS[2],
        BuildTool::Npm => &CLIENTS[3],
        BuildTool::Conan => &CLIENTS[4],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_build_tool_has_its_own_client() {
        for tool in [
            BuildTool::Maven,
            BuildTool::Cargo,
            BuildTool::Go,
            BuildTool::Npm,
            BuildTool::Conan,
        ] {
            assert_eq!(client(tool).build_tool, tool, "{}", tool.as_str());
        }
        assert_eq!(client(BuildTool::Cargo).name, "cargo-sanshain");
        assert_eq!(client(BuildTool::Npm).version, "2.6.0");
    }

    #[test]
    fn every_command_names_its_client() {
        for c in &CLIENTS {
            let program = c
                .provide
                .split_whitespace()
                .next()
                .unwrap_or_default()
                .to_string();
            for command in [
                c.provide_ga,
                c.provide_trunk,
                c.provide_tag,
                c.require,
                c.catalog,
            ] {
                assert!(command.starts_with(&program), "{}: {command}", c.name);
            }
            assert!(c.provide_ga.contains("ga"), "{}", c.name);
            assert!(c.provide_trunk.contains("trunk"), "{}", c.name);
            assert!(c.provide_tag.contains("R1"), "{}", c.name);
            assert!(c.set_placeholder.contains("NAME=value"), "{}", c.name);
        }
    }

    #[test]
    fn only_maven_and_npm_send_a_dry_run() {
        let with: Vec<&str> = CLIENTS
            .iter()
            .filter(|c| c.dry_run.is_some())
            .map(|c| c.build_tool.as_str())
            .collect();
        assert_eq!(with, ["maven", "npm"]);
    }

    #[test]
    fn instance_url_prefers_the_configured_base_url() {
        let request = RequestAddress {
            forwarded_proto: Some("https"),
            forwarded_host: Some("proxy.corp"),
            host: Some("internal:3000"),
        };
        assert_eq!(
            instance_url(Some("https://sanshain.corp/"), &request),
            Some(InstanceUrl {
                url: "https://sanshain.corp".to_string(),
                derived: false
            })
        );
        assert_eq!(
            instance_url(Some("  "), &request),
            Some(InstanceUrl {
                url: "https://proxy.corp".to_string(),
                derived: true
            }),
            "a blank configuration counts as unset"
        );
    }

    #[test]
    fn instance_url_is_derived_from_the_forwarded_headers_then_the_host() {
        let derived = |proto, forwarded, host| {
            instance_url(
                None,
                &RequestAddress {
                    forwarded_proto: proto,
                    forwarded_host: forwarded,
                    host,
                },
            )
            .map(|u| (u.url, u.derived))
        };
        assert_eq!(
            derived(None, None, Some("sanshain.example:3000")),
            Some(("http://sanshain.example:3000".to_string(), true))
        );
        assert_eq!(
            derived(Some("HTTPS, http"), Some("a.corp, b.internal"), Some("x")),
            Some(("https://a.corp".to_string(), true)),
            "the first proxy's entry is the caller's"
        );
        assert_eq!(
            derived(Some("gopher"), None, Some("[::1]:3000")),
            Some(("http://[::1]:3000".to_string(), true))
        );
        assert_eq!(derived(Some("https"), None, None), None);
        assert_eq!(derived(None, Some(" "), Some("")), None);
        assert_eq!(
            derived(None, Some("evil.corp/phish?x="), Some("ok")),
            None,
            "a host with a path is not an address"
        );
    }

    fn orders_require() -> DraftRequire {
        DraftRequire {
            service_name: "orders".to_string(),
            version: "1.2.0".to_string(),
            output_directory: "generated/orders".to_string(),
            endpoints: vec![
                DraftEndpoint {
                    method: "GET".to_string(),
                    path: "/orders/{id}".to_string(),
                },
                DraftEndpoint {
                    method: "POST".to_string(),
                    path: "/orders".to_string(),
                },
            ],
            api_type: None,
            format_openapi: false,
        }
    }

    #[test]
    fn draft_writes_only_what_was_named_in_the_documented_order() {
        let yaml = draft_sanshain_yaml(&DraftRequest {
            service_name: "billing".to_string(),
            catalog_info: Some("catalog-info.yaml".to_string()),
            auto_ga: true,
            annotations: vec![
                AnnotationEntry {
                    scope: AnnotationScope::Version,
                    name: "requirements".to_string(),
                    value: "https://wiki.corp/{{ID}}".to_string(),
                },
                AnnotationEntry {
                    scope: AnnotationScope::System,
                    name: "job".to_string(),
                    value: "{{JOB_URL}}".to_string(),
                },
            ],
            provides: vec![
                DraftProvide {
                    file: "openapi.yaml".to_string(),
                    api_type: Some(ApiType::OpenApi),
                    retired: false,
                },
                DraftProvide {
                    file: "billing.proto".to_string(),
                    api_type: Some(ApiType::Proto),
                    retired: true,
                },
            ],
            requires: vec![
                orders_require(),
                DraftRequire {
                    service_name: "events".to_string(),
                    version: "2.0.0".to_string(),
                    output_directory: "generated/events".to_string(),
                    endpoints: vec![DraftEndpoint {
                        method: "PUB".to_string(),
                        path: "orders.created".to_string(),
                    }],
                    api_type: Some(ApiType::AsyncApi),
                    format_openapi: true,
                },
            ],
        })
        .unwrap();
        let read: crate::yaml::Value = crate::yaml::from_str(&yaml).unwrap();
        let keys: Vec<&str> = read
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        assert_eq!(
            keys,
            [
                "serviceName",
                "catalogInfo",
                "autoGa",
                "annotations",
                "provides",
                "requires"
            ],
            "{yaml}"
        );
        assert_eq!(read["autoGa"], true);
        assert_eq!(read["annotations"][0]["scope"], "version");
        assert_eq!(read["annotations"][0]["value"], "https://wiki.corp/{{ID}}");
        assert_eq!(
            read["annotations"][1],
            serde_json::json!({ "scope": "system", "name": "job", "value": "{{JOB_URL}}" }),
            "a value that starts like a flow mapping is quoted: {yaml}"
        );
        assert_eq!(
            read["provides"],
            serde_json::json!([
                { "file": "openapi.yaml" },
                { "file": "billing.proto", "apiType": "proto", "retired": true }
            ]),
            "openapi is the default and not written"
        );
        assert_eq!(
            read["requires"][0],
            serde_json::json!({
                "serviceName": "orders", "version": "1.2.0",
                "outputDirectory": "generated/orders",
                "endpoints": [
                    { "method": "GET", "path": "/orders/{id}" },
                    { "method": "POST", "path": "/orders" }
                ]
            })
        );
        assert_eq!(read["requires"][1]["apiType"], "asyncapi");
        assert_eq!(read["requires"][1]["format"], "openapi");
    }

    #[test]
    fn draft_of_a_bare_name_is_one_line() {
        let yaml = draft_sanshain_yaml(&DraftRequest {
            service_name: " web ".to_string(),
            ..Default::default()
        })
        .unwrap();
        assert_eq!(yaml.trim(), "serviceName: web");
    }

    #[test]
    fn draft_refuses_what_the_clients_or_the_service_would_refuse() {
        let refused = |request: DraftRequest| match draft_sanshain_yaml(&request) {
            Err(AppError::BadRequest(message)) => message,
            other => panic!("expected a refusal, got {other:?}"),
        };
        assert_eq!(
            refused(DraftRequest::default()),
            "service_name must not be empty"
        );
        for pin in ["1.2", "latest", "^1.2.0", "1.2.x", "v1.2.0"] {
            let mut require = orders_require();
            require.version = pin.to_string();
            let message = refused(DraftRequest {
                service_name: "web".to_string(),
                requires: vec![require],
                ..Default::default()
            });
            assert!(
                message.contains("exact MAJOR.MINOR.PATCH"),
                "{pin}: {message}"
            );
        }
        let mut require = orders_require();
        require.endpoints.clear();
        assert!(
            refused(DraftRequest {
                service_name: "web".to_string(),
                requires: vec![require],
                ..Default::default()
            })
            .contains("names no endpoints")
        );
        let mut require = orders_require();
        require.format_openapi = true;
        assert!(
            refused(DraftRequest {
                service_name: "web".to_string(),
                requires: vec![require],
                ..Default::default()
            })
            .contains("only goes with api_type asyncapi")
        );
        assert!(
            refused(DraftRequest {
                service_name: "web".to_string(),
                annotations: vec![AnnotationEntry {
                    scope: AnnotationScope::System,
                    name: "bad name".to_string(),
                    value: "x".to_string(),
                }],
                ..Default::default()
            })
            .starts_with("annotations: name 'bad name'")
        );
        assert_eq!(
            refused(DraftRequest {
                service_name: "web".to_string(),
                provides: vec![DraftProvide {
                    file: " ".to_string(),
                    api_type: None,
                    retired: false,
                }],
                ..Default::default()
            }),
            "a provide's file must not be empty"
        );
    }
}
