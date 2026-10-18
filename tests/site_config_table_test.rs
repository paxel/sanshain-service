//! The configuration table of the site cannot drift from the code.
//!
//! `site/operator/configuration.html` lists every environment variable the
//! service reads, one row per variable. This test collects the names from the
//! table and the names from the source (`std::env::var("…")`, the `env_number`
//! and `env_choice` readers in `main.rs`, variables read through a named
//! constant, and the ones its SDKs read for it) and fails, naming the variables and the file to edit, when either
//! side has one the other lacks. Prior art: the contract checks in `src/lib.rs`.

use regex::Regex;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

const TABLE_PAGE: &str = "site/operator/configuration.html";
const TABLE_ID: &str = "env-vars";

/// Variables the service reads without a string literal in the source.
/// `RUST_LOG` is read by `tracing_subscriber::EnvFilter::try_from_default_env`
/// in `src/main.rs`, which looks the name up itself.
const READ_WITHOUT_LITERAL: &[&str] = &["RUST_LOG"];

/// Variables a dependency reads on the service's behalf, with no literal in
/// this crate: the OTLP span exporter (`opentelemetry-otlp` 0.32, built with
/// `with_tonic()` in `src/infrastructure/telemetry.rs`) reads the generic
/// `OTEL_EXPORTER_OTLP_*` settings and their `TRACES` variants itself. The
/// service exports no metrics or logs over OTLP, so those variants are not read.
const READ_BY_DEPENDENCIES: &[&str] = &[
    "OTEL_EXPORTER_OTLP_HEADERS",
    "OTEL_EXPORTER_OTLP_COMPRESSION",
    "OTEL_EXPORTER_OTLP_TIMEOUT",
    "OTEL_EXPORTER_OTLP_PROTOCOL",
    "OTEL_EXPORTER_OTLP_TRACES_ENDPOINT",
    "OTEL_EXPORTER_OTLP_TRACES_HEADERS",
    "OTEL_EXPORTER_OTLP_TRACES_COMPRESSION",
    "OTEL_EXPORTER_OTLP_TRACES_TIMEOUT",
    "OTEL_EXPORTER_OTLP_TRACES_PROTOCOL",
];

fn repo_path(relative: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(relative)
}

fn read(relative: &str) -> Result<String, String> {
    fs::read_to_string(repo_path(relative)).map_err(|e| format!("cannot read {relative}: {e}"))
}

/// The `<code>` text of the first cell of every body row of the table with `id`.
fn first_cell_codes(html: &str, id: &str) -> Result<BTreeSet<String>, String> {
    let open = format!("<table id=\"{id}\">");
    let start = html
        .find(&open)
        .ok_or_else(|| format!("{TABLE_PAGE}: no table with id=\"{id}\""))?;
    let rest = &html[start + open.len()..];
    let end = rest
        .find("</table>")
        .ok_or_else(|| format!("{TABLE_PAGE}: table id=\"{id}\" is not closed"))?;
    let table = &rest[..end];
    let row = Regex::new(r"(?s)<tr[^>]*>(.*?)</tr>").map_err(|e| e.to_string())?;
    // Anchored at the start of the row's cells: only the first <td> counts.
    let first_code =
        Regex::new(r"(?s)^\s*<td[^>]*>\s*<code>([^<]+)</code>").map_err(|e| e.to_string())?;
    let tags = Regex::new(r"<[^>]*>").map_err(|e| e.to_string())?;
    let mut names = BTreeSet::new();
    for caps in row.captures_iter(table) {
        let cells = &caps[1];
        if cells.contains("<th") {
            continue; // the header row
        }
        let code = first_code
            .captures(cells)
            .map(|c| c[1].trim().to_string())
            .ok_or_else(|| {
                let text = tags.replace_all(cells, " ");
                format!(
                    "{TABLE_PAGE}: a row of table id=\"{id}\" has no <code> in its first cell: {}",
                    text.split_whitespace().collect::<Vec<_>>().join(" ")
                )
            })?;
        names.insert(code);
    }
    Ok(names)
}

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) -> Result<(), String> {
    let entries = fs::read_dir(dir).map_err(|e| format!("cannot list {}: {e}", dir.display()))?;
    for entry in entries {
        let path = entry.map_err(|e| e.to_string())?.path();
        if path.is_dir() {
            rust_files(&path, out)?;
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            out.push(path);
        }
    }
    Ok(())
}

