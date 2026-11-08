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

/// Which side of the catalog the system takes: most systems are both.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SetupRole {
    Producer,
    Consumer,
    Both,
}

impl SetupRole {
    fn provides(self) -> bool {
        matches!(self, SetupRole::Producer | SetupRole::Both)
    }
    fn requires(self) -> bool {
        matches!(self, SetupRole::Consumer | SetupRole::Both)
    }
}

/// The CI the system's pipelines run on. The first three fill a provide's
/// origin from their own variables; every other CI does not.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CiSystem {
    GithubActions,
    GitlabCi,
    Jenkins,
    Other,
}

/// A file the assistant writes or merges, with what to do with it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct FileSnippet {
    pub file: &'static str,
    pub action: &'static str,
    pub content: String,
}

/// One command line and when a build runs it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct CommandLine {
    pub purpose: &'static str,
    pub run: String,
    pub when: &'static str,
}

/// A service version a feature of the client needs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct MinService {
    pub feature: &'static str,
    pub version: &'static str,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ClientGuide {
    pub name: &'static str,
    pub repository: &'static str,
    pub version: &'static str,
    pub install: &'static str,
    /// The build file the client is wired into, when it has one.
    pub integration: Option<FileSnippet>,
    pub service: &'static str,
    pub min_service: [MinService; 3],
}

/// Where the build's token comes from. Never the MCP token.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct TokenGuide {
    pub scope: &'static str,
    pub mint_at: String,
    pub minted_by: &'static str,
    pub not_the_mcp_token: bool,
    /// On a developer's machine: a file for Maven, the environment else.
    pub local_env: Option<&'static str>,
    pub local_file: Option<FileSnippet>,
    pub ci_env: &'static str,
    pub ci_owner: &'static str,
    /// The role the token's user needs to provide GA (also with `autoGa`).
    pub ga_role: Option<&'static str>,
}

/// One environment variable a build sets.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Variable {
    pub name: &'static str,
    pub value: Option<String>,
    pub when: &'static str,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct CiGuide {
    pub token_secret: &'static str,
    pub url_variable: &'static str,
    /// The runner variables the clients read the origin from; empty when
    /// the build sets `SANSHAIN_BUILD_URL` and `SANSHAIN_CHANGE_URL` itself.
    pub origin_from: &'static [&'static str],
    pub snippet: Option<String>,
}

/// Everything an assistant needs to connect one system: install, commands,
/// variables, the token and its place, the address and a starting file.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct SetupGuide {
    pub role: SetupRole,
    pub build_tool: BuildTool,
    pub ci: Option<CiSystem>,
    pub instance: Option<InstanceUrl>,
    pub client: ClientGuide,
    pub commands: Vec<CommandLine>,
    pub token: TokenGuide,
    pub variables: Vec<Variable>,
    pub ci_setup: Option<CiGuide>,
    pub sanshain_yaml: String,
    /// The placeholder values in `sanshain_yaml` to replace.
    pub replace_in_yaml: Vec<String>,
    pub next_tools: [&'static str; 4],
    pub docs: [&'static str; 3],
}

const MIN_SERVICE: [MinService; 3] = [
    MinService {
        feature: "catalog",
        version: "2.4.0",
    },
    MinService {
        feature: "origin",
        version: "2.9.0",
    },
    MinService {
        feature: "annotations",
        version: "2.13.0",
    },
];

const DOCS: [&str; 3] = [
    "https://paxel.github.io/sanshain-service/developer/clients.html",
    "https://paxel.github.io/sanshain-service/developer/sanshain-yaml.html",
    "https://paxel.github.io/sanshain-service/developer/ci-integration.html",
];

/// Where the build tool's projects keep their spec by convention.
fn spec_file(build_tool: BuildTool) -> &'static str {
    match build_tool {
        BuildTool::Maven => "src/main/resources/openapi.yaml",
        _ => "openapi.yaml",
    }
}

fn maven_plugin(facts: &ClientFacts, role: SetupRole) -> String {
    let mut goals = String::new();
    if role.provides() {
        goals.push_str("        <goal>provide</goal>\n");
    }
    if role.requires() {
        goals.push_str("        <goal>require</goal>\n");
    }
    format!(
        "<plugin>\n  <groupId>io.github.paxel.sanshain</groupId>\n  \
         <artifactId>sanshain-maven-plugin</artifactId>\n  <version>{}</version>\n  \
         <executions>\n    <execution>\n      <phase>initialize</phase>\n      <goals>\n\
         {goals}      </goals>\n    </execution>\n  </executions>\n</plugin>\n",
        facts.version
    )
}

