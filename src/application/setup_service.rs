//! What an AI assistant needs to set a system up with Sanshain (#88): the
//! facts about the five official clients, kept here as data. The MCP setup
//! tools answer from them; `tests/site_clients_table_test.rs` checks them
//! against the client table of `site/developer/clients.html`, so the page
//! and the answers cannot drift apart.

use serde::Serialize;

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
}
