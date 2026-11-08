//! The client table of the site cannot drift from the facts the MCP setup
//! tools answer (#88).
//!
//! `site/developer/clients.html` lists the five official clients in a table
//! (`id="clients"`), one row per build tool. The same facts live as data in
//! `src/application/setup_service.rs` (`CLIENTS`). This test reads the
//! table's rows and fails, naming the row and the file to edit, when a row and
//! its client disagree on the repository, the name, the version, the install
//! line or the provide and require commands, or when either side has a client
//! the other lacks.

use regex::Regex;
use sanshain_service::application::setup_service::{CLIENTS, GITHUB_ACTION};
use std::path::{Path, PathBuf};

const TABLE_PAGE: &str = "site/developer/clients.html";
const TABLE_ID: &str = "clients";

/// One body row of the table: the client's link and the `<code>` texts of
/// its cells, in order.
struct Row {
    link: (String, String),
    codes: Vec<String>,
}

fn rows(html: &str) -> Result<Vec<Row>, String> {
    let open = format!("<table id=\"{TABLE_ID}\">");
    let start = html
        .find(&open)
        .ok_or_else(|| format!("{TABLE_PAGE}: no table with id=\"{TABLE_ID}\""))?;
    let rest = &html[start + open.len()..];
    let end = rest
        .find("</table>")
        .ok_or_else(|| format!("{TABLE_PAGE}: table id=\"{TABLE_ID}\" is not closed"))?;
    let table = &rest[..end];
    let row = Regex::new(r"(?s)<tr[^>]*>(.*?)</tr>").map_err(|e| e.to_string())?;
    let code = Regex::new(r"(?s)<code>([^<]+)</code>").map_err(|e| e.to_string())?;
    let link = Regex::new(r#"(?s)<a href="([^"]+)">([^<]+)</a>"#).map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for caps in row.captures_iter(table) {
        let cells = &caps[1];
        if cells.contains("<th") {
            continue;
        }
        let link = link
            .captures(cells)
            .map(|c| (c[1].to_string(), c[2].trim().to_string()))
            .ok_or_else(|| format!("{TABLE_PAGE}: a client row has no repository link"))?;
        let codes = code
            .captures_iter(cells)
            .map(|c| c[1].split_whitespace().collect::<Vec<_>>().join(" "))
            .collect();
        out.push(Row { link, codes });
    }
    Ok(out)
}

#[test]
fn the_client_table_matches_the_setup_facts() -> Result<(), String> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(TABLE_PAGE);
    let html = std::fs::read_to_string(&path).map_err(|e| format!("{TABLE_PAGE}: {e}"))?;
    let rows = rows(&html)?;
    let tools: Vec<&str> = rows
        .iter()
        .filter_map(|r| r.codes.first().map(String::as_str))
        .collect();
    let expected: Vec<&str> = CLIENTS.iter().map(|c| c.build_tool.as_str()).collect();
    assert_eq!(
        tools, expected,
        "{TABLE_PAGE} lists other build tools than src/application/setup_service.rs (CLIENTS)"
    );
    for (row, client) in rows.iter().zip(CLIENTS.iter()) {
        assert_eq!(
            row.link,
            (client.repository.to_string(), client.name.to_string()),
            "{TABLE_PAGE}: the {} row's repository link differs from CLIENTS",
            client.build_tool.as_str()
        );
        let expected = vec![
            client.build_tool.as_str().to_string(),
            client.version.to_string(),
            client.install.to_string(),
        ];
        let mut codes = row.codes.clone();
        // Maven's install cell names the file the coordinates go into.
        codes.retain(|c| c != "pom.xml");
        assert_eq!(
            codes,
            [
                expected,
                vec![client.provide.to_string(), client.require.to_string()]
            ]
            .concat(),
            "{TABLE_PAGE}: the {} row differs from CLIENTS in src/application/setup_service.rs",
            client.build_tool.as_str()
        );
    }
    Ok(())
}

/// Every `.html` file under `dir`, depth first.
fn html_files(dir: &Path, out: &mut Vec<PathBuf>) -> Result<(), String> {
    let entries = std::fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    for entry in entries {
        let path = entry.map_err(|e| e.to_string())?.path();
        if path.is_dir() {
            html_files(&path, out)?;
        } else if path.extension().is_some_and(|e| e == "html") {
            out.push(path);
        }
    }
    Ok(())
}

/// A workflow step naming the JavaScript client's Action uses the ref the
/// setup answers give (#93): the repository has no `main` branch, so a
/// step written `paxel/sanshain-js@main` fails to resolve.
#[test]
fn every_page_names_the_action_the_setup_facts_give() -> Result<(), String> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("site");
    let mut files = Vec::new();
    html_files(&root, &mut files)?;
    let uses = Regex::new(r"paxel/sanshain-js@[A-Za-z0-9_/-]+(?:\.[A-Za-z0-9_/-]+)*")
        .map_err(|e| e.to_string())?;
    let mut seen = 0;
    for file in &files {
        let html = std::fs::read_to_string(file).map_err(|e| format!("{}: {e}", file.display()))?;
        for found in uses.find_iter(&html) {
            seen += 1;
            assert_eq!(
                found.as_str(),
                GITHUB_ACTION,
                "{} names the Action as {}; src/application/setup_service.rs (GITHUB_ACTION) says {GITHUB_ACTION}",
                file.display(),
                found.as_str()
            );
        }
    }
    assert!(
        seen >= 2,
        "the site should show the Action in clients.html and ci-integration.html"
    );
    Ok(())
}