fn integration(facts: &ClientFacts, role: SetupRole) -> Option<FileSnippet> {
    match facts.build_tool {
        BuildTool::Maven => Some(FileSnippet {
            file: "pom.xml",
            action: "add to <build><plugins>",
            content: maven_plugin(facts, role),
        }),
        BuildTool::Npm if role.requires() => Some(FileSnippet {
            file: "package.json",
            action: "merge into scripts",
            content: "\"scripts\": { \"prebuild\": \"sanshain require\" }\n".to_string(),
        }),
        BuildTool::Conan if role.requires() => Some(FileSnippet {
            file: "conanfile.py",
            action: "call in generate(); runs during conan install",
            content: "from conan import ConanFile\nfrom sanshainconan import Sanshain\n\n\
                      class MyProject(ConanFile):\n    def generate(self):\n        \
                      Sanshain(self).require()\n"
                .to_string(),
        }),
        _ => None,
    }
}

fn maven_settings(url: &str) -> FileSnippet {
    FileSnippet {
        file: "~/.m2/settings.xml",
        action: "add the <server> to <servers>; the password is the token",
        content: format!(
            "<settings>\n  <servers>\n    <server>\n      <id>sanshain</id>\n      \
             <username>ignored</username>\n      <password>san_…</password>\n      \
             <configuration>\n        <sanshainUrl>{url}</sanshainUrl>\n      \
             </configuration>\n    </server>\n  </servers>\n</settings>\n"
        ),
    }
}

fn commands(facts: &ClientFacts, role: SetupRole) -> Vec<CommandLine> {
    let line = |purpose, run: &str, when| CommandLine {
        purpose,
        run: run.to_string(),
        when,
    };
    let mut out = Vec::new();
    if role.requires() {
        out.push(line(
            "require",
            facts.require,
            "every build, before code generation",
        ));
    }
    if role.provides() {
        out.push(line("provide", facts.provide, "any build: a snapshot"));
        out.push(line(
            "provide_trunk",
            facts.provide_trunk,
            "default-branch CI: a snapshot, GA with autoGa",
        ));
        out.push(line(
            "provide_tag",
            facts.provide_tag,
            "release or hotfix pipeline of sanshain-branch R1: a snapshot, GA with autoGa",
        ));
        out.push(line(
            "provide_ga",
            facts.provide_ga,
            "release pipelines without autoGa; the token's user needs releaser",
        ));
        if let Some(dry_run) = facts.dry_run {
            out.push(line("provide_dry_run", dry_run, "pull requests"));
        }
    }
    out.push(line(
        "catalog",
        facts.catalog,
        "with catalogInfo in sanshain.yaml, as its own CI step",
    ));
    out.push(CommandLine {
        purpose: "fill_placeholder",
        run: format!("{} or SANSHAIN_SET_NAME=value", facts.set_placeholder),
        when: "annotations with {{NAME}} placeholders",
    });
    out
}

fn variables(role: SetupRole, ci: Option<CiSystem>, url: Option<&str>) -> Vec<Variable> {
    let var = |name, value: Option<&str>, when| Variable {
        name,
        value: value.map(str::to_string),
        when,
    };
    let mut out = vec![
        var("SANSHAIN_URL", url, "every build"),
        var("SANSHAIN_TOKEN", Some("san_…"), "every build; a secret"),
    ];
    let Some(ci) = ci else {
        return out;
    };
    out.push(var(
        "SANSHAIN_TRUNK",
        Some("true"),
        "default-branch pipeline only",
    ));
    out.push(var(
        "SANSHAIN_TAG",
        Some("<sanshain-branch>"),
        "release and hotfix pipelines only",
    ));
    if role.provides() {
        out.push(var(
            "SANSHAIN_GA",
            Some("true"),
            "release pipelines only, unless autoGa; false keeps a build a snapshot",
        ));
        if ci == CiSystem::Other {
            out.push(var(
                "SANSHAIN_BUILD_URL",
                None,
                "this CI's URL of the build",
            ));
            out.push(var(
                "SANSHAIN_CHANGE_URL",
                None,
                "this CI's URL of the pull or merge request",
            ));
        }
    }
    out
}