/// Every variable name the source reads, with the file it is read in.
fn variables_read_in_src() -> Result<BTreeMap<String, String>, String> {
    let literal = Regex::new(
        r#"(?:env::var(?:_os)?|env_number(?:::<[^>]+>)?|env_choice)\(\s*"([A-Z][A-Z0-9_]*)""#,
    )
    .map_err(|e| e.to_string())?;
    let via_const =
        Regex::new(r"env::var(?:_os)?\(\s*([A-Z][A-Z0-9_]*)\s*\)").map_err(|e| e.to_string())?;
    let const_def =
        Regex::new(r#"const ([A-Z][A-Z0-9_]*): &(?:'static )?str = "([A-Z][A-Z0-9_]*)";"#)
            .map_err(|e| e.to_string())?;

    let mut files = Vec::new();
    rust_files(&repo_path("src"), &mut files)?;
    files.sort();
    let mut sources = Vec::new();
    for path in files {
        let text = fs::read_to_string(&path)
            .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        let relative = path
            .strip_prefix(repo_path(""))
            .map_err(|e| e.to_string())?
            .display()
            .to_string();
        sources.push((relative, text));
    }

    let mut constants = BTreeMap::new();
    for (_, text) in &sources {
        for caps in const_def.captures_iter(text) {
            constants.insert(caps[1].to_string(), caps[2].to_string());
        }
    }

    let mut read_in = BTreeMap::new();
    for (relative, text) in &sources {
        for caps in literal.captures_iter(text) {
            read_in
                .entry(caps[1].to_string())
                .or_insert_with(|| relative.clone());
        }
        for caps in via_const.captures_iter(text) {
            let name = constants.get(&caps[1]).ok_or_else(|| {
                format!(
                    "{relative} reads env::var({}) but no `const {}: &str = \"…\";` was found in src",
                    &caps[1], &caps[1]
                )
            })?;
            read_in
                .entry(name.clone())
                .or_insert_with(|| relative.clone());
        }
    }
    for name in READ_WITHOUT_LITERAL {
        read_in.insert((*name).to_string(), "src/main.rs".to_string());
    }
    for name in READ_BY_DEPENDENCIES {
        read_in.insert(
            (*name).to_string(),
            "the OpenTelemetry SDK (src/infrastructure/telemetry.rs)".to_string(),
        );
    }
    Ok(read_in)
}

#[test]
fn configuration_table_lists_exactly_the_variables_the_service_or_its_sdks_read()
-> Result<(), String> {
    let documented = first_cell_codes(&read(TABLE_PAGE)?, TABLE_ID)?;
    let read_in = variables_read_in_src()?;
    let in_code: BTreeSet<String> = read_in.keys().cloned().collect();

    let undocumented: Vec<String> = in_code
        .difference(&documented)
        .map(|name| format!("{name} (read in {})", read_in[name]))
        .collect();
    let unread: Vec<String> = documented.difference(&in_code).cloned().collect();

    let mut problems = Vec::new();
    if !undocumented.is_empty() {
        problems.push(format!(
            "variables the service or its SDKs read but table id=\"{TABLE_ID}\" in {TABLE_PAGE} does not list: {}",
            undocumented.join(", ")
        ));
    }
    if !unread.is_empty() {
        problems.push(format!(
            "variables listed in table id=\"{TABLE_ID}\" of {TABLE_PAGE} that neither src nor an SDK reads: {}",
            unread.join(", ")
        ));
    }
    if problems.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "{}.\nAdd or remove the rows in {TABLE_PAGE} (or, for a variable read without a string \
             literal, the READ_WITHOUT_LITERAL or READ_BY_DEPENDENCIES list in \
             tests/site_config_table_test.rs).",
            problems.join("; ")
        ))
    }
}

#[test]
fn configuration_table_reader_sees_every_row() -> Result<(), String> {
    let names = first_cell_codes(&read(TABLE_PAGE)?, TABLE_ID)?;
    for expected in ["DATABASE_URL", "BIND_ADDRESS", "PUBLIC_BASE_URL"] {
        if !names.contains(expected) {
            return Err(format!(
                "{TABLE_PAGE}: expected row {expected} not found in table id=\"{TABLE_ID}\""
            ));
        }
    }
    Ok(())
}