/// The GitHub Actions steps the docs show: the JavaScript client's Action,
/// or a run step with the switches in the environment.
fn github_steps(facts: &ClientFacts, role: SetupRole) -> String {
    const ON_MAIN: &str = "${{ github.ref == 'refs/heads/main' }}";
    const ENV: &str = "    SANSHAIN_URL: ${{ vars.SANSHAIN_URL }}\n    \
                       SANSHAIN_TOKEN: ${{ secrets.SANSHAIN_TOKEN }}\n";
    let mut steps = String::new();
    let mut step = |command: &str, run: &str, ga: bool| {
        let ga_line = |indent: &str, key: &str| {
            if ga {
                format!("{indent}{key}: {ON_MAIN}\n")
            } else {
                String::new()
            }
        };
        if facts.build_tool == BuildTool::Npm {
            steps.push_str(&format!(
                "- uses: paxel/sanshain-js@main\n  with:\n    command: {command}\n{}    \
                 trunk: {ON_MAIN}\n  env:\n{ENV}",
                ga_line("    ", "ga")
            ));
        } else {
            steps.push_str(&format!(
                "- run: {run}\n  env:\n{ENV}    SANSHAIN_TRUNK: {ON_MAIN}\n{}",
                ga_line("    ", "SANSHAIN_GA")
            ));
        }
    };
    if role.requires() {
        step("require", facts.require, false);
    }
    if role.provides() {
        step("provide", facts.provide, true);
    }
    steps
}

fn ci_guide(facts: &ClientFacts, role: SetupRole, ci: CiSystem) -> CiGuide {
    match ci {
        CiSystem::GithubActions => CiGuide {
            token_secret: "secrets.SANSHAIN_TOKEN",
            url_variable: "vars.SANSHAIN_URL",
            origin_from: &[
                "GITHUB_SERVER_URL",
                "GITHUB_REPOSITORY",
                "GITHUB_RUN_ID",
                "GITHUB_REF",
            ],
            snippet: Some(github_steps(facts, role)),
        },
        CiSystem::GitlabCi => CiGuide {
            token_secret: "masked CI/CD variable SANSHAIN_TOKEN",
            url_variable: "CI/CD variable SANSHAIN_URL",
            origin_from: &[
                "CI_JOB_URL",
                "CI_MERGE_REQUEST_PROJECT_URL",
                "CI_MERGE_REQUEST_IID",
            ],
            snippet: None,
        },
        CiSystem::Jenkins => CiGuide {
            token_secret: "secret-text credential bound as SANSHAIN_TOKEN",
            url_variable: "environment variable SANSHAIN_URL",
            origin_from: &["BUILD_URL", "CHANGE_URL"],
            snippet: None,
        },
        CiSystem::Other => CiGuide {
            token_secret: "the CI's secret store, exposed as SANSHAIN_TOKEN",
            url_variable: "environment variable SANSHAIN_URL",
            origin_from: &[],
            snippet: None,
        },
    }
}

/// A `sanshain.yaml` to start from, with placeholder names to replace.
fn skeleton(facts: &ClientFacts, role: SetupRole) -> Result<(String, Vec<String>), AppError> {
    let mut request = DraftRequest {
        service_name: "my-service".to_string(),
        ..Default::default()
    };
    let mut replace = vec!["my-service".to_string()];
    if role.provides() {
        let file = spec_file(facts.build_tool);
        request.provides.push(DraftProvide {
            file: file.to_string(),
            api_type: None,
            retired: false,
        });
        replace.push(file.to_string());
    }
    if role.requires() {
        request.requires.push(DraftRequire {
            service_name: "other-service".to_string(),
            version: "1.0.0".to_string(),
            output_directory: format!("{}/other-service", facts.output_root),
            endpoints: vec![DraftEndpoint {
                method: "GET".to_string(),
                path: "/replace/me".to_string(),
            }],
            api_type: None,
            format_openapi: false,
        });
        replace.extend(["other-service", "1.0.0", "/replace/me"].map(str::to_string));
    }
    Ok((draft_sanshain_yaml(&request)?, replace))
}

/// The setup facts for one system: its role, build tool and, when the
/// pipeline is meant too, its CI. `instance` is the address every URL in
/// the answer is built on; unknown, the URLs stay relative.
pub fn setup_guide(
    role: SetupRole,
    build_tool: BuildTool,
    ci: Option<CiSystem>,
    instance: Option<&InstanceUrl>,
) -> Result<SetupGuide, AppError> {
    let facts = client(build_tool);
    let url = instance.map(|i| i.url.as_str());
    let (sanshain_yaml, replace_in_yaml) = skeleton(facts, role)?;
    let maven = build_tool == BuildTool::Maven;
    Ok(SetupGuide {
        role,
        build_tool,
        ci,
        instance: instance.cloned(),
        client: ClientGuide {
            name: facts.name,
            repository: facts.repository,
            version: facts.version,
            install: facts.install,
            integration: integration(facts, role),
            service: "2.x",
            min_service: MIN_SERVICE,
        },
        commands: commands(facts, role),
        token: TokenGuide {
            scope: "full",
            mint_at: format!("{}/account.html", url.unwrap_or_default()),
            minted_by: "the user, for their own builds: API Tokens, read-only unticked",
            not_the_mcp_token: true,
            local_env: (!maven).then_some("SANSHAIN_TOKEN"),
            local_file: maven.then(|| maven_settings(url.unwrap_or("<instance url>"))),
            ci_env: "SANSHAIN_TOKEN",
            ci_owner: "a service account; a user manager issues its token (For on /account.html)",
            ga_role: role.provides().then_some("releaser"),
        },
        variables: variables(role, ci, url),
        ci_setup: ci.map(|ci| ci_guide(facts, role, ci)),
        sanshain_yaml,
        replace_in_yaml,
        next_tools: [
            "list_services",
            "describe_service",
            "find_endpoint",
            "draft_sanshain_yaml",
        ],
        docs: DOCS,
    })
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

    fn corp() -> InstanceUrl {
        InstanceUrl {
            url: "https://sanshain.corp".to_string(),
            derived: false,
        }
    }

    fn purposes(guide: &SetupGuide) -> Vec<&str> {
        guide.commands.iter().map(|c| c.purpose).collect()
    }

    fn names(guide: &SetupGuide) -> Vec<&str> {
        guide.variables.iter().map(|v| v.name).collect()
    }

    #[test]
    fn a_maven_producer_on_github_gets_the_plugin_the_settings_and_a_ga_step() {
        let guide = setup_guide(
            SetupRole::Producer,
            BuildTool::Maven,
            Some(CiSystem::GithubActions),
            Some(&corp()),
        )
        .unwrap();
        let pom = guide.client.integration.as_ref().unwrap();
        assert_eq!(pom.file, "pom.xml");
        assert!(
            pom.content.contains("<version>2.5.0</version>"),
            "{}",
            pom.content
        );
        assert!(pom.content.contains("<goal>provide</goal>"));
        assert!(!pom.content.contains("<goal>require</goal>"));
        let settings = guide.token.local_file.as_ref().unwrap();
        assert_eq!(settings.file, "~/.m2/settings.xml");
        assert!(
            settings
                .content
                .contains("<sanshainUrl>https://sanshain.corp</sanshainUrl>")
        );
        assert_eq!(guide.token.local_env, None);
        assert_eq!(guide.token.mint_at, "https://sanshain.corp/account.html");
        assert_eq!(guide.token.scope, "full");
        assert_eq!(guide.token.ga_role, Some("releaser"));
        assert_eq!(
            purposes(&guide),
            [
                "provide",
                "provide_trunk",
                "provide_tag",
                "provide_ga",
                "provide_dry_run",
                "catalog",
                "fill_placeholder"
            ]
        );
        assert_eq!(
            guide.commands[4].run,
            "mvn sanshain:provide -Dsanshain.dry.run=true"
        );
        assert_eq!(
            guide.commands[6].run,
            "-DNAME=value or SANSHAIN_SET_NAME=value"
        );
        assert_eq!(
            names(&guide),
            [
                "SANSHAIN_URL",
                "SANSHAIN_TOKEN",
                "SANSHAIN_TRUNK",
                "SANSHAIN_TAG",
                "SANSHAIN_GA"
            ]
        );
        assert_eq!(
            guide.variables[0].value.as_deref(),
            Some("https://sanshain.corp")
        );
        let ci = guide.ci_setup.as_ref().unwrap();
        assert_eq!(ci.token_secret, "secrets.SANSHAIN_TOKEN");
        assert_eq!(
            ci.snippet.as_deref(),
            Some(
                "- run: mvn sanshain:provide\n  env:\n    SANSHAIN_URL: ${{ vars.SANSHAIN_URL }}\n    \
                 SANSHAIN_TOKEN: ${{ secrets.SANSHAIN_TOKEN }}\n    \
                 SANSHAIN_TRUNK: ${{ github.ref == 'refs/heads/main' }}\n    \
                 SANSHAIN_GA: ${{ github.ref == 'refs/heads/main' }}\n"
            )
        );
        let yaml: crate::yaml::Value = crate::yaml::from_str(&guide.sanshain_yaml).unwrap();
        assert_eq!(
            yaml,
            serde_json::json!({
                "serviceName": "my-service",
                "provides": [{ "file": "src/main/resources/openapi.yaml" }]
            })
        );
        assert_eq!(
            guide.replace_in_yaml,
            ["my-service", "src/main/resources/openapi.yaml"]
        );
    }

    #[test]
    fn an_npm_consumer_on_github_gets_the_action_and_the_prebuild_script() {
        let guide = setup_guide(
            SetupRole::Consumer,
            BuildTool::Npm,
            Some(CiSystem::GithubActions),
            Some(&corp()),
        )
        .unwrap();
        assert_eq!(purposes(&guide), ["require", "catalog", "fill_placeholder"]);
        assert_eq!(guide.commands[0].run, "npx sanshain require");
        let scripts = guide.client.integration.as_ref().unwrap();
        assert_eq!(scripts.file, "package.json");
        assert!(
            scripts
                .content
                .contains("\"prebuild\": \"sanshain require\"")
        );
        assert_eq!(guide.token.ga_role, None);
        assert_eq!(guide.token.local_env, Some("SANSHAIN_TOKEN"));
        assert_eq!(guide.token.local_file, None);
        assert!(!names(&guide).contains(&"SANSHAIN_GA"));
        assert_eq!(
            guide.ci_setup.and_then(|c| c.snippet).as_deref(),
            Some(
                "- uses: paxel/sanshain-js@main\n  with:\n    command: require\n    \
                 trunk: ${{ github.ref == 'refs/heads/main' }}\n  env:\n    \
                 SANSHAIN_URL: ${{ vars.SANSHAIN_URL }}\n    \
                 SANSHAIN_TOKEN: ${{ secrets.SANSHAIN_TOKEN }}\n"
            )
        );
        let yaml: crate::yaml::Value = crate::yaml::from_str(&guide.sanshain_yaml).unwrap();
        assert_eq!(
            yaml["requires"][0]["outputDirectory"],
            "generated/other-service"
        );
        assert_eq!(yaml["requires"][0]["version"], "1.0.0");
        assert_eq!(yaml.get("provides"), None);
    }

    #[test]
    fn a_local_setup_without_ci_names_only_the_address_and_the_token() {
        let guide = setup_guide(SetupRole::Both, BuildTool::Cargo, None, Some(&corp())).unwrap();
        assert_eq!(names(&guide), ["SANSHAIN_URL", "SANSHAIN_TOKEN"]);
        assert_eq!(guide.ci_setup, None);
        assert_eq!(guide.client.integration, None);
        assert_eq!(guide.client.install, "cargo install cargo-sanshain");
        assert!(
            !purposes(&guide).contains(&"provide_dry_run"),
            "cargo sends none"
        );
        assert_eq!(purposes(&guide)[..2], ["require", "provide"]);
        assert_eq!(
            guide.replace_in_yaml,
            [
                "my-service",
                "openapi.yaml",
                "other-service",
                "1.0.0",
                "/replace/me"
            ]
        );
    }

    #[test]
    fn another_ci_sets_the_origin_urls_itself() {
        let guide = setup_guide(
            SetupRole::Producer,
            BuildTool::Go,
            Some(CiSystem::Other),
            None,
        )
        .unwrap();
        let set = names(&guide);
        assert!(set.contains(&"SANSHAIN_BUILD_URL") && set.contains(&"SANSHAIN_CHANGE_URL"));
        let ci = guide.ci_setup.as_ref().unwrap();
        assert!(ci.origin_from.is_empty());
        assert_eq!(ci.snippet, None);
        assert_eq!(guide.commands[1].run, "sanshain-go --trunk provide");
        let jenkins = setup_guide(
            SetupRole::Producer,
            BuildTool::Conan,
            Some(CiSystem::Jenkins),
            None,
        )
        .unwrap();
        assert!(!names(&jenkins).contains(&"SANSHAIN_BUILD_URL"));
        assert_eq!(
            jenkins.ci_setup.map(|c| c.origin_from),
            Some(&["BUILD_URL", "CHANGE_URL"][..])
        );
    }

    #[test]
    fn an_unknown_address_leaves_the_urls_relative() {
        let guide = setup_guide(SetupRole::Both, BuildTool::Maven, None, None).unwrap();
        assert_eq!(guide.instance, None);
        assert_eq!(guide.token.mint_at, "/account.html");
        assert_eq!(guide.variables[0].value, None);
        assert!(
            guide
                .token
                .local_file
                .unwrap()
                .content
                .contains("<sanshainUrl><instance url></sanshainUrl>")
        );
        let pom = guide.client.integration.unwrap().content;
        assert!(
            pom.contains("<goal>provide</goal>\n        <goal>require</goal>"),
            "{pom}"
        );
        let conan = setup_guide(SetupRole::Consumer, BuildTool::Conan, None, None).unwrap();
        assert!(
            conan
                .client
                .integration
                .unwrap()
                .content
                .contains("        Sanshain(self).require()\n")
        );
    }
}
